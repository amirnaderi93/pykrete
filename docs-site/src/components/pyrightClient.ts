/**
 * Thin LSP client for the bundled pyright-browser worker.
 *
 * # Why a hand-rolled client and not `vscode-languageclient`?
 *
 * `vscode-languageclient` targets the VS Code extension host — it
 * wants a `vscode.ExtensionContext`, opens TextDocuments through
 * `vscode.workspace.openTextDocument`, and so on. None of that exists
 * in a plain browser page. The playground only needs the four LSP
 * methods the multiplexer cares about (hover / completion /
 * definition / textDocument/publishDiagnostics), so a hand-rolled
 * `MessageConnection` on top of `vscode-jsonrpc/browser` is a fraction
 * of the dependency weight.
 *
 * # Boot sequence
 *
 * `@typefox/pyright-browser` ships a single prebuilt Web Worker
 * script. Per its dist's `addEventListener('message', …)` entry, the
 * worker waits for a `browser/boot` message:
 *
 *   - `mode: 'foreground'` — pyright runs in this worker. Subsequent
 *     `postMessage` traffic is LSP JSON-RPC. This is what we use.
 *   - `mode: 'background'` — pyright will spawn *additional* copies
 *     of itself via `new Worker(self.location.toString())` to run
 *     analysis off-thread. We don't drive this directly, but the
 *     foreground server triggers it internally; that's why the
 *     worker script must live at a stable, accessible URL (see
 *     `scripts/copy-pyright-worker.mjs`).
 *
 * After boot, the worker exposes a standard LSP server over
 * `BrowserMessageReader/Writer`. We do the conventional dance:
 * `initialize` → `initialized` → `workspace/didChangeConfiguration`
 * (to set `typeCheckingMode: 'standard'`, matching what the VS Code
 * extension's embedded engine sees) → `textDocument/didOpen`. After
 * that, every edit becomes a `didChange` + a fresh diagnostics
 * notification.
 *
 * # Failure mode
 *
 * Anything that throws during init transitions the client to the
 * `failed` state. Callers should treat that as "no pyright today" and
 * fall back to pykrete-only. Crucially, request methods on a failed
 * client return `null` rather than throwing — the playground's
 * Monaco providers expect that shape.
 */

// IMPORTANT: every LSP symbol imported here comes from
// `vscode-languageserver-protocol/browser`, which re-exports both
// `vscode-jsonrpc/browser` (BrowserMessageReader/Writer +
// createMessageConnection) and the LSP request/notification type
// constants. Importing from a single entry point makes sure all the
// "_parameterStructures private property" identity checks line up —
// the moment we pull `vscode-jsonrpc` separately, TypeScript sees two
// copies of the same class declaration and rejects every
// `sendRequest(SomeRequest.type, …)` call. Don't split this import
// without re-running `npx tsc` on this file.
import {
  BrowserMessageReader,
  BrowserMessageWriter,
  createMessageConnection,
  CompletionRequest,
  DefinitionRequest,
  DidChangeConfigurationNotification,
  DidChangeTextDocumentNotification,
  DidOpenTextDocumentNotification,
  HoverRequest,
  InitializeRequest,
  InitializedNotification,
  PublishDiagnosticsNotification,
  type CompletionList,
  type CompletionItem,
  type Definition,
  type DefinitionLink,
  type Diagnostic,
  type Hover,
  type InitializeParams,
  type MessageConnection,
  type Position,
  type PublishDiagnosticsParams,
} from 'vscode-languageserver-protocol/browser';

/** Stable URI for the single document the playground edits. pyright
 * uses this as the file identity in `didOpen`/`didChange` and
 * resolves diagnostics back against it. Any `file://` URI works; this
 * particular name is purely cosmetic if it ever surfaces in an error
 * message. */
export const PLAYGROUND_URI = 'file:///playground.py';

/** Public lifecycle state. `failed` is sticky — once we hit it, we
 * stay there for the lifetime of the page. The user-facing status
 * indicator reads this. */
export type PyrightState = 'loading' | 'ready' | 'failed';

/** Caller-side handle. Methods that take a position use 0-indexed
 * (line, character) — LSP convention. Monaco speaks 1-indexed
 * everything, so the playground translates at the call site. */
