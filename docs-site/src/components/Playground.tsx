/**
 * Interactive `.pyk` playground.
 *
 * Renders a Monaco editor on the left, a diagnostics panel on the
 * right (or below, on narrow screens), and an example-snippet
 * dropdown above. Every edit (debounced ~300 ms) runs the source
 * through the `pykrete-wasm` analyzer and updates both the
 * in-editor squiggle markers and the diagnostics list.
 *
 * Spark API autocomplete and hover (DataFrame, Column, GroupedData,
 * `F.<fn>`, Window) come from `public/pyspark-symbols.json`, generated
 * at build time by `scripts/gen-pyspark-symbols.py` against a real
 * pyspark install. See `scripts/README.md` for the regeneration
 * recipe.
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
import './Playground.css';

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

interface HoverPayload {
  contents: string;
  line: number;
  column: number;
  end_line: number;
  end_column: number;
}

interface CompletionPayload {
  label: string;
  detail: string | null;
  insert_text: string;
  /** `"field"` or `"schema"` today; stringly-typed so the wasm boundary
   * doesn't leak Monaco's enum back across it. */
  kind: string;
}

interface LocationPayload {
  line: number;
  column: number;
  end_line: number;
  end_column: number;
}

interface Snippet {
  id: string;
  label: string;
  description: string;
  source: string;
}

interface SparkSymbol {
  name: string;
  signature: string;
  doc: string;
}

interface SparkSymbols {
  DataFrame: SparkSymbol[];
  Column: SparkSymbol[];
  GroupedData: SparkSymbol[];
  Window: SparkSymbol[];
  functions: SparkSymbol[];
}

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

function markerSeverity(monaco: Monaco, severity: string): number {
  if (severity === 'warning') return monaco.MarkerSeverity.Warning;
  return monaco.MarkerSeverity.Error;
}

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

/** Module-level cache for the pyspark symbol table. The JSON is a
 * 150 KB static asset; fetch it once on first playground mount and
 * reuse across remounts (React strict mode, hot reload, etc.). */
let sparkSymbolsCache: SparkSymbols | null = null;
let sparkSymbolsPromise: Promise<SparkSymbols | null> | null = null;

async function loadSparkSymbols(): Promise<SparkSymbols | null> {
  if (sparkSymbolsCache) return sparkSymbolsCache;
  if (sparkSymbolsPromise) return sparkSymbolsPromise;
  const baseUrl =
    (import.meta as unknown as { env?: { BASE_URL?: string } }).env
      ?.BASE_URL ?? '/';
  const url =
    (baseUrl.endsWith('/') ? baseUrl : baseUrl + '/') +
    'pyspark-symbols.json';
  sparkSymbolsPromise = (async () => {
    try {
      const res = await fetch(url);
      if (!res.ok) return null;
      const payload = (await res.json()) as Partial<SparkSymbols>;
      const symbols: SparkSymbols = {
        DataFrame: payload.DataFrame ?? [],
        Column: payload.Column ?? [],
        GroupedData: payload.GroupedData ?? [],
        Window: payload.Window ?? [],
        functions: payload.functions ?? [],
      };
      sparkSymbolsCache = symbols;
      return symbols;
    } catch {
      return null;
    }
  })();
  return sparkSymbolsPromise;
}

/** Scan the source for identifiers known to be DataFrames.
 *
 * Recognises:
 *   - `name: DataFrame[Schema]`   (function params, locals)
 *   - `name: DataFrame`           (bare annotation, rare)
 *   - `name = <chain>.<method>(`  where method is a known DataFrame-returning method
 *
 * We only handle the first two precisely; the third is approximated by
 * scanning for `name = ` assignments whose RHS contains `.select(`,
 * `.filter(`, `.withColumn(`, etc. — the heuristic is conservative:
 * misclassifying a non-DataFrame as a DataFrame just means we offer
 * Spark suggestions where they're useless; misclassifying a DataFrame
 * as not-a-DataFrame is the failure mode we want to avoid.
 */
const DATAFRAME_ANNOTATION_RE = /^[ \t]*([A-Za-z_][A-Za-z0-9_]*)\s*:\s*DataFrame\b/;
const PARAM_DATAFRAME_RE = /([A-Za-z_][A-Za-z0-9_]*)\s*:\s*DataFrame\b/g;
const DATAFRAME_RETURNING_METHODS = new Set([
  'select',
  'filter',
  'where',
  'withColumn',
  'withColumnRenamed',
  'drop',
  'distinct',
  'dropDuplicates',
  'orderBy',
  'sort',
  'limit',
  'union',
  'unionAll',
  'unionByName',
  'join',
  'crossJoin',
  'alias',
  'cache',
  'persist',
  'coalesce',
  'repartition',
  'sample',
  'agg',
  'sortWithinPartitions',
  'transform',
]);
const ASSIGN_TO_DF_RE =
  /^[ \t]*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^\n]+)/;

