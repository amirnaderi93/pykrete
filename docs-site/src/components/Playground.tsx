/**
 * Interactive `.pyk` playground.
 *
 * Renders a Monaco editor on the left, a diagnostics panel on the
 * right (or below, on narrow screens), and an example-snippet
 * dropdown above. Every edit (debounced ~300 ms) runs the source
 * through the `pykrete-wasm` analyzer and updates both the
 * in-editor squiggle markers and the diagnostics list.
 *
 * ## Why React + `@monaco-editor/react`
 *
 * Monaco is the editor that powers VS Code; the React wrapper takes
 * care of the loader machinery (Monaco insists on a global `MonacoEnvironment`
 * that's awkward in a no-build SSR context) and ships Monaco's
 * 2-MB-ish bundle from the unpkg CDN instead of bundling it locally.
 * For an Astro Starlight site that's otherwise pure static markdown,
 * loading Monaco from a CDN keeps the playground's weight off every
 * other page.
 *
 * ## Why `client:only`
 *
 * Monaco needs `window` and `document` at import time — neither
 * exists during Astro's static-render pass. `client:only="react"`
 * on the consuming `<Playground />` instance tells Astro to skip SSR
 * entirely and render the component on the client.
 *
 * ## wasm initialization
 *
 * `pykrete-wasm` ships an async `init()` plus the synchronous
 * `check_source`. We call `init()` once on mount, then re-run
 * `check_source` on each edit. While init is pending the editor
 * still works — diagnostics just lag a moment until the wasm is
 * ready.
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import Editor, { type Monaco, type OnChange, type OnMount } from '@monaco-editor/react';
import type { editor, languages } from 'monaco-editor';
import init, {
  check_source,
  hover_at,
  complete_at,
  definition_at,
} from 'pykrete-wasm';
import type {
  CompletionItem as LspCompletionItem,
  CompletionList as LspCompletionList,
  Definition as LspDefinition,
  DefinitionLink as LspDefinitionLink,
  Diagnostic as LspDiagnostic,
  Hover as LspHover,
  MarkupContent as LspMarkupContent,
  Position as LspPosition,
  Range as LspRange,
} from 'vscode-languageserver-protocol';
import {
  createPyrightClient,
  type PyrightClient,
  type PyrightState,
} from './pyrightClient';
import './Playground.css';

/** Diagnostic shape coming back from `check_source`. Must stay in sync
 * with `DiagnosticOut` in `crates/pykrete-wasm/src/lib.rs`. */
interface Diagnostic {
  code: string;
  rule_name: string;
  severity: 'error' | 'warning' | string;
  message: string;
  line: number;
  column: number;
  end_line: number;
  end_column: number;
}

/** Hover payload from `hover_at`. Mirrors `HoverOut` in
 * `crates/pykrete-wasm/src/lib.rs`. */
interface HoverPayload {
  contents: string;
  line: number;
  column: number;
  end_line: number;
  end_column: number;
}

/** Completion item from `complete_at`. Mirrors `CompletionOut`. */
interface CompletionPayload {
  label: string;
  detail: string | null;
  insert_text: string;
  /** `"field"` or `"schema"` today; stringly-typed so the wasm boundary
   * doesn't leak Monaco's enum back across it. */
  kind: string;
}

/** Definition location from `definition_at`. Mirrors `LocationOut`. */
interface LocationPayload {
  line: number;
  column: number;
  end_line: number;
  end_column: number;
}

/** One pre-loaded snippet shown in the dropdown above the editor. */
interface Snippet {
  /** Stable key, used as the option `value`. */
  id: string;
  /** Label shown in the dropdown. */
  label: string;
  /** One-line description shown under the dropdown. */
  description: string;
  /** The `.pyk` source. */
  source: string;
}

/** Short blank-slate snippet — the "empty" option in the dropdown.
 * Just enough scaffolding (a Schema + a function stub) so a user can
 * start typing and immediately see diagnostics flow. */
const EMPTY_SOURCE = `# Define a schema, then a function over a DataFrame[Schema].
# pykrete checks column names, types, and the shape of return values.

class Users(Schema):
    id: int
    name: string
    email: string

def keep(df: DataFrame[Users]) -> DataFrame[Users]:
    return df.select(col("id"), col("name"), col("email"))
`;

const SNIPPETS: Snippet[] = [
  {
    id: 'column-typo',
    label: 'Column typo (D0030)',
    description:
      'A misspelled column name fires `unknownColumn` with a did-you-mean suggestion.',
    source: `class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int

def revenue_by_region(sales: DataFrame[Sale]):
    # "regoin" is a typo — pykrete suggests "region".
    return sales.groupBy("regoin").agg(F.sum("amount"))
`,
  },
  {
    id: 'schema-flow',
    label: 'Schema flow through a chain',
    description:
      'Hover Sale, sales, or summary. Type col("…") inside the chain for column completions.',
    source: `class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int

def report(sales: DataFrame[Sale]):
    # Hover \`Sale\` above to see its fields.
    # Hover \`sales\` here to see the bound schema.
    summary = (
        sales
        .withColumn("revenue", F.col("amount") * F.col("quantity"))
        .drop("amount", "quantity")
        .groupBy("region")
        .agg(F.sum("revenue").alias("total"))
    )
    # Hover \`summary\` — pykrete tracked the schema through the chain.
    return summary
`,
  },
  {
    id: 'empty',
    label: 'Empty — start from scratch',
    description: 'A blank-ish slate. Edit freely; diagnostics update as you type.',
    source: EMPTY_SOURCE,
  },
];