export interface PyrightClient {
  state: PyrightState;
  errorMessage: string | null;
  /** Push a new full document text. Idempotent — sends `didChange`
   * only when the text actually differs from the last value. */
  setText(text: string): void;
  hover(position: Position): Promise<Hover | null>;
  completion(
    position: Position,
  ): Promise<CompletionList | CompletionItem[] | null>;
  definition(
    position: Position,
  ): Promise<Definition | DefinitionLink[] | null>;
  /** Subscribe to diagnostic-publish notifications. Returns a
   * disposable. */
  onDiagnostics(listener: (diags: Diagnostic[]) => void): () => void;
  /** Tear down the worker and any open subscriptions. Safe to call
   * multiple times; safe to call before init has completed. */
  dispose(): void;
}

/** Result of `createPyrightClient` — the client plus the underlying
 * init promise the caller can `await` to know when it's ready. The
 * playground doesn't actually await this (the client is usable
 * immediately, just queueing requests until ready), but it's a nicer
 * surface than scraping `state` in a loop. */
export interface PyrightHandle {
  client: PyrightClient;
  ready: Promise<void>;
}

/** Spawn the worker, boot pyright in foreground mode, complete the
 * LSP `initialize` handshake, configure standard-mode type checking,
 * and `didOpen` the playground document.
 *
 * `workerUrl` is the URL of the prebuilt pyright worker script. In
 * the playground that's `<base>/pyright.worker.js` (see
 * `scripts/copy-pyright-worker.mjs`). Passing it in (rather than
 * resolving it here) keeps this module decoupled from Astro's
 * `import.meta.env.BASE_URL` and easier to unit-test if we ever do.
 *
 * `initialText` is the document's contents at the moment of `didOpen`
 * — usually the active snippet. Subsequent edits flow through
 * `setText`. */