function findDataFrameNames(source: string): Set<string> {
  const names = new Set<string>();
  const lines = source.split('\n');
  for (let li = 0; li < lines.length; li++) {
    const line = lines[li];
    const m1 = line.match(DATAFRAME_ANNOTATION_RE);
    if (m1) names.add(m1[1]);
    PARAM_DATAFRAME_RE.lastIndex = 0;
    let pm: RegExpExecArray | null;
    while ((pm = PARAM_DATAFRAME_RE.exec(line)) !== null) {
      names.add(pm[1]);
    }
    const a = line.match(ASSIGN_TO_DF_RE);
    if (a) {
      // If the RHS opens a paren that doesn't close on this line
      // (`summary = (\n    sales\n    .groupBy(...)\n)`), join the
      // continuation lines until the paren balances, then inspect the
      // combined RHS for a DataFrame-returning method. Conservative on
      // both sides — we stop at the first matching close paren on a
      // line by itself and don't try to track nested string parens.
      let rhs = a[2];
      const open = (rhs.match(/\(/g) ?? []).length;
      const close = (rhs.match(/\)/g) ?? []).length;
      if (open > close) {
        let depth = open - close;
        for (let j = li + 1; j < lines.length && depth > 0; j++) {
          const next = lines[j];
          rhs += '\n' + next;
          depth += (next.match(/\(/g) ?? []).length;
          depth -= (next.match(/\)/g) ?? []).length;
        }
      }
      // Reject obvious literals so `x = 1` doesn't get flagged.
      if (/\.[A-Za-z_]+\s*\(/.test(rhs)) {
        for (const method of DATAFRAME_RETURNING_METHODS) {
          if (rhs.includes('.' + method + '(')) {
            names.add(a[1]);
            break;
          }
        }
      }
    }
  }
  return names;
}

/** Detect whether the receiver before the cursor is a known DataFrame,
 * `F`, or a `.groupBy(...)` chain. Returns the bucket key into
 * `SparkSymbols`, or `null` if no static suggestions apply. */
function dispatchBucket(
  source: string,
  lineText: string,
  lineNumber: number,
  cursorColumn: number,
  dfNames: Set<string>,
): keyof SparkSymbols | null {
  // `cursorColumn` is Monaco 1-indexed; the char to the left of the
  // cursor lives at index `cursorColumn - 2`. For a `.<cursor>` trigger
  // that char is the dot.
  const leftIdx = cursorColumn - 2;
  if (leftIdx < 0) return null;
  if (lineText[leftIdx] !== '.') {
    // Find the last `.` to the left of the cursor on the same line.
    // Monaco fires this provider mid-identifier too — e.g. with cursor
    // after `sales.sel`, we still want to dispatch on `sales`.
    const before = lineText.slice(0, cursorColumn - 1);
    const dotIdx = before.lastIndexOf('.');
    if (dotIdx < 0) return null;
    // Anything between the dot and the cursor must be a partial
    // identifier (no whitespace, no parens) — otherwise we're past
    // the receiver context.
    const partial = before.slice(dotIdx + 1);
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(partial) && partial !== '') {
      return null;
    }
    return classifyReceiver(source, lineText, lineNumber, dotIdx, dfNames);
  }
  return classifyReceiver(source, lineText, lineNumber, leftIdx, dfNames);
}

/** Given the source and the index of the `.` on a line, decide which
 * symbol bucket applies. */
function classifyReceiver(
  source: string,
  lineText: string,
  lineNumber: number,
  dotIdx: number,
  dfNames: Set<string>,
): keyof SparkSymbols | null {
  const before = lineText.slice(0, dotIdx);
  // Identifier immediately before the dot — covers `F.`, `sales.`,
  // `Window.`.
  const ident = before.match(/([A-Za-z_][A-Za-z0-9_]*)\s*$/);
  if (ident) {
    const name = ident[1];
    if (name === 'F') return 'functions';
    if (name === 'Window') return 'Window';
    if (dfNames.has(name)) return 'DataFrame';
  }
  // Method-call chain: `.foo(...).` — look at the previous method name.
  // If it's a known GroupedData-returning method, suggest GroupedData;
  // if it's a known DataFrame-returning method, suggest DataFrame.
  if (before.endsWith(')')) {
    const prevMethod = findPreviousMethodName(source, before, lineNumber);
    if (prevMethod === 'groupBy' || prevMethod === 'rollup' || prevMethod === 'cube') {
      return 'GroupedData';
    }
    if (prevMethod && DATAFRAME_RETURNING_METHODS.has(prevMethod)) {
      return 'DataFrame';
    }
  }
  return null;
}

