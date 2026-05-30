//! WebAssembly wrapper around the pykrete analyzer.
//!
//! Exposes pykrete's single-file analyzer plus three position-aware
//! LSP capabilities — hover, completion, and go-to-definition — across
//! the wasm-bindgen boundary. The browser playground (Monaco editor +
//! this wasm module, wired up in `docs-site/src/components/Playground.tsx`)
//! is the consumer. No filesystem access, no project-config loading,
//! no cross-file resolution — those all need a real file system and
//! are out of scope for the playground.
//!
//! Each entry point calls a `pykrete::*` public function that the LSP
//! server also uses, so behavior in the playground matches what users
//! get locally:
//!
//! - [`check_source`] → `pykrete::check`
//! - [`hover_at`]     → `pykrete::hover`
//! - [`complete_at`]  → `pykrete::completions`
//! - [`definition_at`] → `pykrete::definition`
//!
//! ## Panic safety
//!
//! A Rust panic that crosses the wasm-bindgen boundary aborts the
//! whole module and tears down the JS host's view of it — every
//! subsequent call would fail with a poisoned-instance error, and the
//! playground would visibly break. The analyzer is well-tested, but
//! the playground feeds it arbitrary user input from a public website,
//! so a defensive `catch_unwind` around every analyzer call is cheap
//! insurance. The four wasm-bindgen entry points each wrap their
//! pykrete call in `catch_unwind`; on panic they degrade to a safe
//! fallback (a `D9999` synthetic diagnostic for `check_source`, null
//! for hover/definition, an empty array for completion).
//!
//! The hook from `console_error_panic_hook` routes panic messages to
//! `console.error` in the browser. That doesn't *prevent* the panic
//! propagating — `catch_unwind` does — but it makes the panic
//! debuggable when someone files a bug report.

use serde::Serialize;
use std::panic;
use wasm_bindgen::prelude::*;

/// One diagnostic, in a shape friendly to a JS / TS host.
///
/// Mirrors `pykrete::diagnostics::Diagnostic` but trims the Rust-only
/// bits (the `min_mode` filter is already applied; the optional
/// `suggestion` is dropped for v1 — the playground doesn't render
/// quick-fixes yet). Line and column are 1-indexed, matching pykrete's
/// CLI output and what Monaco expects for marker positions.
#[derive(Serialize)]
struct DiagnosticOut {
    /// Stable diagnostic identifier, e.g. `"D0030"`.
    code: String,
    /// Friendly rule name, e.g. `"unknownColumn"`.
    rule_name: String,
    /// `"error"` or `"warning"`.
    severity: String,
    message: String,
    line: u32,
    column: u32,
    end_line: u32,
    end_column: u32,
}

/// Hover payload returned across the wasm boundary.
///
/// `contents` is the rendered markdown; the range is the editor-side
/// hint of which token the hover decorates. pykrete's `HoverInfo`
/// doesn't carry a range today (the LSP server omits it and lets the
/// editor compute "word at cursor"), so the wrapper derives the range
/// here from the source — see [`word_range_at`].
#[derive(Serialize)]
struct HoverOut {
    contents: String,
    line: u32,
    column: u32,
    end_line: u32,
    end_column: u32,
}

/// One completion suggestion. `label` is what shows in the dropdown,
/// `insert_text` is what gets inserted (always equal to `label` today,
/// kept as a distinct field so we can grow snippet completions later
/// without an interface break). `kind` is a stringly-typed signal the
/// JS side maps to Monaco's `CompletionItemKind` enum.
#[derive(Serialize)]
struct CompletionOut {
    label: String,
    detail: Option<String>,
    insert_text: String,
    kind: String,
}

/// A source range, 1-indexed, used for go-to-definition results.
#[derive(Serialize)]
struct LocationOut {
    line: u32,
    column: u32,
    end_line: u32,
    end_column: u32,
}

/// Runs once when the wasm module is instantiated. Installs a panic
/// hook so panic messages show up in the browser's devtools console
/// — without this, panics print nothing useful (just a generic wasm
/// trap).
#[wasm_bindgen(start)]
pub fn _start() {
    console_error_panic_hook::set_once();
}