export function createPyrightClient(
  workerUrl: string,
  initialText: string,
): PyrightHandle {
  // Diagnostic listeners. We hold them externally so the
  // PublishDiagnostics handler can fan out without coupling to React
  // state directly.
  const diagListeners = new Set<(diags: Diagnostic[]) => void>();
  let lastDiagnostics: Diagnostic[] = [];

  let worker: Worker | null = null;
  let connection: MessageConnection | null = null;
  let documentText = initialText;
  let documentVersion = 1;
  let disposed = false;

  const client: PyrightClient = {
    state: 'loading',
    errorMessage: null,
    setText(text: string) {
      if (disposed || client.state !== 'ready' || !connection) return;
      if (text === documentText) return;
      documentText = text;
      documentVersion += 1;
      // didChange is fire-and-forget. If it throws (e.g. the
      // worker died between renders), swallow — the diagnostics
      // listener will simply stop receiving updates and the next
      // render's setText() will try again.
      connection
        .sendNotification(DidChangeTextDocumentNotification.type, {
          textDocument: {
            uri: PLAYGROUND_URI,
            version: documentVersion,
          },
          contentChanges: [{ text }],
        })
        .catch(() => {
          /* see comment above */
        });
    },
    async hover(position) {
      if (client.state !== 'ready' || !connection) return null;
      try {
        return await connection.sendRequest(HoverRequest.type, {
          textDocument: { uri: PLAYGROUND_URI },
          position,
        });
      } catch {
        return null;
      }
    },
    async completion(position) {
      if (client.state !== 'ready' || !connection) return null;
      try {
        return await connection.sendRequest(CompletionRequest.type, {
          textDocument: { uri: PLAYGROUND_URI },
          position,
        });
      } catch {
        return null;
      }
    },
    async definition(position) {
      if (client.state !== 'ready' || !connection) return null;
      try {
        return await connection.sendRequest(DefinitionRequest.type, {
          textDocument: { uri: PLAYGROUND_URI },
          position,
        });
      } catch {
        return null;
      }
    },
    onDiagnostics(listener) {
      diagListeners.add(listener);
      // Replay the most recent batch so a subscriber that mounted
      // after the first publish doesn't have to wait for the next
      // edit to see anything. (pyright emits an initial diagnostics
      // batch on `didOpen` — without replay, a slow React mount
      // would silently miss it.)
      if (lastDiagnostics.length > 0) listener(lastDiagnostics);
      return () => {
        diagListeners.delete(listener);
      };
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      try {
        connection?.dispose();
      } catch {
        /* connection already dead */
      }
      try {
        worker?.terminate();
      } catch {
        /* worker already dead */
      }
      diagListeners.clear();
    },
  };

  const ready = (async () => {
    try {
      worker = new Worker(workerUrl);

      // Boot pyright in this worker. Per pyright-browser's worker
      // entry point, this kicks off PyrightServer with a
      // BrowserMessage{Reader,Writer} over the worker's own
      // postMessage channel — i.e. all subsequent LSP traffic flows
      // through the same Worker instance.
      worker.postMessage({
        type: 'browser/boot',
        mode: 'foreground',
      });

      const reader = new BrowserMessageReader(worker);
      const writer = new BrowserMessageWriter(worker);
      connection = createMessageConnection(reader, writer);

      // Diagnostics handler: just cache + fan out. pyright republishes
      // the full list per document on every edit, so listeners can
      // replace state wholesale.
      connection.onNotification(
        PublishDiagnosticsNotification.type,
        (params: PublishDiagnosticsParams) => {
          if (params.uri !== PLAYGROUND_URI) return;
          lastDiagnostics = params.diagnostics;
          for (const listener of diagListeners) {
            try {
              listener(params.diagnostics);
            } catch {
              // A bad subscriber shouldn't bring down the loop.
            }
          }
        },
      );

      // Stub `workspace/configuration`. pyright queries this after
      // initialization; returning an empty array per request is
      // sufficient (settings already arrived via
      // `didChangeConfiguration`). Without a handler, the request
      // sits unresolved and pyright logs a warning.
      connection.onRequest('workspace/configuration', () => []);

      // Swallow log/show messages — they're chatty in the console
      // otherwise and we don't surface them.
      connection.onNotification('window/logMessage', () => {});
      connection.onNotification('window/showMessage', () => {});

      connection.listen();

      const initParams: InitializeParams = {
        // pyright wants a workspace folder to anchor lookups against
        // even though the playground has no real folder. A `file:///`
        // root is the conventional stub.
        processId: null,
        rootUri: 'file:///',
        capabilities: {
          textDocument: {
            hover: {
              contentFormat: ['markdown', 'plaintext'],
            },
            completion: {
              completionItem: {
                snippetSupport: true,
                documentationFormat: ['markdown', 'plaintext'],
              },
            },
            definition: {
              linkSupport: true,
            },
            publishDiagnostics: {
              versionSupport: true,
            },
          },
          workspace: {
            configuration: true,
            didChangeConfiguration: { dynamicRegistration: false },
          },
        },
        workspaceFolders: [
          { uri: 'file:///', name: 'playground' },
        ],
      };

      await connection.sendRequest(InitializeRequest.type, initParams);
      await connection.sendNotification(InitializedNotification.type, {});

      // `typeCheckingMode: 'standard'` — matches what the VS Code
      // extension's embedded engine runs at. Loud enough to catch
      // real type errors, quiet enough not to drown a new visitor in
      // red squiggles for legal-but-loose Python.
      await connection.sendNotification(
        DidChangeConfigurationNotification.type,
        {
          settings: {
            python: {
              analysis: {
                typeCheckingMode: 'standard',
                diagnosticMode: 'openFilesOnly',
                useLibraryCodeForTypes: true,
                autoSearchPaths: true,
              },
            },
          },
        },
      );

      await connection.sendNotification(
        DidOpenTextDocumentNotification.type,
        {
          textDocument: {
            uri: PLAYGROUND_URI,
            languageId: 'python',
            version: documentVersion,
            text: documentText,
          },
        },
      );

      if (disposed) {
        // Caller disposed us mid-init — honour that.
        try {
          connection.dispose();
        } catch {
          /* ignore */
        }
        try {
          worker.terminate();
        } catch {
          /* ignore */
        }
        return;
      }

      client.state = 'ready';
    } catch (err) {
      client.state = 'failed';
      client.errorMessage =
        err instanceof Error ? err.message : String(err);
      try {
        connection?.dispose();
      } catch {
        /* ignore */
      }
      try {
        worker?.terminate();
      } catch {
        /* ignore */
      }
      connection = null;
      worker = null;
      throw err;
    }
  })();

  // Detach the promise from React's "unhandled rejection" pipeline —
  // the `state === 'failed'` branch is the canonical signal callers
  // read, and the playground deliberately treats pyright failure as a
  // degraded-but-functional state rather than a fatal error.
  ready.catch(() => {
    /* see comment above */
  });

  return { client, ready };
}