/** Walk backwards from the `.` to find the previous method-call name in
 * a chained expression. Returns `null` if we can't recover one (e.g.
 * multi-line chain crossing parens we don't track). */
function findPreviousMethodName(
  source: string,
  before: string,
  lineNumber: number,
): string | null {
  // Strip the trailing `)` and walk back balancing parens to find the
  // matching `(`.
  let depth = 0;
  let i = before.length - 1;
  while (i >= 0) {
    const c = before[i];
    if (c === ')') depth++;
    else if (c === '(') {
      depth--;
      if (depth === 0) break;
    }
    i--;
  }
  if (i < 0) {
    // Open paren wasn't on this line — try scanning upward in source
    // for a multi-line chain like:
    //   summary = (
    //       sales
    //       .groupBy("region")
    //       .agg(...)
    //   )
    return findMethodInMultilineChain(source, lineNumber);
  }
  // The method name is the identifier directly before `(`.
  const head = before.slice(0, i);
  const m = head.match(/([A-Za-z_][A-Za-z0-9_]*)\s*$/);
  return m ? m[1] : null;
}

/** Multi-line chain scan: when the cursor's line starts with a dot
 * (`        .foo(...)`), walk upward through preceding `.method(...)`
 * lines and return the most recent method name. */
function findMethodInMultilineChain(
  source: string,
  lineNumber: number,
): string | null {
  // `lineNumber` is Monaco 1-indexed; subtract one for the array index
  // of the cursor's line. We walk backwards from the line *before* the
  // cursor, looking for the most recent `.method(...)` continuation.
  const lines = source.split('\n');
  const cursorIdx = lineNumber - 1;
  if (cursorIdx <= 0 || cursorIdx >= lines.length) return null;
  for (let i = cursorIdx - 1; i >= 0; i--) {
    const candidate = lines[i].trim();
    if (candidate.length === 0) continue;
    const m = candidate.match(/^\.([A-Za-z_][A-Za-z0-9_]*)\s*\(/);
    if (m) return m[1];
    // Non-chain line — stop.
    if (!candidate.startsWith('.')) break;
  }
  return null;
}

/** Returns a value that only updates `delay` ms after the source
 * stops changing. Keeps the analyzer off the keystroke hot path. */
function useDebounced<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const handle = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(handle);
  }, [value, delay]);
  return debounced;
}

/** Build a Monaco completion item from a spark symbol. */
function sparkSymbolToCompletion(
  monaco: Monaco,
  bucket: keyof SparkSymbols,
  sym: SparkSymbol,
  range: import('monaco-editor').IRange,
): languages.CompletionItem {
  const kind =
    bucket === 'functions'
      ? monaco.languages.CompletionItemKind.Function
      : monaco.languages.CompletionItemKind.Method;
  // Build an LSP-style snippet `insertText` if the signature has
  // parameters; otherwise insert the bare name. Monaco recognises
  // `${1:placeholder}` syntax when `insertTextRules` requests it.
  const hasParams = /\([^)]*[^)\s]/.test(sym.signature);
  const insertText = hasParams ? `${sym.name}($0)` : sym.name;
  return {
    label: sym.name,
    detail: sym.signature || undefined,
    documentation: sym.doc
      ? ({ value: sym.doc } as languages.CompletionItem['documentation'])
      : undefined,
    insertText,
    insertTextRules: hasParams
      ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet
      : undefined,
    kind,
    range,
  };
}

/** Build a Monaco hover payload from a spark symbol. */
function sparkSymbolToHover(sym: SparkSymbol): string {
  const sig = sym.signature || '';
  const head = '```python\n' + sym.name + sig + '\n```';
  return sym.doc ? head + '\n\n' + sym.doc : head;
}