/// Run pykrete on a single `.pyk` source string. Returns a JS array of
/// diagnostic objects (see [`DiagnosticOut`]).
///
/// The path passed to the analyzer is a synthetic `"<playground>.pyk"`
/// — the analyzer uses it for error formatting only, and the
/// single-file mode doesn't trigger any cross-file resolution that
/// would require a real path.
///
/// ## Failure modes
///
/// - If the analyzer panics, this function returns a single synthetic
///   `D9999` "internal error" diagnostic pointing the user at the
///   bug tracker. A visible failure is better than a silent one — a
///   user filing an issue with a reproducer is more useful than
///   "everything looked fine but my code was broken."
/// - If `serde_wasm_bindgen` fails to serialize the diagnostics list
///   (very unlikely — the shape is plain owned strings and numbers),
///   we return an empty array. Returning `JsValue::NULL` would crash
///   JS callers iterating `.length`; an empty array is safe.
#[wasm_bindgen]
pub fn check_source(source: &str) -> JsValue {
    // `catch_unwind` requires an `UnwindSafe` closure. `&str` is
    // unwind-safe, and `run_analyzer` returns an owned `Vec` — no
    // shared mutable state crosses the boundary, so this is fine.
    let result = panic::catch_unwind(|| run_analyzer(source));
    let diagnostics = result.unwrap_or_else(|_| vec![internal_error_diagnostic()]);
    serde_wasm_bindgen::to_value(&diagnostics).unwrap_or_else(|_| {
        // Fall back to an empty JS array, never null — JS code doing
        // `for (const d of result)` or `result.length` would crash on
        // null but handle `[]` cleanly.
        serde_wasm_bindgen::to_value(&Vec::<DiagnosticOut>::new()).expect("empty Vec serializes")
    })
}

/// Hover info at `(line, column)` (1-indexed). Returns the serialized
/// [`HoverOut`] or `JsValue::NULL` when the cursor isn't on a hoverable
/// symbol.
///
/// On panic, returns null — matching the LSP convention that "no
/// hover" is a valid response. A noisy synthetic hover popup on every
/// keystroke would be much more annoying than silent degradation.
#[wasm_bindgen]
pub fn hover_at(source: &str, line: u32, column: u32) -> JsValue {
    let line_usize = line as usize;
    let column_usize = column as usize;
    let result = panic::catch_unwind(|| run_hover(source, line_usize, column_usize));
    let Ok(Some(out)) = result else {
        return JsValue::NULL;
    };
    serde_wasm_bindgen::to_value(&out).unwrap_or(JsValue::NULL)
}

/// Completion suggestions at `(line, column)` (1-indexed). Returns a
/// JS array (possibly empty) of [`CompletionOut`].
///
/// On panic, returns an empty array — same reasoning as
/// [`check_source`]'s fallback: JS code doing `.length` or iteration
/// is safe with `[]` but breaks on null.
#[wasm_bindgen]
pub fn complete_at(source: &str, line: u32, column: u32) -> JsValue {
    let line_usize = line as usize;
    let column_usize = column as usize;
    let result = panic::catch_unwind(|| run_completion(source, line_usize, column_usize));
    let items = result.unwrap_or_default();
    serde_wasm_bindgen::to_value(&items).unwrap_or_else(|_| {
        serde_wasm_bindgen::to_value(&Vec::<CompletionOut>::new()).expect("empty Vec serializes")
    })
}

/// Go-to-definition target at `(line, column)` (1-indexed). Returns
/// the serialized [`LocationOut`] of the declaration site within the
/// same source file, or `JsValue::NULL` when nothing is resolvable.
///
/// Single-file only — the playground has no cross-file imports, so
/// this maps directly to `pykrete::definition`.
#[wasm_bindgen]
pub fn definition_at(source: &str, line: u32, column: u32) -> JsValue {
    let line_usize = line as usize;
    let column_usize = column as usize;
    let result = panic::catch_unwind(|| run_definition(source, line_usize, column_usize));
    let Ok(Some(out)) = result else {
        return JsValue::NULL;
    };
    serde_wasm_bindgen::to_value(&out).unwrap_or(JsValue::NULL)
}

/// Synthetic diagnostic shown when the analyzer panics. The position
/// is `(1, 1, 1, 1)` — there's no real source range to point at, and
/// putting it at the top of the file keeps it visible and out of the
/// way of whatever the user is typing.
fn internal_error_diagnostic() -> DiagnosticOut {
    DiagnosticOut {
        code: "D9999".to_string(),
        rule_name: "internalError".to_string(),
        severity: "error".to_string(),
        message: "pykrete panicked during analysis — please report this as a bug at \
             github.com/amirnaderi93/pykrete/issues"
            .to_string(),
        line: 1,
        column: 1,
        end_line: 1,
        end_column: 1,
    }
}

