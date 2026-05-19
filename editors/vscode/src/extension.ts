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
  const serverPath = resolveServerPath();
  if (!serverPath) {
    vscode.window.showErrorMessage(
      "Pykrete: could not locate the pykrete-lsp binary. Set 'pykrete.serverPath' in settings, " +
        "or run `cargo build --release -p pykrete-lsp` inside the workspace.",
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

function resolveServerPath(): string | undefined {
  const configured = vscode.workspace
    .getConfiguration("pykrete")
    .get<string>("serverPath", "")
    .trim();
  if (configured.length > 0) {
    return fs.existsSync(configured) ? configured : undefined;
  }

  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    const root = folder.uri.fsPath;
    const candidates = [
      path.join(root, "target", "release", "pykrete-lsp"),
      path.join(root, "target", "debug", "pykrete-lsp"),
    ];
    for (const candidate of candidates) {
      if (fs.existsSync(candidate)) {
        return candidate;
      }
    }
  }

  return "pykrete-lsp";
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