/** Map a pykrete severity string to a Monaco marker severity. The
 * `monaco` namespace is only available after the editor mounts, so
 * we accept it as a parameter rather than importing it at module
 * scope (which would defeat the lazy-load). */
function markerSeverity(monaco: Monaco, severity: string): number {
  if (severity === 'warning') return monaco.MarkerSeverity.Warning;
  // Treat everything else (including unknown) as Error — louder is
  // safer than silent for a diagnostic the user actually wants to
  // see.
  return monaco.MarkerSeverity.Error;
}

/** Map pykrete-wasm's stringly-typed completion kind to Monaco's
 * `CompletionItemKind` enum. New kinds added on the Rust side fall
 * back to `Text` so the icon is at worst neutral, never wrong. */
function completionKindFor(
  monaco: Monaco,
  kind: string,
): languages.CompletionItemKind {
  switch (kind) {
    case 'field':
      return monaco.languages.CompletionItemKind.Field;
    case 'schema':
      return monaco.languages.CompletionItemKind.Class;
    default:
      return monaco.languages.CompletionItemKind.Text;
  }
}

/**
 * Race a promise against a deadline. Returns the promise's resolved
 * value, or `fallback` if it doesn't settle inside `timeoutMs`. Used
 * for every pyright call — the multiplexer in `pykrete-lsp` enforces
 * the same 2s ceiling on its embedded engine, and a stuck pyright
 * worker should never hang the playground's hover popup or completion
 * menu.
 *
 * Errors thrown by the wrapped promise are also collapsed to
 * `fallback`. Per the multiplex contract ("never crash the
 * playground"), the caller treats every non-success the same way:
 * fall back to pykrete-only.
 */
async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  fallback: T,
): Promise<T> {
  return new Promise<T>((resolve) => {
    const handle = setTimeout(() => resolve(fallback), timeoutMs);
    promise
      .then((value) => {
        clearTimeout(handle);
        resolve(value);
      })
      .catch(() => {
        clearTimeout(handle);
        resolve(fallback);
      });
  });
}

/** Pyright/LSP timeout budget for hover/completion/definition. Matches
 * the multiplexer's 2 s ceiling in `crates/pykrete-lsp/src/multiplex.rs`
 * — see the comment there about why basedpyright/pyright can silently
 * never answer. */
const PYRIGHT_REQUEST_TIMEOUT_MS = 2000;

/** Convert Monaco's 1-indexed (lineNumber, column) to LSP's 0-indexed
 * (line, character). Monaco's `column` is also 1-indexed (the cell to
 * the LEFT of the column number is column 0 in LSP terms). */
function toLspPosition(lineNumber: number, column: number): LspPosition {
  return { line: lineNumber - 1, character: column - 1 };
}

/** Convert an LSP range to Monaco's `IRange` shape (1-indexed
 * everything). LSP `end` is exclusive in the same sense Monaco's
 * `endColumn` is exclusive, so a straight +1 on both ends works. */
function toMonacoRange(monaco: Monaco, range: LspRange): import('monaco-editor').IRange {
  return {
    startLineNumber: range.start.line + 1,
    startColumn: range.start.character + 1,
    endLineNumber: range.end.line + 1,
    endColumn: range.end.character + 1,
  } as unknown as import('monaco-editor').IRange;
  // The cast keeps this helper independent of the editor's runtime
  // Monaco namespace — we only need the shape. (The `monaco` param is
  // still there to give a stable signature if we ever construct a real
  // `monaco.Range` instance.)
}

/** Flatten LSP `Hover.contents` (string | MarkedString | MarkupContent
 * | array-of-those) into a single markdown string. Mirrors what
 * `hover_contents_to_string` in `crates/pykrete-lsp/src/multiplex.rs`
 * does for the LSP fan-out so the playground stays semantically aligned
 * with the editor multiplexer. */
function hoverContentsToString(contents: LspHover['contents']): string {
  if (contents == null) return '';
  if (typeof contents === 'string') return contents;
  if (Array.isArray(contents)) {
    return contents
      .map((item) => {
        if (typeof item === 'string') return item;
        // MarkedString { language, value } → fenced code block; the
        // server may also return a bare `{ value }` MarkupContent here.
        const language = (item as { language?: string }).language;
        const value = (item as { value?: string }).value ?? '';
        return language ? '```' + language + '\n' + value + '\n```' : value;
      })
      .filter((s) => s.length > 0)
      .join('\n\n');
  }
  // MarkupContent — { kind: 'markdown' | 'plaintext', value }
  return (contents as LspMarkupContent).value ?? '';
}

/** Is this a pykrete schema hover? Mirrors `is_schema_hover` in
 * `crates/pykrete-lsp/src/multiplex.rs`: if the rendered markdown
 * starts with `**schema** `, pykrete already owns the popup and
 * pyright's "(class) Foo" echo is noise. The string comes from
 * `render_schema_hover` in `crates/pykrete/src/hover.rs`; if that ever
 * changes prefix, revisit both this check and the Rust counterpart. */
