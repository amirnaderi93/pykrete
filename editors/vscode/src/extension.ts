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

  const clientOptions: LanguageClientOptions = {
    // Two selectors so dathon-lsp activates in both modes:
    //
    //   1. Default — VS Code identifies `.dpy` as the `dathon` language
    //      (declared by this extension's `contributes.languages`).
    //   2. Co-activation — the user has `"files.associations": {"*.dpy":
    //      "python"}` in settings, so VS Code identifies `.dpy` as
    //      `python` and a Python LSP (Pylance / basedpyright /
    //      pyright / ruff-lsp) handles general Python features
    //      (highlighting, std-lib completion, formatting, references)
    //      alongside dathon's dataframe-specific checks.
    documentSelector: [
      { scheme: "file", language: "dathon" },
      { scheme: "file", language: "python", pattern: "**/*.dpy" },
    ],
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
