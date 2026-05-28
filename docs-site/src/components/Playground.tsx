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
import type { editor } from 'monaco-editor';
import init, { check_source } from 'pykrete-wasm';
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
      'pykrete tracks the schema through withColumn / drop / groupBy / agg.',
    source: `class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int

def report(sales: DataFrame[Sale]):
    summary = (
        sales
        .withColumn("revenue", F.col("amount") * F.col("quantity"))
        .drop("amount", "quantity")
        .groupBy("region")
        .agg(F.sum("revenue").alias("total"))
    )
    # Hover \`summary\` in your editor (locally) — schema is
    # \`region: string, product: string, total: long\`.
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
  const [wasmError, setWasmError] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<Diagnostic[]>([]);

  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<Monaco | null>(null);

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

  // Re-run the analyzer whenever the debounced source or the wasm-ready
  // flag changes. Surfaces both as in-editor markers and as a list
  // below the editor.
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

    // Push markers into Monaco so the user sees red squiggles at the
    // right positions.
    const editor = editorRef.current;
    const monaco = monacoRef.current;
    if (editor && monaco) {
      const model = editor.getModel();
      if (model) {
        const markers = next.map((d) => ({
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
      }
    }
  }, [debouncedSource, wasmReady]);

  const handleMount: OnMount = (editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;
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

  const statusLine = useMemo(() => {
    if (wasmError) return `wasm failed to load: ${wasmError}`;
    if (!wasmReady) return 'loading analyzer…';
    if (diagnostics.length === 0) return 'no diagnostics — schema checks pass.';
    const errors = diagnostics.filter((d) => d.severity === 'error').length;
    const warnings = diagnostics.filter((d) => d.severity === 'warning').length;
    const parts: string[] = [];
    if (errors) parts.push(`${errors} error${errors === 1 ? '' : 's'}`);
    if (warnings) parts.push(`${warnings} warning${warnings === 1 ? '' : 's'}`);
    return parts.join(', ');
  }, [wasmReady, wasmError, diagnostics]);

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
          }}
        />
      </div>

      <div className="pk-status">{statusLine}</div>

      <ol className="pk-diagnostics" aria-label="diagnostics">
        {diagnostics.length === 0 && wasmReady && !wasmError && (
          <li className="pk-empty">No diagnostics. Edit the source above to see pykrete in action.</li>
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