/// Core analyzer call, factored out so the unit tests below can hit
/// it without paying the wasm-bindgen / `JsValue` round-trip.
fn run_analyzer(source: &str) -> Vec<DiagnosticOut> {
    let result = pykrete::check("<playground>.pyk", source);
    result
        .diagnostics
        .into_iter()
        .map(|d| DiagnosticOut {
            code: d.code.to_string(),
            rule_name: pykrete::diagnostics::rule_name(d.code).to_string(),
            severity: d.severity.label().to_string(),
            message: d.message,
            line: d.line as u32,
            column: d.column as u32,
            end_line: d.end_line as u32,
            end_column: d.end_column as u32,
        })
        .collect()
}

/// Inner hover call — runs pykrete's hover entry point and tags the
/// result with the surrounding word range from the source. Same
/// factoring as `run_analyzer`: the unit tests below exercise this
/// directly.
fn run_hover(source: &str, line: usize, column: usize) -> Option<HoverOut> {
    let info = pykrete::hover(source, line, column)?;
    let (start, end) = word_range_at(source, line, column);
    Some(HoverOut {
        contents: info.markdown,
        line: start.0 as u32,
        column: start.1 as u32,
        end_line: end.0 as u32,
        end_column: end.1 as u32,
    })
}

/// Inner completion call.
fn run_completion(source: &str, line: usize, column: usize) -> Vec<CompletionOut> {
    pykrete::completions(source, line, column)
        .into_iter()
        .map(|item| CompletionOut {
            insert_text: item.label.clone(),
            label: item.label,
            detail: item.detail,
            kind: completion_kind_label(item.kind).to_string(),
        })
        .collect()
}

/// Inner go-to-definition call.
fn run_definition(source: &str, line: usize, column: usize) -> Option<LocationOut> {
    let span = pykrete::definition(source, line, column)?;
    Some(LocationOut {
        line: span.start_line as u32,
        column: span.start_column as u32,
        end_line: span.end_line as u32,
        end_column: span.end_column as u32,
    })
}

/// Map pykrete's pared-down completion kind to the string labels the
/// JS host turns into Monaco's `CompletionItemKind` enum. Keeping the
/// boundary stringly-typed avoids leaking either pykrete's enum or
/// Monaco's into the other side.
fn completion_kind_label(kind: pykrete::CompletionItemKind) -> &'static str {
    match kind {
        pykrete::CompletionItemKind::Class => "schema",
        pykrete::CompletionItemKind::Field => "field",
    }
}

