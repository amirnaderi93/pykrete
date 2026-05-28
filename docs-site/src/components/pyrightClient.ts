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

/** Optional hidden preamble configuration. The playground prepends a
 * stub module that declares `Schema`, `DataFrame`, `col`, `F`, and the
 * pykrete schema type aliases — so users can write idiomatic pykrete
 * code in the sandbox without explicit imports. The preamble is sent
 * to pyright but never shown in the editor; positions are offset at
 * the LSP boundary so the user's view sees lines 1..N as line 1..N.
 *
 * - `text` is prepended to every `didOpen`/`didChange` payload.
 * - `lines` is the number of newline characters in `text` (NOT
 *   `split('\n').length` — beware: `'foo\nbar\n'.split('\n').length`
 *   is 3, not 2, and the trailing empty element would over-offset by
 *   one). For a preamble whose final character is `\n`, this is the
 *   line number (0-indexed) where the user's line 0 lands inside the
 *   combined source.
 *
 * Diagnostics whose line falls inside the preamble (line < offset)
 * are dropped entirely — the user can't see preamble code. Hover and
 * definition results that land in the preamble are also dropped (the
 * editor has nowhere to point a "go-to-definition" at line -3). */
export interface PyrightPreamble {
  text: string;
  lines: number;
}

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
  preamble?: PyrightPreamble,
): PyrightHandle {
  // Offset applied at every position boundary with pyright. Zero when
  // no preamble was passed — the math degenerates to a no-op, so the
  // non-playground call sites stay unaffected.
  const preambleText = preamble?.text ?? '';
  const offset = preamble?.lines ?? 0;

  /** Splice `preambleText` in front of `userText` for the wire. The
   * preamble is purely for pyright's view of the document — the
   * editor never sees it. */
  const withPreamble = (userText: string): string =>
    preambleText + userText;

  /** Translate a user-space (line, character) into a pyright-space
   * position by adding the preamble offset. Character is unchanged —
   * the preamble doesn't shift columns. */
  const toPyrightPosition = (position: Position): Position => ({
    line: position.line + offset,
    character: position.character,
  });

  /** Translate a pyright-space line back to user-space. Returns `null`
   * if the result would be negative — i.e. pyright is pointing at the
   * preamble, which the user can't see. Callers should treat `null`
   * as "drop this result". */
  const fromPyrightLine = (line: number): number | null => {
    const userLine = line - offset;
    return userLine < 0 ? null : userLine;
  };

  /** Translate a pyright range to user-space. Returns `null` when
   * EITHER endpoint falls in the preamble — partial ranges that span
   * the preamble/user boundary have no meaningful editor target. */
  const rangeToUserSpace = (
    range: { start: Position; end: Position } | undefined,
  ): { start: Position; end: Position } | undefined => {
    if (!range) return range;
    const startLine = fromPyrightLine(range.start.line);
    const endLine = fromPyrightLine(range.end.line);
    if (startLine == null || endLine == null) return undefined;
    return {
      start: { line: startLine, character: range.start.character },
      end: { line: endLine, character: range.end.character },
    };
  };
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
      //
      // The preamble is spliced in transparently for pyright; the
      // public `setText` contract still talks in user-space text.
      connection
        .sendNotification(DidChangeTextDocumentNotification.type, {
          textDocument: {
            uri: PLAYGROUND_URI,
            version: documentVersion,
          },
          contentChanges: [{ text: withPreamble(text) }],
        })
        .catch(() => {
          /* see comment above */
        });
    },
    async hover(position) {
      if (client.state !== 'ready' || !connection) return null;
      try {
        const hover = await connection.sendRequest(HoverRequest.type, {
          textDocument: { uri: PLAYGROUND_URI },
          position: toPyrightPosition(position),
        });
        if (!hover) return hover;
        // pyright may attach a range to the hover; translate it back
        // to user-space. If the range falls in the preamble, drop the
        // range (Monaco then highlights the word under the cursor)
        // but keep the hover contents — the user still gets the type
        // info for whatever they hovered (which is necessarily inside
        // their own code, since `position` came from their view).
        const userRange = rangeToUserSpace(hover.range);
        return { ...hover, range: userRange };
      } catch {
        return null;
      }
    },
    async completion(position) {
      if (client.state !== 'ready' || !connection) return null;
      try {
        const result = await connection.sendRequest(CompletionRequest.type, {
          textDocument: { uri: PLAYGROUND_URI },
          position: toPyrightPosition(position),
        });
        if (!result) return result;
        // Each completion item may carry a textEdit with a range in
        // pyright-space. Walk the list and translate; items whose
        // edit range falls fully in the preamble are dropped (no
        // valid editor target). Completion items without a textEdit
        // are kept verbatim — the playground falls back to the
        // word-at-cursor range.
        const translate = (item: CompletionItem): CompletionItem | null => {
          const rawTextEdit = item.textEdit;
          if (!rawTextEdit) return item;
          const range =
            (rawTextEdit as { range?: { start: Position; end: Position } })
              .range ??
            (rawTextEdit as { insert?: { start: Position; end: Position } })
              .insert;
          if (!range) return item;
          const userRange = rangeToUserSpace(range);
          if (!userRange) return null;
          // Reshape the textEdit with the user-space range. Preserve
          // the discriminant so InsertReplaceEdit stays an
          // InsertReplaceEdit.
          const insertReplace = rawTextEdit as {
            insert?: { start: Position; end: Position };
            replace?: { start: Position; end: Position };
            newText: string;
          };
          if (insertReplace.insert && insertReplace.replace) {
            const replaceUser = rangeToUserSpace(insertReplace.replace);
            if (!replaceUser) return null;
            return {
              ...item,
              textEdit: {
                insert: userRange,
                replace: replaceUser,
                newText: insertReplace.newText,
              },
            };
          }
          return {
            ...item,
            textEdit: { range: userRange, newText: insertReplace.newText },
          };
        };
        if (Array.isArray(result)) {
          return result
            .map(translate)
            .filter((i): i is CompletionItem => i !== null);
        }
        return {
          ...result,
          items: result.items
            .map(translate)
            .filter((i): i is CompletionItem => i !== null),
        };
      } catch {
        return null;
      }
    },
    async definition(position) {
      if (client.state !== 'ready' || !connection) return null;
      try {
        const def = await connection.sendRequest(DefinitionRequest.type, {
          textDocument: { uri: PLAYGROUND_URI },
          position: toPyrightPosition(position),
        });
        if (!def) return def;
        // Walk every location/link; translate ranges. Anything fully
        // inside the preamble drops out — the user can't navigate
        // there. If everything drops, return null so the playground
        // falls back to "no pyright definition".
        const locations = Array.isArray(def) ? def : [def];
        const translated = locations
          .map((loc) => {
            const link = loc as Partial<DefinitionLink> & {
              targetUri?: string;
              targetRange?: { start: Position; end: Position };
              targetSelectionRange?: { start: Position; end: Position };
            };
            if (link.targetUri && link.targetRange) {
              const targetRange = rangeToUserSpace(link.targetRange);
              const targetSelectionRange = rangeToUserSpace(
                link.targetSelectionRange,
              );
              if (!targetRange) return null;
              return {
                ...link,
                targetRange,
                targetSelectionRange: targetSelectionRange ?? targetRange,
              };
            }
            const l = loc as { uri?: string; range?: { start: Position; end: Position } };
            if (l.uri && l.range) {
              const userRange = rangeToUserSpace(l.range);
              if (!userRange) return null;
              return { uri: l.uri, range: userRange };
            }
            return null;
          })
          .filter((x): x is NonNullable<typeof x> => x !== null);
        if (translated.length === 0) return null;
        return translated as unknown as Definition | DefinitionLink[];
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

      // Diagnostics handler: translate ranges back to user-space and
      // drop anything that falls inside the preamble (the user can't
      // see preamble code, so a squiggle pointed there is noise).
      // pyright republishes the full list per document on every edit,
      // so listeners can replace state wholesale.
      connection.onNotification(
        PublishDiagnosticsNotification.type,
        (params: PublishDiagnosticsParams) => {
          if (params.uri !== PLAYGROUND_URI) return;
          const translated: Diagnostic[] = [];
          for (const d of params.diagnostics) {
            const range = rangeToUserSpace(d.range);
            if (!range) continue; // diagnostic lives in the preamble
            translated.push({ ...d, range });
          }
          lastDiagnostics = translated;
          for (const listener of diagListeners) {
            try {
              listener(translated);
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
            // Prepend the preamble so pyright sees `Schema`,
            // `DataFrame`, `col`, `F`, and the lowercase type aliases
            // pre-declared. The user's editor view never includes the
            // preamble — all position math at the LSP boundary is
            // offset by `preamble.lines`.
            text: withPreamble(documentText),
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
