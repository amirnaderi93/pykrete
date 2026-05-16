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
      "Dathon: could not locate the dathon-lsp binary. Set 'dathon.serverPath' in settings, " +
        "or run `cargo build --release -p dathon-lsp` inside the workspace.",
    );
    return;
  }

  const serverOptions: ServerOptions = {
    run: { command: serverPath, transport: TransportKind.stdio },
    debug: { command: serverPath, transport: TransportKind.stdio },
  };

  // The Python language server dathon-lsp embeds for general Python
  // features. `undefined` lets dathon-lsp fall back to PATH discovery,
  // and failing that to dathon-only mode.
  const pythonServer = resolvePythonServer(context);

  const clientOptions: LanguageClientOptions = {
    // Two selectors so dathon-lsp activates whether VS Code identifies
    // `.dpy` as this extension's `dathon` language (the default) or as
    // `python` (when the user sets `"files.associations"`).
    documentSelector: [
      { scheme: "file", language: "dathon" },
      { scheme: "file", language: "python", pattern: "**/*.dpy" },
    ],
    // dathon-lsp multiplexes the embedded Python engine internally;
    // `pythonServer` tells it how to launch the one this extension
    // bundles. dathon's schema features work even when it's absent.
    initializationOptions: pythonServer ? { pythonServer } : {},
  };

  client = new LanguageClient(
    "dathon",
    "Dathon Language Server",
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
    .getConfiguration("dathon")
    .get<string>("serverPath", "")
    .trim();
  if (configured.length > 0) {
    return fs.existsSync(configured) ? configured : undefined;
  }

  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    const root = folder.uri.fsPath;
    const candidates = [
      path.join(root, "target", "release", "dathon-lsp"),
      path.join(root, "target", "debug", "dathon-lsp"),
    ];
    for (const candidate of candidates) {
      if (fs.existsSync(candidate)) {
        return candidate;
      }
    }
  }

  return "dathon-lsp";
}

/**
 * Resolve how dathon-lsp should launch the embedded Python engine.
 *
 *   1. `dathon.pythonServer.path` — a user-supplied langserver binary,
 *      run directly as `<path> --stdio`.
 *   2. The basedpyright bundled in this extension's `node_modules`,
 *      run as `node <langserver.index.js> --stdio` (needs Node.js on
 *      PATH).
 *   3. `undefined` — dathon-lsp searches PATH itself, and runs
 *      dathon-only if nothing is found.
 */
function resolvePythonServer(
  context: vscode.ExtensionContext,
): { command: string; args: string[] } | undefined {
  const override = vscode.workspace
    .getConfiguration("dathon")
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
