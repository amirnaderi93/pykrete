import * as fs from "fs";
import * as os from "os";
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
    // A `server/` directory inside the extension means this is the
    // per-platform .vsix and the binary should have been there — most likely
    // a packaging bug. Its absence means this is the universal .vsix served
    // to platforms without a per-target build, and the user just needs to
    // install pykrete-lsp themselves.
    const isBundledVariant = fs.existsSync(
      path.join(context.extensionPath, "server"),
    );
    const message = isBundledVariant
      ? "pykrete: the bundled pykrete-lsp binary is missing from this extension. " +
        "Try reinstalling the extension, or set `pykrete.serverPath` in settings. " +
        "Manual install: https://amirnaderi93.github.io/pykrete/getting-started/install/"
      : "pykrete: no pykrete-lsp binary is bundled for your platform. " +
        "Install it via `cargo install pykrete-lsp` or `brew install amirnaderi93/pykrete/pykrete`, " +
        "or set `pykrete.serverPath` in settings. " +
        "Manual install: https://amirnaderi93.github.io/pykrete/getting-started/install/";
    void vscode.window.showErrorMessage(message);
    return;
  }

  // vsce/vsix packaging doesn't always preserve the executable bit through
  // zip → install, so re-set it ourselves on POSIX. Best-effort: if we can't
  // (permissions, read-only FS) spawn will fail next and surface the real error.
  if (process.platform !== "win32") {
    try {
      fs.chmodSync(serverPath, 0o755);
    } catch {
      /* ignore */
    }
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

// Resolution order: user setting → bundled (per-platform .vsix) → workspace
// cargo build → PATH + Homebrew prefixes. Returns undefined when nothing is
// found; activate() surfaces a help message in that case.
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

  // 4. PATH + common install-prefix fallback. Resolve explicitly so a missing
  // binary surfaces our help message instead of a raw ENOENT later. GUI-launched
  // VS Code (especially on macOS) doesn't inherit the shell PATH, so binaries
  // installed via `cargo install` (~/.cargo/bin) or `brew` would otherwise be
  // invisible. Order matters — preserve PATH ordering and dedupe.
  const pathSep = process.platform === "win32" ? ";" : ":";
  const candidates = (process.env.PATH ?? "").split(pathSep).filter(Boolean);
  const home = os.homedir();
  if (process.platform === "darwin") {
    candidates.push(
      path.join(home, ".cargo", "bin"),
      "/opt/homebrew/bin",
      "/usr/local/bin",
    );
  } else if (process.platform === "linux") {
    candidates.push(
      path.join(home, ".cargo", "bin"),
      path.join(home, ".local", "bin"),
      "/home/linuxbrew/.linuxbrew/bin",
      "/usr/local/bin",
    );
  }
  const seen = new Set<string>();
  for (const dir of candidates) {
    if (seen.has(dir)) continue;
    seen.add(dir);
    const candidate = path.join(dir, exe);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  return undefined;
}

// Resolution order: user setting → bundled basedpyright (needs node on PATH) →
// undefined (pykrete-lsp searches PATH itself, falls back to pykrete-only).
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
