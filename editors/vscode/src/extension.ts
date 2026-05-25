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

  // 4. PATH + Homebrew-prefix fallback. Resolve explicitly so a missing
  // binary surfaces our help message instead of a raw ENOENT later. Order
  // matters — preserve PATH ordering and dedupe.
  const pathSep = process.platform === "win32" ? ";" : ":";
  const candidates = (process.env.PATH ?? "").split(pathSep).filter(Boolean);
  if (process.platform === "darwin") {
    // GUI-launched VS Code on macOS doesn't inherit the shell PATH, so a
    // pykrete-lsp installed via `brew` would otherwise be invisible here.
    candidates.push("/opt/homebrew/bin", "/usr/local/bin");
  } else if (process.platform === "linux") {
    candidates.push("/home/linuxbrew/.linuxbrew/bin", "/usr/local/bin");
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