/// Return the 1-indexed `(line, column)` range of the identifier-ish
/// token around `(line, column)` in `source`. Used to anchor the
/// hover popup; Monaco shows the popup decorating this range.
///
/// "Identifier-ish" = a run of ASCII alphanumerics and underscores, or
/// the inside of a string literal up to the surrounding quote. Falls
/// back to a single-character range at the cursor when the cursor
/// isn't on a word character.
///
/// pykrete's hover entry point doesn't currently return a range with
/// its `HoverInfo` — the LSP layer passes `range: None` and lets the
/// editor compute "word at cursor". Doing it here keeps the change
/// scoped to `pykrete-wasm` and gives Monaco a precise range.
fn word_range_at(source: &str, line: usize, column: usize) -> ((usize, usize), (usize, usize)) {
    let is_word_char = |c: char| c.is_ascii_alphanumeric() || c == '_';

    let mut current_line = 1usize;
    let mut current_col = 1usize;
    let mut line_buf: Vec<char> = Vec::new();
    let mut cursor_ch: Option<char> = None;
    let mut chars = source.chars();
    for ch in chars.by_ref() {
        if current_line == line && current_col == column {
            cursor_ch = Some(ch);
            break;
        }
        if ch == '\n' {
            if current_line == line {
                // Hit EOL on the target line before reaching the cursor.
                return ((line, column), (line, column));
            }
            current_line += 1;
            current_col = 1;
            line_buf.clear();
        } else {
            line_buf.push(ch);
            current_col += 1;
        }
    }
    let Some(cur_ch) = cursor_ch else {
        return ((line, column), (line, column));
    };
    if !is_word_char(cur_ch) {
        return ((line, column), (line, column + 1));
    }
    line_buf.push(cur_ch);
    for ch in chars {
        if ch == '\n' {
            break;
        }
        line_buf.push(ch);
    }
    // The cursor char sits at index (column - 1) in line_buf. Columns
    // are 1-indexed, line_buf is 0-indexed, so line_buf[i] is at column
    // i + 1.
    let mut start_col = column;
    while start_col > 1 && is_word_char(line_buf[start_col - 2]) {
        start_col -= 1;
    }
    let mut end_col = column;
    while end_col < line_buf.len() && is_word_char(line_buf[end_col]) {
        end_col += 1;
    }
    // end column is exclusive in LSP / Monaco — point one past the last
    // word char.
    ((line, start_col), (line, end_col + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.pyk` source with no schema issues should produce no
    /// diagnostics. Keeps the wrapper honest — if pykrete starts
    /// flagging a benign file, the test catches it here too.
    #[test]
    fn check_source_with_clean_pyk_returns_no_diagnostics() {
        // `.pyk` syntax: column types are bare atomic names (no
        // quotes), Schema / DataFrame / col are pre-imported by the
        // transpiler — same shape as `examples/orders.pyk`.
        let source = r#"
class Users(Schema):
    id: int
    name: string

def keep(df: DataFrame[Users]) -> DataFrame[Users]:
    return df.select(col("id"), col("name"))
"#;
        let diags = run_analyzer(source);
        assert!(
            diags.is_empty(),
            "expected no diagnostics, got: {:?}",
            diags
                .iter()
                .map(|d| (&d.code, &d.message))
                .collect::<Vec<_>>()
        );
    }

    /// A `.pyk` source that selects a misspelled column should
    /// surface `D0030` (`unknownColumn`).
    #[test]
    fn check_source_with_typo_returns_d0030() {
        let source = r#"
class Users(Schema):
    id: int
    name: string

def keep(df: DataFrame[Users]) -> DataFrame[Users]:
    return df.select(col("id"), col("nme"))
"#;
        let diags = run_analyzer(source);
        assert!(
            diags.iter().any(|d| d.code == "D0030"),
            "expected a D0030 diagnostic, got: {:?}",
            diags
                .iter()
                .map(|d| (&d.code, &d.message))
                .collect::<Vec<_>>()
        );
        let d = diags.iter().find(|d| d.code == "D0030").unwrap();
        assert_eq!(d.rule_name, "unknownColumn");
        assert_eq!(d.severity, "error");
        assert!(d.line >= 1 && d.column >= 1);
    }

    /// A source with a hard Python parse error should surface
    /// `D0001` (`parseError`) and short-circuit further analysis.
    #[test]
    fn check_source_with_invalid_python_returns_d0001() {
        let source = "def broken(\n";
        let diags = run_analyzer(source);
        assert!(
            diags.iter().any(|d| d.code == "D0001"),
            "expected a D0001 diagnostic, got: {:?}",
            diags
                .iter()
                .map(|d| (&d.code, &d.message))
                .collect::<Vec<_>>()
        );
    }

    /// Smoke test for the panic-safety net.
    ///
    /// We can't easily construct an input that makes pykrete itself
    /// panic — the analyzer is well-tested and the wasm wrapper is
    /// thin. But we *can* verify the `catch_unwind` / fallback wiring
    /// works in isolation by panicking from a closure with the same
    /// shape `check_source` uses, then checking the fallback path
    /// produces the synthetic `D9999` diagnostic.
    ///
    /// If pykrete ever does panic on real input, this test confirms
    /// the safety net would catch it instead of tearing down the
    /// wasm host.
    #[test]
    fn catch_unwind_falls_back_to_internal_error_diagnostic() {
        let result: Result<Vec<DiagnosticOut>, _> =
            panic::catch_unwind(|| panic!("simulated analyzer panic"));
        let diagnostics = result.unwrap_or_else(|_| vec![internal_error_diagnostic()]);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "D9999");
        assert_eq!(diagnostics[0].rule_name, "internalError");
        assert_eq!(diagnostics[0].severity, "error");
        assert!(diagnostics[0].message.contains("pykrete panicked"));
        assert!(diagnostics[0].message.contains("github.com"));
    }

    // -----------------------------------------------------------------
    // hover_at
    // -----------------------------------------------------------------

    /// Hover on the Schema class name returns the markdown block
    /// pykrete renders for declarations — heading + fenced python
    /// snippet listing each field.
    #[test]
    fn hover_on_schema_class_name_returns_markdown() {
        // Line 1, column 7 lands inside "Orders".
        let source = "class Orders(Schema):\n    place_code: int\n    price: int\n";
        let out = run_hover(source, 1, 7).expect("expected a hover at the schema name");
        assert!(
            out.contents.contains("**schema** `Orders`"),
            "missing schema heading: {:?}",
            out.contents
        );
        assert!(
            out.contents.contains("```python"),
            "missing python fence: {:?}",
            out.contents
        );
        assert!(
            out.contents.contains("class Orders(Schema):"),
            "missing class line: {:?}",
            out.contents
        );
        // word_range_at should cover the whole identifier "Orders" —
        // columns 7..=12 (inclusive), end column exclusive at 13.
        assert_eq!(out.line, 1);
        assert_eq!(out.column, 7);
        assert_eq!(out.end_line, 1);
        assert_eq!(out.end_column, 13);
    }

    /// Hover on a position with no symbol — a column inside a comment
    /// — returns null. The cursor isn't inside any of the cases hover
    /// recognizes (declarations, references, col-literals, bindings).
    #[test]
    fn hover_on_unrelated_position_returns_null() {
        // Line 2 column 4 is inside the comment ("# just a comment").
        let source = "class Users(Schema):\n    # just a comment\n    id: int\n";
        assert!(run_hover(source, 2, 4).is_none());
    }

    // -----------------------------------------------------------------
    // complete_at
    // -----------------------------------------------------------------

    /// Completion inside a `col("…")` string literal returns the
    /// surrounding schema's column names.
    #[test]
    fn complete_inside_col_string_arg_returns_column_names() {
        // The body line is `return df.select(col(""), col("name"))`.
        // The cursor sits between the two quotes of the empty string
        // — column 26 on line 6 lands inside it.
        let source = r#"class Users(Schema):
    id: int
    name: string
    email: string

def keep(df: DataFrame[Users]) -> DataFrame[Users]:
    return df.select(col(""), col("name"))
"#;
        let items = run_completion(source, 7, 26);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"id"),
            "expected `id` in completions, got: {labels:?}"
        );
        assert!(
            labels.contains(&"name"),
            "expected `name` in completions, got: {labels:?}"
        );
        assert!(
            labels.contains(&"email"),
            "expected `email` in completions, got: {labels:?}"
        );
        // Every column completion is labeled `"field"` so the JS side
        // can map to Monaco's `CompletionItemKind.Field`.
        assert!(
            items.iter().all(|i| i.kind == "field"),
            "expected every item to be a field, got: {:?}",
            items
                .iter()
                .map(|i| (&i.label, &i.kind))
                .collect::<Vec<_>>()
        );
    }

    /// Helper for the bare-string-arg regression tests below: locate the
    /// cursor inside the empty string at `needle` (which should be a
    /// `"<method>(""` substring) and return the 1-indexed `(line, column)`
    /// of the position between the two empty-string quotes. Computing the
    /// position from the source — rather than hardcoding it — keeps the
    /// tests robust against future edits to the schema scaffolding above
    /// the body line.
    fn cursor_inside_empty_string_after(source: &str, needle: &str) -> (usize, usize) {
        let idx = source
            .find(needle)
            .unwrap_or_else(|| panic!("needle {needle:?} not found in source"))
            + needle.len();
        let prefix = &source[..idx];
        let line = prefix.matches('\n').count() + 1;
        let line_start = prefix.rfind('\n').map(|p| p + 1).unwrap_or(0);
        let column = idx - line_start + 1;
        (line, column)
    }

    /// The user's exact reported failure on the playground's "Column
    /// typo" snippet: delete the typo'd column name in `groupBy("regoin")`
    /// and the cursor lands inside `groupBy("")`. The completion
    /// surface should fire and return the receiver dataframe's columns.
    ///
    /// Note: the analyzer's bare-string-arg completion path was already
    /// wired before this round — `pykrete::completions` recognizes the
    /// position via the column-ref trace that `groupBy(...)`'s D0030
    /// pass records. What the playground was actually missing was the
    /// provider-registration race fix in `Playground.tsx` (the
    /// completion-provider effect early-returned when Monaco hadn't
    /// mounted yet and never re-ran). This test pins the analyzer
    /// guarantee so a future refactor doesn't silently drop the bare-
    /// string surface and re-break the user's case from a different
    /// angle.
    #[test]
    fn complete_inside_groupby_string_arg_returns_column_names() {
        let source = "class Sale(Schema):\n    region: string\n    product: string\n    amount: int\n    quantity: int\n\ndef revenue_by_region(sales: DataFrame[Sale]):\n    return sales.groupBy(\"\").agg(F.sum(\"amount\"))\n";
        let (line, column) = cursor_inside_empty_string_after(source, "groupBy(\"");
        let items = run_completion(source, line, column);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"region"),
            "expected `region` in completions, got: {labels:?}"
        );
        assert!(
            labels.contains(&"product") && labels.contains(&"amount"),
            "expected every Sale column in completions, got: {labels:?}"
        );
        assert!(
            items.iter().all(|i| i.kind == "field"),
            "expected every item to be a field, got: {:?}",
            items
                .iter()
                .map(|i| (&i.label, &i.kind))
                .collect::<Vec<_>>()
        );
    }

    /// The same guarantee for `select("...")` — a bare-string arg to
    /// `select` is a column-name position, and the completion list is
    /// the receiver's schema.
    #[test]
    fn complete_inside_select_string_arg_returns_column_names() {
        let source = "class Sale(Schema):\n    region: string\n    amount: int\n\ndef pick(sales: DataFrame[Sale]) -> DataFrame:\n    return sales.select(\"\")\n";
        let (line, column) = cursor_inside_empty_string_after(source, "select(\"");
        let items = run_completion(source, line, column);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"region") && labels.contains(&"amount"),
            "expected `region` and `amount`, got: {labels:?}"
        );
    }

    /// The same guarantee for `F.sum("...")` inside an `agg(...)` call.
    /// `F.sum` is a known aggregate that takes a column name as its
    /// first positional arg; the relevant schema is the grouped
    /// dataframe's underlying schema.
    #[test]
    fn complete_inside_f_sum_string_arg_returns_column_names() {
        let source = "class Sale(Schema):\n    region: string\n    amount: int\n\ndef summarize(sales: DataFrame[Sale]) -> DataFrame:\n    return sales.groupBy(\"region\").agg(F.sum(\"\"))\n";
        let (line, column) = cursor_inside_empty_string_after(source, "F.sum(\"");
        let items = run_completion(source, line, column);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"region") && labels.contains(&"amount"),
            "expected `region` and `amount`, got: {labels:?}"
        );
    }

    /// And `withColumnRenamed("...", "new")` — the FIRST arg is a
    /// column-name reference (the existing column to rename), so the
    /// completion list is the receiver's schema. The SECOND arg is the
    /// new name and is deliberately NOT a completion surface (no test
    /// here for it — completing a fresh name doesn't make sense).
    #[test]
    fn complete_inside_with_column_renamed_first_arg_returns_column_names() {
        let source = "class Sale(Schema):\n    region: string\n    amount: int\n\ndef rename(sales: DataFrame[Sale]) -> DataFrame:\n    return sales.withColumnRenamed(\"\", \"r\")\n";
        let (line, column) = cursor_inside_empty_string_after(source, "withColumnRenamed(\"");
        let items = run_completion(source, line, column);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"region") && labels.contains(&"amount"),
            "expected `region` and `amount`, got: {labels:?}"
        );
    }

    /// Completion at a position where pykrete recognizes no surface
    /// returns an empty list — not an error. Matches the "empty on
    /// no-match" contract of `pykrete::completions`. The playground
    /// then defers to Monaco's built-in word-list completion, which
    /// is fine for early-typing scaffolding.
    #[test]
    fn complete_on_blank_returns_empty() {
        let items = run_completion("", 1, 1);
        assert!(
            items.is_empty(),
            "expected empty completion list, got: {:?}",
            items.iter().map(|i| &i.label).collect::<Vec<_>>()
        );
    }

    /// Completion in a `DataFrame[<cursor>]` schema slot returns
    /// every Schema class in the file, with `kind == "schema"` so the
    /// JS side maps to Monaco's class icon. Uses an existing schema
    /// reference (rather than an empty subscript) because
    /// pykrete::completions matches against the *slice expression's*
    /// range — an empty subscript has no slice expression to anchor.
    #[test]
    fn complete_in_dataframe_schema_slot_returns_schemas() {
        // Line 7: `def keep(df: DataFrame[Users]) -> DataFrame[Users]:`
        // Cursor at column 26 is mid-way through "Users" inside the
        // first DataFrame slot — the slice expression's range covers
        // it, so the completion surface fires.
        let source = r#"class Users(Schema):
    id: int

class Orders(Schema):
    code: int

def keep(df: DataFrame[Users]) -> DataFrame[Users]:
    return df
"#;
        let items = run_completion(source, 7, 26);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"Users") && labels.contains(&"Orders"),
            "expected schema names in completions, got: {labels:?}"
        );
        assert!(
            items.iter().all(|i| i.kind == "schema"),
            "expected every item to be a schema kind, got: {:?}",
            items
                .iter()
                .map(|i| (&i.label, &i.kind))
                .collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------
    // definition_at
    // -----------------------------------------------------------------

    /// Go-to-definition on a `DataFrame[X]` reference jumps to the
    /// `class X(Schema):` declaration in the same file.
    #[test]
    fn definition_on_schema_reference_returns_declaration_position() {
        let source = "class Orders(Schema):\n    id: int\n\ndef keep(df: DataFrame[Orders]):\n    return df\n";
        // Line 4 column 24 lands inside "Orders" in DataFrame[Orders].
        let out = run_definition(source, 4, 24).expect("expected a definition target");
        // class Orders is on line 1 starting at column 7 ("Orders" after "class ").
        assert_eq!(out.line, 1);
        assert_eq!(out.column, 7);
        // Span end is exclusive; "Orders" is 6 chars → end column 13.
        assert_eq!(out.end_line, 1);
        assert_eq!(out.end_column, 13);
    }

    // -----------------------------------------------------------------
    // word_range_at
    // -----------------------------------------------------------------

    /// `word_range_at` expands left and right to cover an identifier
    /// run. The cursor at column 9 inside `Orders` should produce a
    /// range covering all six characters.
    #[test]
    fn word_range_expands_to_full_identifier() {
        let source = "class Orders(Schema):\n";
        let (start, end) = word_range_at(source, 1, 9);
        assert_eq!(start, (1, 7));
        assert_eq!(end, (1, 13));
    }

    /// On a non-word character `word_range_at` returns a single-char
    /// range — enough for Monaco to anchor the hover popup without
    /// over-decorating surrounding tokens.
    #[test]
    fn word_range_on_non_word_char_returns_single_char_range() {
        let source = "class Orders(Schema):\n";
        // Column 6 is the space after "class".
        let (start, end) = word_range_at(source, 1, 6);
        assert_eq!(start, (1, 6));
        assert_eq!(end, (1, 7));
    }

    /// Cursor on the last char of an identifier that ends a line (no
    /// trailing newline) still expands across the whole identifier.
    #[test]
    fn word_range_on_last_char_of_line_expands_full_identifier() {
        let source = "abc def";
        let (start, end) = word_range_at(source, 1, 7);
        assert_eq!(start, (1, 5));
        assert_eq!(end, (1, 8));
    }

    /// Cursor at EOF on a word char expands left to the start of the
    /// identifier.
    #[test]
    fn word_range_at_eof_on_word_char_expands_left() {
        let source = "hello";
        let (start, end) = word_range_at(source, 1, 5);
        assert_eq!(start, (1, 1));
        assert_eq!(end, (1, 6));
    }

    /// Cursor sitting on whitespace returns the single-char range — the
    /// non-word-char branch.
    #[test]
    fn word_range_on_whitespace_returns_single_char_range() {
        let source = "foo bar";
        let (start, end) = word_range_at(source, 1, 4);
        assert_eq!(start, (1, 4));
        assert_eq!(end, (1, 5));
    }

    /// Cursor past the end of a line (column beyond the visible chars)
    /// returns a zero-width range at the requested position — same
    /// sentinel as the original whole-source-allocating implementation.
    #[test]
    fn word_range_past_eol_returns_zero_width_range() {
        let source = "abc\ndef\n";
        let (start, end) = word_range_at(source, 1, 10);
        assert_eq!(start, (1, 10));
        assert_eq!(end, (1, 10));
    }

    /// Single-char identifier at the start of the file.
    #[test]
    fn word_range_single_char_identifier_at_file_start() {
        let source = "x = 1\n";
        let (start, end) = word_range_at(source, 1, 1);
        assert_eq!(start, (1, 1));
        assert_eq!(end, (1, 2));
    }
}