function isPykreteSchemaHover(markdown: string): boolean {
  return markdown.startsWith('**schema** ');
}

/** Map an LSP `DiagnosticSeverity` integer to the playground's
 * stringly-typed shape so the existing renderer doesn't need to
 * branch. Anything other than Warning/Information/Hint becomes
 * `error` — matches the conservative-louder choice in
 * `markerSeverity`. */
function lspSeverityToString(
  severity: number | undefined,
): 'error' | 'warning' {
  // LSP: 1=Error, 2=Warning, 3=Information, 4=Hint
  if (severity === 2 || severity === 3 || severity === 4) return 'warning';
  return 'error';
}

/** Convert an LSP diagnostic to the in-playground `Diagnostic` shape
 * so pyright and pykrete share one render path. The code prefix
 * (`py:`) distinguishes pyright's diagnostics in the panel from
 * pykrete's `D\d\d\d\d` codes at a glance. */
function lspDiagnosticToPlayground(d: LspDiagnostic): Diagnostic {
  const code =
    typeof d.code === 'string' || typeof d.code === 'number'
      ? `py:${d.code}`
      : 'py:type';
  // LSP `source` is conventionally "pyright" here; fall back to the
  // same so the panel's title attributes look sensible.
  return {
    code,
    rule_name: d.source ?? 'pyright',
    severity: lspSeverityToString(d.severity),
    message: d.message,
    line: d.range.start.line + 1,
    column: d.range.start.character + 1,
    end_line: d.range.end.line + 1,
    end_column: d.range.end.character + 1,
  };
}

/** Pull a uniform `CompletionItem[]` out of pyright's
 * `CompletionList | CompletionItem[] | null` so the merge step
 * doesn't have to branch on result shape. */
function lspCompletionItems(
  result: LspCompletionList | LspCompletionItem[] | null,
): LspCompletionItem[] {
  if (!result) return [];
  return Array.isArray(result) ? result : result.items;
}

/**
 * Hook: returns a value that only updates `delay` ms after the
 * source stops changing. Keeps us from re-running the analyzer on
 * every keystroke.
 */
function useDebounced<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const handle = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(handle);
  }, [value, delay]);
  return debounced;
}

