import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const serverPath = resolveServerPath(context);
  if (!serverPath) {
    void vscode.window.showErrorMessage(
      "pykrete: the pykrete-lsp binary wasn't found. Reinstall the extension " +
        "(it should ship with the right binary for your platform), or set " +
        "`pykrete.serverPath` in settings to an absolute path. " +
        "Manual install: https://amirnaderi93.github.io/pykrete/getting-started/install/",
    );
    return;
  }

  const serverOptions: ServerOptions = {
    run: { command: serverPath, transport: TransportKind.stdio },
    debug: { command: serverPath, transport: TransportKind.stdio },
  };

  // The Python language server pykrete-lsp embeds for general Python
  // features. `undefined` lets pykrete-lsp fall back to PATH discovery,
  // and failing that to pykrete-only mode.
  const pythonServer = resolvePythonServer(context);

  // pykrete-lsp multiplexes the embedded Python engine internally;
  // `pythonServer` tells it how to launch the one this extension
  // bundles, and `typeCheckingMode` is the single strictness knob it
  // applies to both pykrete's checks and the engine.
  const initializationOptions: Record<string, unknown> = {
    typeCheckingMode: vscode.workspace
      .getConfiguration("pykrete")
      .get<string>("typeCheckingMode", "standard"),
  };
  if (pythonServer) {
    initializationOptions.pythonServer = pythonServer;
  }

  const clientOptions: LanguageClientOptions = {
    // Two selectors so pykrete-lsp activates whether VS Code identifies
    // `.pyk` as this extension's `pykrete` language (the default) or as
    // `python` (when the user sets `"files.associations"`).
    documentSelector: [
      { scheme: "file", language: "pykrete" },
      { scheme: "file", language: "python", pattern: "**/*.pyk" },
    ],
    initializationOptions,
  };

  client = new LanguageClient(
    "pykrete",
    "Pykrete Language Server",
    serverOptions,
    clientOptions,
  );

  await client.start();
  context.subscriptions.push({
    dispose: () => {
      if (client) {
        client.stop();
      }
    },
  });
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}

/**
 * Find the pykrete-lsp binary to launch.
 *
 * Order:
 *   1. `pykrete.serverPath` setting — user override, must exist.
 *   2. Binary bundled inside the .vsix at `server/pykrete-lsp{.exe}` —
 *      the per-platform release workflow drops the matching binary
 *      here so most users have nothing to install.
 *   3. Workspace `target/release/pykrete-lsp` or `target/debug/...` —
 *      contributors running `cargo build` against this repo.
 *   4. `pykrete-lsp` on `PATH`, plus the common Homebrew prefixes
 *      (`/opt/homebrew/bin`, `/usr/local/bin`) explicitly — GUI-launched
 *      VS Code on macOS doesn't inherit the shell PATH, so the binary
 *      installed via `brew install …/pykrete/pykrete` would otherwise
 *      be invisible even though the user's terminal sees it.
 *
 * Returns `undefined` if nothing is found — the activate() error
 * message then tells the user how to fix it.
 */
function resolveServerPath(context: vscode.ExtensionContext): string | undefined {
  const exe = process.platform === "win32" ? "pykrete-lsp.exe" : "pykrete-lsp";

  // 1. User override.
  const configured = vscode.workspace
    .getConfiguration("pykrete")
    .get<string>("serverPath", "")
    .trim();
  if (configured.length > 0) {
    return fs.existsSync(configured) ? configured : undefined;
  }

  // 2. Bundled binary (shipped inside the .vsix by the release workflow).
  const bundled = path.join(context.extensionPath, "server", exe);
  if (fs.existsSync(bundled)) {
    return bundled;
  }

  // 3. Workspace builds — contributors.
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    const root = folder.uri.fsPath;
    for (const sub of ["release", "debug"]) {
      const candidate = path.join(root, "target", sub, exe);
      if (fs.existsSync(candidate)) {
        return candidate;
      }
    }
  }

  // 4. PATH + Homebrew-prefix fallback. We resolve explicitly rather
  // than rely on `spawn`-with-PATH so a missing binary surfaces our
  // helpful error instead of a raw ENOENT later.
  const pathSep = process.platform === "win32" ? ";" : ":";
  const pathDirs = new Set(
    (process.env.PATH ?? "").split(pathSep).filter(Boolean),
  );
  if (process.platform === "darwin") {
    pathDirs.add("/opt/homebrew/bin");
    pathDirs.add("/usr/local/bin");
  } else if (process.platform === "linux") {
    pathDirs.add("/home/linuxbrew/.linuxbrew/bin");
    pathDirs.add("/usr/local/bin");
  }
  for (const dir of pathDirs) {
    const candidate = path.join(dir, exe);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  return undefined;
}

/**
 * Resolve how pykrete-lsp should launch the embedded Python engine.
 *
 *   1. `pykrete.pythonServer.path` — a user-supplied langserver binary,
 *      run directly as `<path> --stdio`.
 *   2. The basedpyright bundled in this extension's `node_modules`,
 *      run as `node <langserver.index.js> --stdio` (needs Node.js on
 *      PATH).
 *   3. `undefined` — pykrete-lsp searches PATH itself, and runs
 *      pykrete-only if nothing is found.
 */
function resolvePythonServer(
  context: vscode.ExtensionContext,
): { command: string; args: string[] } | undefined {
  const override = vscode.workspace
    .getConfiguration("pykrete")
    .get<string>("pythonServer.path", "")
    .trim();
  if (override.length > 0) {
    return { command: override, args: ["--stdio"] };
  }

  const bundled = path.join(
    context.extensionPath,
    "node_modules",
    "basedpyright",
    "langserver.index.js",
  );
  if (fs.existsSync(bundled)) {
    return { command: "node", args: [bundled, "--stdio"] };
  }

  return undefined;
}