export default function Playground() {
  const [snippetId, setSnippetId] = useState<string>(SNIPPETS[0].id);
  const activeSnippet =
    SNIPPETS.find((s) => s.id === snippetId) ?? SNIPPETS[0];
  const [source, setSource] = useState<string>(activeSnippet.source);
  const debouncedSource = useDebounced(source, 300);

  const [wasmReady, setWasmReady] = useState(false);
  const [editorReady, setEditorReady] = useState(false);
  const [wasmError, setWasmError] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<Diagnostic[]>([]);

  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<Monaco | null>(null);
  const providerDisposablesRef = useRef<{ dispose(): void }[]>([]);
  const liveSourceRef = useRef<string>(activeSnippet.source);
  const sparkSymbolsRef = useRef<SparkSymbols | null>(null);

  // Dedicated host node for Monaco's overflow content/overlay widgets
  // (hover popups, suggest menu, parameter hints). Passed to Monaco via
  // `overflowWidgetsDomNode` so the widgets mount under `document.body`
  // instead of the editor's parent, which is inside Starlight's
  // `.main-pane { isolation: isolate }` stacking context that otherwise
  // traps them behind the right-hand TOC sidebar.
  const [overflowRoot] = useState<HTMLDivElement | null>(() => {
    if (typeof document === 'undefined') return null;
    const div = document.createElement('div');
    div.id = 'playground-overflow-root';
    return div;
  });

  useEffect(() => {
    if (!overflowRoot) return;
    document.body.appendChild(overflowRoot);
    return () => {
      overflowRoot.remove();
    };
  }, [overflowRoot]);

  useEffect(() => {
    let cancelled = false;
    init()
      .then(() => {
        if (!cancelled) setWasmReady(true);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setWasmError(
            err instanceof Error ? err.message : String(err),
          );
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Load the pyspark symbol table once. Cached at module level so
  // remounts (React strict mode, hot reload) don't refetch.
  useEffect(() => {
    let cancelled = false;
    loadSparkSymbols().then((syms) => {
      if (!cancelled && syms) sparkSymbolsRef.current = syms;
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    liveSourceRef.current = source;
  }, [source]);

  useEffect(() => {
    if (!wasmReady) return;
    let next: Diagnostic[];
    try {
      next = (check_source(debouncedSource) ?? []) as Diagnostic[];
    } catch (err) {
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

  // Register pykrete's hover / completion / definition providers on
  // the `python` language. Monaco's provider API is global per
  // language ID — registering once and reading the source from a ref
  // lets the providers stay responsive without re-registering on every
  // keystroke. The wasm-ready flag gates registration so we never call
  // the wasm exports before they exist.
  //
  // `editorReady` is tracked as React state (set in `handleMount`) so
  // this effect re-runs the moment Monaco finishes loading — without
  // it, a fast wasm boot beating Monaco's CDN fetch would leave the
  // providers never registered.
  useEffect(() => {
    if (!wasmReady || !editorReady) return;
    const monaco = monacoRef.current;
    if (!monaco) return;
    for (const d of providerDisposablesRef.current) d.dispose();
    providerDisposablesRef.current = [];

    const hover = monaco.languages.registerHoverProvider('python', {
      provideHover(_model, position) {
        // pykrete first — schema hovers, identifier-to-schema, etc.
        // take precedence over the static Spark API table.
        try {
          const out = hover_at(
            liveSourceRef.current,
            position.lineNumber,
            position.column,
          ) as HoverPayload | null;
          if (out) {
            return {
              range: new monaco.Range(
                out.line,
                out.column,
                out.end_line,
                out.end_column,
              ),
              contents: [{ value: out.contents }],
            };
          }
        } catch {
          /* fall through to spark table */
        }

        const symbols = sparkSymbolsRef.current;
        if (!symbols) return null;
        const model = _model;
        const lineText = model.getLineContent(position.lineNumber);
        const wordInfo = model.getWordAtPosition(position);
        if (!wordInfo) return null;
        const word = wordInfo.word;
        // Look up the identifier just before the dot to decide which
        // bucket to search.
        const before = lineText.slice(0, wordInfo.startColumn - 1);
        if (!before.endsWith('.')) return null;
        const dfNames = findDataFrameNames(liveSourceRef.current);
        const bucket = classifyReceiver(
          liveSourceRef.current,
          lineText,
          position.lineNumber,
          before.length - 1,
          dfNames,
        );
        if (!bucket) return null;
        const sym = symbols[bucket].find((s) => s.name === word);
        if (!sym) return null;
        return {
          range: new monaco.Range(
            position.lineNumber,
            wordInfo.startColumn,
            position.lineNumber,
            wordInfo.endColumn,
          ),
          contents: [{ value: sparkSymbolToHover(sym) }],
        };
      },
    });

    const completion = monaco.languages.registerCompletionItemProvider('python', {
      // Auto-open completion on the surfaces where a pykrete or Spark
      // completion is meaningful. Identifier chars are NOT triggers —
      // Monaco already pops the menu mid-identifier; listing them
      // would re-fire the provider noisily inside every name.
      triggerCharacters: ['"', "'", '(', '[', '.'],
      provideCompletionItems(model, position) {
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

        // Spark API surface — driven by the static symbol table.
        let sparkSuggestions: languages.CompletionItem[] = [];
        const symbols = sparkSymbolsRef.current;
        if (symbols && pykreteSuggestions.length === 0) {
          const lineText = model.getLineContent(position.lineNumber);
          const dfNames = findDataFrameNames(liveSourceRef.current);
          const bucket = dispatchBucket(
            liveSourceRef.current,
            lineText,
            position.lineNumber,
            position.column,
            dfNames,
          );
          if (bucket) {
            sparkSuggestions = symbols[bucket].map((sym) =>
              sparkSymbolToCompletion(monaco, bucket, sym, fallbackRange),
            );
          }
        }

        return {
          suggestions: [...pykreteSuggestions, ...sparkSuggestions],
        };
      },
    });

    const definition = monaco.languages.registerDefinitionProvider('python', {
      provideDefinition(model, position) {
        try {
          const loc = definition_at(
            liveSourceRef.current,
            position.lineNumber,
            position.column,
          ) as LocationPayload | null;
          if (!loc) return null;
          return {
            uri: model.uri,
            range: new monaco.Range(
              loc.line,
              loc.column,
              loc.end_line,
              loc.end_column,
            ),
          };
        } catch {
          return null;
        }
      },
    });

    providerDisposablesRef.current = [hover, completion, definition];
    return () => {
      for (const d of providerDisposablesRef.current) d.dispose();
      providerDisposablesRef.current = [];
    };
  }, [wasmReady, editorReady]);

  const handleMount: OnMount = (editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;
    // Swallow Cmd/Ctrl+S — Monaco's default action saves the model,
    // a no-op in the browser, and without this binding the browser
    // shows its native "Save Page As…" dialog.
    editor.addCommand(
      monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS,
      () => {
        /* no-op */
      },
    );
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

  const jumpToDiagnostic = (d: Diagnostic) => {
    const editor = editorRef.current;
    if (!editor) return;
    editor.setPosition({ lineNumber: d.line, column: d.column });
    editor.revealLineInCenter(d.line);
    editor.focus();
  };

  const formatDiagnostic = (d: Diagnostic): string =>
    `<playground>.pyk:${d.line}:${d.column} - ${d.severity} ${d.rule_name}: ${d.message}`;

  const statusLine = useMemo(() => {
    if (wasmError) return `wasm failed to load: ${wasmError}`;
    if (!wasmReady) return 'loading analyzer…';
    if (diagnostics.length === 0) {
      return 'no diagnostics — schema and type checks pass.';
    }
    const errors = diagnostics.filter((d) => d.severity === 'error').length;
    const warnings = diagnostics.filter((d) => d.severity === 'warning').length;
    const parts: string[] = [];
    if (errors) parts.push(`${errors} error${errors === 1 ? '' : 's'}`);
    if (warnings) parts.push(`${warnings} warning${warnings === 1 ? '' : 's'}`);
    return parts.join(', ');
  }, [wasmReady, wasmError, diagnostics]);

  return (
    <div className="pk-playground not-content">
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
            padding: { top: 8, bottom: 8 },
            // See `overflowRoot` above for the rationale; together these
            // two options move Monaco's hover/suggest/parameter-hint
            // widgets onto our body-level host node, escaping both the
            // playground wrapper's `overflow: hidden` and Starlight's
            // trapped stacking context.
            fixedOverflowWidgets: true,
            overflowWidgetsDomNode: overflowRoot ?? undefined,
            // Pykrete's completion surface lives mostly inside string
            // literals (`col("…")`, `groupBy("…")`, …). Monaco's default
            // disables auto-trigger inside strings; override so the
            // provider fires as the user types column-name characters.
            quickSuggestions: {
              other: true,
              comments: false,
              strings: true,
            },
            suggestOnTriggerCharacters: true,
          }}
        />
      </div>

      <div className="pk-status">{statusLine}</div>

      <ol className="pk-diagnostics" aria-label="diagnostics">
        {diagnostics.length === 0 && wasmReady && !wasmError && (
          <li className="pk-empty">
            No diagnostics. Edit the source above to see pykrete in action.
          </li>
        )}
        {diagnostics.map((d, i) => (
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