export default function Playground() {
  // The active snippet drives the initial editor contents. After the
  // user starts editing we don't push snippet changes back into
  // `source` automatically — only when they explicitly select a new
  // snippet from the dropdown.
  const [snippetId, setSnippetId] = useState<string>(SNIPPETS[0].id);
  const activeSnippet =
    SNIPPETS.find((s) => s.id === snippetId) ?? SNIPPETS[0];
  const [source, setSource] = useState<string>(activeSnippet.source);
  const debouncedSource = useDebounced(source, 300);

  const [wasmReady, setWasmReady] = useState(false);
  /** True after Monaco's `onMount` callback fires. Tracked as state (not
   * just a ref) so the provider-registration effect can re-run when the
   * editor finishes loading — see the race-condition comment on that
   * effect below. */
  const [editorReady, setEditorReady] = useState(false);
  const [wasmError, setWasmError] = useState<string | null>(null);
  /** Pykrete-side diagnostics. Pyright's are kept in a separate state
   * (`pyrightDiagnostics`) and merged at render time — see the
   * `mergedDiagnostics` memo below. Splitting the two means we don't
   * have to rebuild pykrete's marker list just because pyright sent a
   * new batch (and vice versa). */
  const [diagnostics, setDiagnostics] = useState<Diagnostic[]>([]);
  const [pyrightDiagnostics, setPyrightDiagnostics] = useState<Diagnostic[]>([]);
  /** Pyright lifecycle, surfaced in the status line and used to gate
   * the multiplex paths. `loading` while the worker boots, `ready`
   * after `initialize` + `didOpen` complete, `failed` if anything in
   * that chain throws. */
  const [pyrightState, setPyrightState] = useState<PyrightState>('loading');
  const [pyrightError, setPyrightError] = useState<string | null>(null);

  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<Monaco | null>(null);
  /** Pyright client handle. Held in a ref (not state) because the
   * language providers read from it lexically and re-rendering when
   * the client *value* identity changes would be churn for no gain —
   * lifecycle is communicated via `pyrightState`. */
  const pyrightRef = useRef<PyrightClient | null>(null);
  /** Mirror of `pyrightState` accessible from the language providers
   * without re-registering them. JSX closures capture the state value
   * at registration time, but a ref always sees the latest write. */
  const pyrightStateRef = useRef<PyrightState>('loading');
  /** Language-provider disposables. Monaco registers providers
   * globally (per language ID, not per editor), so we keep handles to
   * dispose them on unmount — otherwise React fast-refresh + repeated
   * navigations would stack duplicate providers and Monaco would call
   * each one once per hover/completion request. */
  const providerDisposablesRef = useRef<{ dispose(): void }[]>([]);
  /** Latest editor source, kept in a ref so the language providers
   * (registered once, capture lexically) can read it without
   * re-registering on every keystroke. Declared early because the
   * pyright init effect reads it for the initial `didOpen` text. */
  const liveSourceRef = useRef<string>(activeSnippet.source);

  // wasm init runs once. The pykrete-wasm package's default export is
  // a function that fetches and instantiates the `.wasm` binary; it
  // resolves once `check_source` is callable.
  useEffect(() => {
    let cancelled = false;
    init()
      .then(() => {
        if (!cancelled) setWasmReady(true);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          // Surface a friendly error in the diagnostics panel rather
          // than letting the user wonder why nothing happens.
          setWasmError(
            err instanceof Error ? err.message : String(err),
          );
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // pyright-browser init also runs once, in parallel with the wasm
  // boot. The worker file is copied to `public/pyright.worker.js` at
  // build/dev time (see scripts/copy-pyright-worker.mjs) and served at
  // `<base>/pyright.worker.js`. We prefix with Astro's base URL so the
  // playground works whether the site is deployed at the root or under
  // a subpath (e.g. /pykrete).
  //
  // The whole pyright surface is best-effort: if init fails (worker
  // 404'd, OOM, browser without Worker support), the playground keeps
  // running on pykrete alone. The status-line indicator surfaces what
  // happened.
  useEffect(() => {
    // import.meta.env.BASE_URL is Vite/Astro's runtime base path
    // (ends with a trailing slash). Concatenating the worker filename
    // gives the absolute URL Astro serves the static asset from.
    const baseUrl =
      (import.meta as unknown as { env?: { BASE_URL?: string } }).env
        ?.BASE_URL ?? '/';
    const workerUrl =
      (baseUrl.endsWith('/') ? baseUrl : baseUrl + '/') +
      'pyright.worker.js';

    let cancelled = false;
    let handle: ReturnType<typeof createPyrightClient> | null = null;

    try {
      handle = createPyrightClient(workerUrl, liveSourceRef.current);
      pyrightRef.current = handle.client;
      // Subscribe to diagnostics — pyright will publish an initial
      // batch right after didOpen and then a fresh batch per
      // didChange. The client replays the last batch on subscribe so
      // we don't miss the initial one even if React mounts late.
      const unsubscribe = handle.client.onDiagnostics((diags) => {
        if (cancelled) return;
        setPyrightDiagnostics(diags.map(lspDiagnosticToPlayground));
      });
      handle.ready
        .then(() => {
          if (cancelled) return;
          setPyrightState('ready');
        })
        .catch((err: unknown) => {
          if (cancelled) return;
          setPyrightError(
            err instanceof Error ? err.message : String(err),
          );
          setPyrightState('failed');
        });
      return () => {
        cancelled = true;
        unsubscribe();
        handle?.client.dispose();
        pyrightRef.current = null;
      };
    } catch (err) {
      // `createPyrightClient` itself can throw synchronously if the
      // environment doesn't support Workers at all.
      setPyrightError(err instanceof Error ? err.message : String(err));
      setPyrightState('failed');
      return () => {
        cancelled = true;
      };
    }
    // We intentionally depend on nothing — pyright is initialised
    // exactly once per playground mount. The init runs in parallel
    // with Monaco and the wasm boot; the multiplex paths each gate on
    // `pyrightState === 'ready'` and degrade gracefully until then.
  }, []);

  // Forward every debounced source change to pyright so its
  // diagnostics stay in sync with the editor. setText is a no-op
  // until pyright is ready, so it's safe to fire before init
  // completes; the `didOpen` text was already seeded with the initial
  // source.
  useEffect(() => {
    pyrightRef.current?.setText(debouncedSource);
  }, [debouncedSource, pyrightState]);

  // Keep `pyrightStateRef` in sync so the language providers (closures
  // captured at register-time) always read the latest state when
  // Monaco invokes them.
  useEffect(() => {
    pyrightStateRef.current = pyrightState;
  }, [pyrightState]);

  // Keep the live-source ref in sync with the source state so the
  // language providers (registered once, see the provider-registration
  // useEffect below) always see the freshest text without
  // re-registering on every keystroke.
  useEffect(() => {
    liveSourceRef.current = source;
  }, [source]);

  // Re-run the analyzer whenever the debounced source or the wasm-ready
  // flag changes. Surfaces the diagnostics into local state — Monaco
  // markers (pykrete's owner string) are managed by a separate effect
  // below so pyright's diagnostics can ride along on their own marker
  // owner.
  useEffect(() => {
    if (!wasmReady) return;
    let next: Diagnostic[];
    try {
      next = (check_source(debouncedSource) ?? []) as Diagnostic[];
    } catch (err) {
      // `check_source` itself can't throw on the Rust side (the
      // wrapper catches panics), but the wasm-bindgen glue can throw
      // if the module is half-instantiated.
      next = [
        {
          code: 'D9999',
          rule_name: 'internalError',
          severity: 'error',
          message: `Playground error: ${err instanceof Error ? err.message : String(err)}`,
          line: 1,
          column: 1,
          end_line: 1,
          end_column: 1,
        },
      ];
    }
    setDiagnostics(next);
  }, [debouncedSource, wasmReady]);

  /** Merged diagnostics for both the panel and the marker effect.
   * Pykrete first (matches the multiplexer's render order in the LSP),
   * pyright second. Recomputed only when either source's list changes —
   * not on every keystroke — because both `diagnostics` and
   * `pyrightDiagnostics` are only updated by their respective debounced
   * effects/notifications. */
  const mergedDiagnostics = useMemo(
    () => [...diagnostics, ...pyrightDiagnostics],
    [diagnostics, pyrightDiagnostics],
  );

  // Push pykrete's markers into Monaco under owner `pykrete`. Separate
  // owner string from pyright's so each can clear/update independently
  // without trampling the other's markers (Monaco's `setModelMarkers`
  // replaces the full set for a given owner).
  useEffect(() => {
    const editor = editorRef.current;
    const monaco = monacoRef.current;
    if (!editor || !monaco) return;
    const model = editor.getModel();
    if (!model) return;
    const markers = diagnostics.map((d) => ({
      startLineNumber: d.line,
      startColumn: d.column,
      endLineNumber: d.end_line,
      endColumn: d.end_column,
      message: `${d.rule_name}: ${d.message}`,
      severity: markerSeverity(monaco, d.severity),
      code: d.code,
      source: 'pykrete',
    }));
    monaco.editor.setModelMarkers(model, 'pykrete', markers);
  }, [diagnostics, editorReady]);

  // And pyright's markers under owner `pyright`. Same shape, separate
  // bucket. Fires on every PublishDiagnostics notification (which
  // pyright sends after each `didChange`); pykrete's markers above are
  // untouched.
  useEffect(() => {
    const editor = editorRef.current;
    const monaco = monacoRef.current;
    if (!editor || !monaco) return;
    const model = editor.getModel();
    if (!model) return;
    const markers = pyrightDiagnostics.map((d) => ({
      startLineNumber: d.line,
      startColumn: d.column,
      endLineNumber: d.end_line,
      endColumn: d.end_column,
      message: `${d.rule_name}: ${d.message}`,
      severity: markerSeverity(monaco, d.severity),
      code: d.code,
      source: 'pyright',
    }));
    monaco.editor.setModelMarkers(model, 'pyright', markers);
  }, [pyrightDiagnostics, editorReady]);

  // Register pykrete's hover / completion / definition providers on
  // the `python` language. Monaco's provider API is global per
  // language ID — registering once and reading the source from a ref
  // (`liveSourceRef`) lets the providers stay responsive without
  // re-registering on every keystroke. The wasm-ready flag gates
  // registration so we never call the wasm exports before they exist.
  //
  // RACE-CONDITION NOTE: `monacoRef.current` is populated by
  // `handleMount` (Monaco's `onMount` callback), which does NOT
  // trigger a React re-render. If this effect depended only on
  // `wasmReady` it would run once when wasm becomes ready — and if
  // Monaco hadn't mounted yet (the realistic case: wasm is tiny/cached
  // while Monaco is a ~2MB CDN fetch), `monacoRef.current` would still
  // be null, the effect would early-return, and would NEVER re-run —
  // leaving the user without hover / completion / go-to-definition
  // until a page refresh. Tracking `editorReady` as React STATE (set
  // in `handleMount`) and listing it as a dep makes this effect re-run
  // the moment Monaco finishes loading, regardless of which side wins.
  useEffect(() => {
    if (!wasmReady || !editorReady) return;
    const monaco = monacoRef.current;
    if (!monaco) return;
    // Dispose any previously-registered providers first — React strict
    // mode runs effects twice, and a hot-reload could leave stale
    // providers behind. The cleanup function handles the unmount case.
    for (const d of providerDisposablesRef.current) d.dispose();
    providerDisposablesRef.current = [];

    // Hover provider — multiplexed. Asks pykrete and pyright in
    // parallel; merges per the rules in
    // `crates/pykrete-lsp/src/multiplex.rs::merge_hover`:
    //   - If pykrete returns a schema hover (`**schema** …`), suppress
    //     pyright's contribution (its "(class) Foo" echo crowds the
    //     popup bottom — same reasoning as the v0.1.10 LSP fix).
    //   - Otherwise stack pykrete first, then a `---`, then pyright.
    //   - If only one side answers, return that side alone.
    //   - If neither, return null (Monaco hides the popup).
    const hover = monaco.languages.registerHoverProvider('python', {
      async provideHover(_model, position) {
        // pykrete is synchronous (wasm); pyright is a Worker round-trip
        // capped at PYRIGHT_REQUEST_TIMEOUT_MS. We launch both then
        // merge, so a slow pyright never delays pykrete's hover.
        const pykreteOut = (() => {
          try {
            return hover_at(
              liveSourceRef.current,
              position.lineNumber,
              position.column,
            ) as HoverPayload | null;
          } catch {
            return null;
          }
        })();

        const pyrightOut =
          pyrightRef.current && pyrightStateRef.current === 'ready'
            ? await withTimeout(
                pyrightRef.current.hover(
                  toLspPosition(position.lineNumber, position.column),
                ),
                PYRIGHT_REQUEST_TIMEOUT_MS,
                null as LspHover | null,
              )
            : null;

        const pykreteText = pykreteOut?.contents ?? '';
        const pyrightText = pyrightOut
          ? hoverContentsToString(pyrightOut.contents)
          : '';

        // Suppress pyright if pykrete owns a schema hover. Same rule
        // the LSP multiplexer enforces — the schema heading already
        // covers everything pyright would echo.
        const suppressPyright =
          pykreteOut && isPykreteSchemaHover(pykreteText);

        if (pykreteOut && (suppressPyright || pyrightText.length === 0)) {
          return {
            range: new monaco.Range(
              pykreteOut.line,
              pykreteOut.column,
              pykreteOut.end_line,
              pykreteOut.end_column,
            ),
            contents: [{ value: pykreteText }],
          };
        }
        if (!pykreteOut && pyrightOut && pyrightText.length > 0) {
          return {
            range: pyrightOut.range
              ? toMonacoRange(monaco, pyrightOut.range)
              : undefined,
            contents: [{ value: pyrightText }],
          };
        }
        if (pykreteOut && pyrightOut && pyrightText.length > 0) {
          // Both sides have content: stacked render with a horizontal
          // rule between them. pykrete first (more specific) — same
          // ordering as the LSP multiplexer.
          const range = new monaco.Range(
            pykreteOut.line,
            pykreteOut.column,
            pykreteOut.end_line,
            pykreteOut.end_column,
          );
          return {
            range,
            contents: [
              { value: pykreteText + '\n\n---\n\n' + pyrightText },
            ],
          };
        }
        return null;
      },
    });

    const completion = monaco.languages.registerCompletionItemProvider('python', {
      // Auto-open completion when the user types one of the surfaces
      // where a pykrete completion is meaningful:
      //   - `"` and `'` — string column args, e.g. `col("…")`
      //   - `(`        — opening any function call
      //   - `[`        — `DataFrame[…]` schema slot
      //   - `.`        — chain methods on a DataFrame
      // Identifier chars (letters, `_`) are deliberately NOT trigger
      // characters: Monaco already pops the menu mid-identifier, and
      // listing them here would re-fire the provider on every keystroke
      // inside any name (e.g. `df_filtered`), producing noisy popups.
      // Ctrl+Space (manual trigger) always invokes the provider
      // regardless of trigger characters.
      triggerCharacters: ['"', "'", '(', '[', '.'],
      async provideCompletionItems(model, position) {
        // pykrete first (more specific — column names, schema names,
        // DataFrame member methods). Then pyright (general Python —
        // builtins, imported names, std-lib argument names). Order
        // matches `crates/pykrete-lsp/src/multiplex.rs::merge_completion`.
        const pykreteItems = (() => {
          try {
            return (complete_at(
              liveSourceRef.current,
              position.lineNumber,
              position.column,
            ) ?? []) as CompletionPayload[];
          } catch {
            return [] as CompletionPayload[];
          }
        })();

        const pyrightResult =
          pyrightRef.current && pyrightStateRef.current === 'ready'
            ? await withTimeout(
                pyrightRef.current.completion(
                  toLspPosition(position.lineNumber, position.column),
                ),
                PYRIGHT_REQUEST_TIMEOUT_MS,
                null,
              )
            : null;

        // Pykrete suggestions replace the "word at cursor" — Monaco
        // inserts the chosen item in place of any partial identifier
        // the user has typed so far. Inside a string literal
        // `col("|")` the word is empty and the suggestion is inserted
        // at the cursor, which is the right behavior.
        const word = model.getWordUntilPosition(position);
        const fallbackRange = new monaco.Range(
          position.lineNumber,
          word.startColumn,
          position.lineNumber,
          word.endColumn,
        );

        const pykreteSuggestions: languages.CompletionItem[] =
          pykreteItems.map((item) => ({
            label: item.label,
            detail: item.detail ?? undefined,
            insertText: item.insert_text,
            kind: completionKindFor(monaco, item.kind),
            range: fallbackRange,
          }));

        const pyrightSuggestions: languages.CompletionItem[] =
          lspCompletionItems(pyrightResult).map((item) => {
            // pyright items often carry their own textEdit / range; if
            // present, prefer it (it knows about the document model)
            // so insertion replaces the right span. Fall back to the
            // word-at-cursor range when missing.
            const rawTextEdit = (item as { textEdit?: unknown }).textEdit;
            const lspRange =
              rawTextEdit && typeof rawTextEdit === 'object'
                ? ((rawTextEdit as { range?: LspRange }).range ??
                    (rawTextEdit as { insert?: LspRange }).insert)
                : undefined;
            const range = lspRange
              ? toMonacoRange(monaco, lspRange)
              : fallbackRange;
            const insertText = item.insertText ?? item.label;
            const detail =
              item.detail ??
              (typeof item.labelDetails?.description === 'string'
                ? item.labelDetails.description
                : undefined);
            return {
              // LSP `CompletionItem.label` is always a string —
              // `CompletionItemLabelDetails` lives on the separate
              // `labelDetails` field.
              label: item.label,
              detail,
              insertText,
              kind:
                // LSP CompletionItemKind enum overlaps with Monaco's
                // numerically (both follow LSP's spec values). Default
                // unknown to Text so the icon is at worst neutral.
                (item.kind ?? monaco.languages.CompletionItemKind.Text) as
                  unknown as languages.CompletionItemKind,
              range,
              filterText: item.filterText,
              sortText: item.sortText,
              documentation:
                item.documentation === undefined
                  ? undefined
                  : typeof item.documentation === 'string'
                    ? item.documentation
                    : { value: item.documentation.value },
            };
          });

        // Concatenate pykrete first. Same ordering as the LSP
        // multiplexer's merge_completion — pykrete's items are more
        // specific so the user sees them at the top of the menu.
        return {
          suggestions: [...pykreteSuggestions, ...pyrightSuggestions],
        };
      },
    });

    const definition = monaco.languages.registerDefinitionProvider('python', {
      async provideDefinition(model, position) {
        // Prefer pykrete; fall back to pyright if pykrete returns
        // nothing. Matches the multiplex policy
        // (`merge_locations` in pykrete-lsp prefers pykrete when present).
        let pykreteLoc: LocationPayload | null = null;
        try {
          pykreteLoc = definition_at(
            liveSourceRef.current,
            position.lineNumber,
            position.column,
          ) as LocationPayload | null;
        } catch {
          pykreteLoc = null;
        }
        if (pykreteLoc) {
          return {
            uri: model.uri,
            range: new monaco.Range(
              pykreteLoc.line,
              pykreteLoc.column,
              pykreteLoc.end_line,
              pykreteLoc.end_column,
            ),
          };
        }

        if (!pyrightRef.current || pyrightStateRef.current !== 'ready') return null;

        const pyrightDef = await withTimeout(
          pyrightRef.current.definition(
            toLspPosition(position.lineNumber, position.column),
          ),
          PYRIGHT_REQUEST_TIMEOUT_MS,
          null as LspDefinition | LspDefinitionLink[] | null,
        );
        if (!pyrightDef) return null;
        // Normalise to an array of locations; pyright may return a
        // single Location, an array of Locations, or LocationLinks.
        // Filter to definitions in the playground's own document —
        // anything pointing at typeshed/stub URIs has no editor target.
        const locations = Array.isArray(pyrightDef)
          ? pyrightDef
          : [pyrightDef];
        const out = locations
          .map((loc) => {
            // LocationLink — { targetUri, targetRange, targetSelectionRange }
            const link = loc as Partial<LspDefinitionLink> & {
              targetUri?: string;
              targetRange?: LspRange;
              targetSelectionRange?: LspRange;
            };
            if (link.targetUri && link.targetRange) {
              return {
                uri: link.targetUri,
                range: link.targetSelectionRange ?? link.targetRange,
              };
            }
            // Location — { uri, range }
            const l = loc as { uri?: string; range?: LspRange };
            if (l.uri && l.range) return { uri: l.uri, range: l.range };
            return null;
          })
          .filter((l): l is { uri: string; range: LspRange } => l !== null)
          // pyright may point at typeshed stubs we don't surface in the
          // editor; only return definitions inside the playground doc.
          .filter((l) => l.uri.endsWith('/playground.py'));
        if (out.length === 0) return null;
        return out.map((l) => ({
          uri: model.uri,
          range: toMonacoRange(monaco, l.range),
        }));
      },
    });

    providerDisposablesRef.current = [hover, completion, definition];
    return () => {
      for (const d of providerDisposablesRef.current) d.dispose();
      providerDisposablesRef.current = [];
    };
    // `pyrightState` is intentionally NOT in the dep list. The
    // providers read `pyrightState` and `pyrightRef.current` lexically
    // every time Monaco invokes them, so they always see the current
    // values — re-registering on every pyright state change (loading
    // → ready, or transient `failed` flips) would only thrash
    // Monaco's registry.
  }, [wasmReady, editorReady]);

  const handleMount: OnMount = (editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;
    // Swallow Cmd/Ctrl+S inside the editor. Monaco's default action is
    // "save the editor model", which is a no-op in a browser
    // playground — and without this binding, the browser handles the
    // keystroke and pops its native "Save Page As…" dialog. Neither is
    // what the user wants, so we bind the chord to a no-op command.
    editor.addCommand(
      monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS,
      () => {
        /* no-op: prevent browser save dialog inside the editor */
      },
    );
    // Flip `editorReady` so the provider-registration effect re-runs
    // even if `wasmReady` already turned true while Monaco was still
    // loading its CDN bundle. See the race-condition note on that
    // effect.
    setEditorReady(true);
  };

  const handleChange: OnChange = (value) => {
    setSource(value ?? '');
  };

  const handleSnippetChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const next = SNIPPETS.find((s) => s.id === e.target.value) ?? SNIPPETS[0];
    setSnippetId(next.id);
    setSource(next.source);
  };

  /** Click a diagnostic row → jump the editor to that position and
   * focus it. Lets users use the panel as a fast index into a long
   * snippet. */
  const jumpToDiagnostic = (d: Diagnostic) => {
    const editor = editorRef.current;
    if (!editor) return;
    editor.setPosition({ lineNumber: d.line, column: d.column });
    editor.revealLineInCenter(d.line);
    editor.focus();
  };

  /** A pykrete-CLI-shaped one-liner per diagnostic, e.g.
   * `<playground>.pyk:8:23 - error unknownColumn: Column 'regoin' ...`.
   * The format matches what `pykrete check` prints locally — users
   * familiar with the CLI recognize it instantly. */
  const formatDiagnostic = (d: Diagnostic): string =>
    `<playground>.pyk:${d.line}:${d.column} - ${d.severity} ${d.rule_name}: ${d.message}`;

  /** Main diagnostics-summary status line. Same shape as before:
   * loading / error / counts / empty. Now spans the merged
   * pykrete+pyright diagnostic set. */
  const statusLine = useMemo(() => {
    if (wasmError) return `wasm failed to load: ${wasmError}`;
    if (!wasmReady) return 'loading analyzer…';
    if (mergedDiagnostics.length === 0) {
      return 'no diagnostics — schema and type checks pass.';
    }
    const errors = mergedDiagnostics.filter((d) => d.severity === 'error').length;
    const warnings = mergedDiagnostics.filter((d) => d.severity === 'warning').length;
    const parts: string[] = [];
    if (errors) parts.push(`${errors} error${errors === 1 ? '' : 's'}`);
    if (warnings) parts.push(`${warnings} warning${warnings === 1 ? '' : 's'}`);
    return parts.join(', ');
  }, [wasmReady, wasmError, mergedDiagnostics]);

  /** Pyright-specific status line. Lives next to the main status so
   * the user gets a clear signal that Python-side help is loading or
   * failed. Stays silent on the happy path — the main line covers
   * diagnostics counts for both engines.
   *
   * `null` here means "render nothing" (collapsed line). The component
   * skips the surrounding element entirely when the value is null. */
  const pyrightStatusLine = useMemo<string | null>(() => {
    if (pyrightState === 'loading') return 'loading pyright…';
    if (pyrightState === 'failed') {
      return `pyright failed: ${pyrightError ?? 'unknown error'}`;
    }
    return null;
  }, [pyrightState, pyrightError]);

  return (
    <div className="pk-playground not-content">
      {/* Mobile heads-up: Monaco is unusable on small touch screens.
          The diagnostics list still renders below, which is useful. */}
      <p className="pk-mobile-notice">
        Monaco editor works best on desktop. On a phone, scroll past the
        editor to see the diagnostics list — it updates as you'd type.
      </p>

      <div className="pk-toolbar">
        <label className="pk-snippet-picker">
          <span className="pk-snippet-label">Example:</span>
          <select value={snippetId} onChange={handleSnippetChange}>
            {SNIPPETS.map((s) => (
              <option key={s.id} value={s.id}>
                {s.label}
              </option>
            ))}
          </select>
        </label>
        <p className="pk-snippet-description">{activeSnippet.description}</p>
      </div>

      <div className="pk-editor-wrap">
        <Editor
          height="420px"
          defaultLanguage="python"
          language="python"
          theme="vs-dark"
          value={source}
          onChange={handleChange}
          onMount={handleMount}
          options={{
            minimap: { enabled: false },
            fontSize: 13,
            scrollBeyondLastLine: false,
            renderLineHighlight: 'gutter',
            tabSize: 4,
            insertSpaces: true,
            automaticLayout: true,
            // A little breathing room above the first line and below
            // the last line. Default is 0/0, which makes the top of the
            // file feel cramped against the editor border.
            padding: { top: 8, bottom: 8 },
            // Render hover, suggest, and other overlay widgets at the
            // document body level instead of inside the editor's own
            // DOM. Without this, the playground's wrapper has
            // `overflow: hidden` (so the rounded border crops the
            // editor cleanly), which also clips any hover popup that
            // extends beyond the editor's box — schemas with many
            // fields lose their bottom half. Lifting overlay widgets
            // to the body lets the popup extend anywhere on the page.
            fixedOverflowWidgets: true,
            // Pykrete's entire completion surface lives inside string
            // literals — `col("…")`, `groupBy("…")`, `select("…")`,
            // etc. Monaco's default `quickSuggestions.strings: 'off'`
            // suppresses the auto-trigger-on-typing path inside
            // strings: with the cursor already inside `col("|")` and
            // the user typing an identifier character, providers
            // would not fire. Trigger characters (`"`, `(`, etc.) and
            // manual Ctrl+Space invoke providers regardless of this
            // setting — but identifier-char auto-trigger is what
            // closes the IDE gap for users typing column names mid-
            // string. Override to `strings: true`.
            quickSuggestions: {
              other: true,
              comments: false,
              strings: true,
            },
            // Default is already true, but explicit documents the
            // intent for the trigger-character path (`"`, `(`, etc.).
            suggestOnTriggerCharacters: true,
          }}
        />
      </div>

      <div className="pk-status">{statusLine}</div>
      {pyrightStatusLine && (
        // Second status line — only renders while pyright is still
        // loading or after it failed. On the happy path it disappears
        // entirely; the main `statusLine` above carries the
        // diagnostics summary for both engines combined.
        <div
          className={`pk-status pk-status-pyright pk-status-pyright-${pyrightState}`}
        >
          {pyrightStatusLine}
        </div>
      )}

      <ol className="pk-diagnostics" aria-label="diagnostics">
        {mergedDiagnostics.length === 0 && wasmReady && !wasmError && (
          <li className="pk-empty">
            No diagnostics. Edit the source above to see pykrete and pyright
            in action.
          </li>
        )}
        {mergedDiagnostics.map((d, i) => (
          <li
            key={`${d.code}-${i}-${d.line}-${d.column}`}
            className={`pk-diag pk-diag-${d.severity}`}
          >
            <button
              type="button"
              className="pk-diag-button"
              onClick={() => jumpToDiagnostic(d)}
              title="Jump to this position in the editor"
            >
              {formatDiagnostic(d)}
            </button>
          </li>
        ))}
      </ol>
    </div>
  );
}
