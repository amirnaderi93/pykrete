//! v1.1 enum constraints — parser, ColumnType representation, schema
//! operators, and D0084 reservation (PR-A scope only).
//!
//! Check-site emission of D0084 — `==`, `.isin`, `.fillna`, `lit`,
//! `F.expr`, and branch-form expressions — lands in PR-B. These tests
//! pin down what PR-A delivers: the type-system foundation that PR-B
//! plugs into.

mod common;

use common::{
    assert_does_not_have_code, assert_has_code, assert_message_contains, assert_no_diagnostics,
    check, check_strict,
};

use pykrete::diagnostics::DIAGNOSTIC_CATALOG;
use pykrete::types::{ColumnType, EnumParseError, render_enum_vocab};

// ---------------------------------------------------------------------------
// Schema parser — `enum["a", "b", ...]` as a type annotation
// ---------------------------------------------------------------------------

#[test]
fn schema_declares_enum_with_two_values() {
    let src = "\
class Order(Schema):
    id: long
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.select(col(\"status\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn schema_declares_enum_with_a_single_value() {
    let src = "\
class Order(Schema):
    status: enum[\"only\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.select(col(\"status\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn empty_enum_vocabulary_is_rejected_via_the_enum_from_values_api() {
    // `enum[]` is a Python syntax error (the subscript can't be empty),
    // so D0001 fires before pykrete sees a subscript shape. The
    // `EnumParseError::Empty` guard exists for the type-level API where
    // the values vector can be empty in principle; it is the spec-named
    // safety net per the "rejected at parse time" guarantee.
    assert_eq!(
        ColumnType::enum_from_values(Vec::new()),
        Err(EnumParseError::Empty),
    );
}

#[test]
fn duplicate_enum_entry_fires_d0011_and_names_the_duplicate() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"pending\"]
";
    let result = check(src);
    assert_has_code(&result, "D0011");
    assert_message_contains(&result, "D0011", "pending");
    assert_message_contains(&result, "D0011", "unique");
}

#[test]
fn enum_values_are_case_sensitive() {
    // `enum["Pending"]` and `enum["pending"]` are distinct vocabularies —
    // no normalization happens at parse time per the spec.
    let values_a = vec!["Pending".to_string()];
    let values_b = vec!["pending".to_string()];
    let a = ColumnType::enum_from_values(values_a).unwrap();
    let b = ColumnType::enum_from_values(values_b).unwrap();
    assert_ne!(a, b, "enum vocabularies must be byte-exact");
    assert!(!ColumnType::enum_vocab_eq(&a, &b));
}

#[test]
fn enum_values_preserve_unicode_and_whitespace_exactly() {
    let values = vec![
        "café".to_string(),
        "us-east".to_string(),
        "pending ".to_string(),
    ];
    let ct = ColumnType::enum_from_values(values.clone()).unwrap();
    let ColumnType::Enum(actual) = ct else {
        panic!("expected Enum variant");
    };
    assert_eq!(actual, values, "raw byte-for-byte preservation expected");
}

#[test]
fn enum_from_values_rejects_empty() {
    assert_eq!(
        ColumnType::enum_from_values(Vec::new()),
        Err(EnumParseError::Empty),
    );
}

#[test]
fn enum_from_values_rejects_duplicate_and_names_first_repeat() {
    assert_eq!(
        ColumnType::enum_from_values(vec!["a".into(), "b".into(), "a".into()]),
        Err(EnumParseError::Duplicate("a".into())),
    );
}

// ---------------------------------------------------------------------------
// Set-equality on enum vocabularies — Q4 / Q9 semantics
// ---------------------------------------------------------------------------

#[test]
fn enum_vocab_eq_ignores_declaration_order() {
    let a = ColumnType::enum_from_values(vec!["a".into(), "b".into(), "c".into()]).unwrap();
    let b = ColumnType::enum_from_values(vec!["c".into(), "a".into(), "b".into()]).unwrap();
    assert!(ColumnType::enum_vocab_eq(&a, &b));
}

#[test]
fn enum_vocab_eq_is_false_when_one_side_has_an_extra_value() {
    let a = ColumnType::enum_from_values(vec!["a".into(), "b".into()]).unwrap();
    let b = ColumnType::enum_from_values(vec!["a".into(), "b".into(), "c".into()]).unwrap();
    assert!(!ColumnType::enum_vocab_eq(&a, &b));
}

#[test]
fn enum_vocab_eq_peels_nullable_wrappers() {
    let inner_a = ColumnType::enum_from_values(vec!["a".into(), "b".into()]).unwrap();
    let inner_b = ColumnType::enum_from_values(vec!["b".into(), "a".into()]).unwrap();
    let a = ColumnType::Nullable(Box::new(inner_a));
    let b = ColumnType::Nullable(Box::new(inner_b));
    assert!(ColumnType::enum_vocab_eq(&a, &b));
}

#[test]
fn enum_vocab_eq_is_false_against_a_non_enum_type() {
    let enum_ty = ColumnType::enum_from_values(vec!["a".into()]).unwrap();
    assert!(!ColumnType::enum_vocab_eq(&enum_ty, &ColumnType::String));
}

// ---------------------------------------------------------------------------
// Hover / completion rendering — truncate-to-5
// ---------------------------------------------------------------------------

#[test]
fn render_enum_vocab_inlines_all_values_up_to_five() {
    let values: Vec<String> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        render_enum_vocab(&values),
        "\"a\", \"b\", \"c\", \"d\", \"e\""
    );
}

#[test]
fn render_enum_vocab_truncates_past_five_values_with_more_suffix() {
    let values: Vec<String> = ["a", "b", "c", "d", "e", "f", "g", "h"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        render_enum_vocab(&values),
        "\"a\", \"b\", \"c\", \"d\", \"e\", ... (3 more)",
    );
}

#[test]
fn enum_display_format_uses_the_shared_render_helper() {
    let ct = ColumnType::enum_from_values(vec!["a".into(), "b".into()]).unwrap();
    assert_eq!(format!("{ct}"), "enum[\"a\", \"b\"]");
}

#[test]
fn enum_as_str_is_the_bare_kind_name() {
    // Used by symbol/outline renders that don't want the inline
    // vocabulary; hover and completion go through the full render
    // helper instead.
    let ct = ColumnType::enum_from_values(vec!["a".into()]).unwrap();
    assert_eq!(ct.as_str(), "Enum");
}

// ---------------------------------------------------------------------------
// Schema operators — Pick / Omit / Merge carry-through
// ---------------------------------------------------------------------------

#[test]
fn pick_preserves_enum_constraint_on_kept_column() {
    let src = "\
class Order(Schema):
    id: long
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Pick[Order, \"status\"]]) -> DataFrame:
    return orders.select(col(\"status\"))
";
    assert_no_diagnostics(&check(src));
    // Vacuity guard: confirm the surviving `status` column is still
    // typed as the same enum vocabulary, not silently downgraded to
    // plain String. Without this, the operator could strip the
    // constraint and the no-diagnostics check above would still pass.
    let pick_status_type = resolve_derived_status_type(src, "DataFrame[Pick[Order, \"status\"]]");
    assert_eq!(
        pick_status_type,
        Some(ColumnType::enum_from_values(vec!["pending".into(), "shipped".into()]).unwrap()),
    );
}

#[test]
fn omit_preserves_enum_constraint_on_surviving_column() {
    let src = "\
class Order(Schema):
    id: long
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Omit[Order, \"id\"]]) -> DataFrame:
    return orders.select(col(\"status\"))
";
    assert_no_diagnostics(&check(src));
    let omit_status_type = resolve_derived_status_type(src, "DataFrame[Omit[Order, \"id\"]]");
    assert_eq!(
        omit_status_type,
        Some(ColumnType::enum_from_values(vec!["pending".into(), "shipped".into()]).unwrap()),
    );
}

/// Re-parse `src` and return the `ColumnType` of the `status` column
/// in the derived schema expression named by `needle` (the DataFrame
/// annotation text the test source uses). Used by the Pick/Omit
/// preservation tests to assert the operator carries the enum vocabulary
/// through, not just that the source produces no diagnostics.
fn resolve_derived_status_type(src: &str, needle: &str) -> Option<ColumnType> {
    use pykrete::schema::{discover_schemas, resolve_derived_schema};
    use pykrete::walk::discover_top_level_classes;
    use ruff_python_ast::{Expr, Stmt};
    use ruff_text_size::Ranged;

    let parsed = ruff_python_parser::parse_module(src).expect("test source must parse");
    let module = parsed.syntax();
    let classes = discover_top_level_classes(module);
    let schemas = discover_schemas(&classes);

    for stmt in &module.body {
        let Stmt::FunctionDef(func) = stmt else {
            continue;
        };
        for param in &func.parameters.args {
            let Some(ann) = param.parameter.annotation.as_deref() else {
                continue;
            };
            if &src[ann.range()] != needle {
                continue;
            }
            let Expr::Subscript(sub) = ann else { continue };
            let view = resolve_derived_schema(&sub.slice, &schemas)?;
            return view.field_type("status", &schemas);
        }
    }
    None
}

#[test]
fn merge_with_set_equal_enum_vocabularies_is_silent() {
    // Declaration order differs; the vocabulary set is the same. Per Q4,
    // no D0040 fires.
    let src = "\
class A(Schema):
    status: enum[\"pending\", \"shipped\"]

class B(Schema):
    status: enum[\"shipped\", \"pending\"]

def f(d: DataFrame[Merge[A, B]]) -> DataFrame:
    return d.select(col(\"status\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn merge_with_non_set_equal_enum_vocabularies_fires_d0040() {
    let src = "\
class A(Schema):
    status: enum[\"pending\", \"shipped\"]

class B(Schema):
    status: enum[\"pending\", \"delivered\"]

def f(d: DataFrame[Merge[A, B]]) -> DataFrame:
    return d
";
    let result = check(src);
    assert_has_code(&result, "D0040");
    assert_message_contains(&result, "D0040", "status");
}

#[test]
fn merge_with_enum_vs_plain_string_fires_d0040() {
    let src = "\
class A(Schema):
    status: enum[\"pending\", \"shipped\"]

class B(Schema):
    status: string

def f(d: DataFrame[Merge[A, B]]) -> DataFrame:
    return d
";
    let result = check(src);
    assert_has_code(&result, "D0040");
    assert_message_contains(&result, "D0040", "status");
}

#[test]
fn merge_with_only_one_side_declaring_status_is_silent() {
    let src = "\
class A(Schema):
    status: enum[\"pending\", \"shipped\"]
    amount: int

class B(Schema):
    rating: int

def f(d: DataFrame[Merge[A, B]]) -> DataFrame:
    return d.select(col(\"status\"), col(\"amount\"), col(\"rating\"))
";
    assert_no_diagnostics(&check(src));
}

// ---------------------------------------------------------------------------
// D0084 reservation in the diagnostic catalog
// ---------------------------------------------------------------------------

#[test]
fn d0084_is_reserved_in_the_diagnostic_catalog() {
    let entry = DIAGNOSTIC_CATALOG
        .iter()
        .find(|(code, _)| *code == "D0084")
        .expect("DIAGNOSTIC_CATALOG should reserve D0084 in PR-A");
    assert_eq!(entry.1, "enumValueMismatch");
}

#[test]
fn d0084_immediately_follows_d0083_in_the_catalog() {
    // The spec is explicit: D0084 is appended at the end of
    // DIAGNOSTIC_CATALOG, immediately after the existing D0083 entry.
    let d83_idx = DIAGNOSTIC_CATALOG
        .iter()
        .position(|(c, _)| *c == "D0083")
        .expect("D0083 must be in the catalog");
    let d84_idx = DIAGNOSTIC_CATALOG
        .iter()
        .position(|(c, _)| *c == "D0084")
        .expect("D0084 must be in the catalog");
    assert_eq!(d84_idx, d83_idx + 1);
}

// ---------------------------------------------------------------------------
// PR-B check sites — D0084 emission across spec-defined sites.
//
// Each category exercises a positive case (off-vocab → fire), a negative
// case (in-vocab → silent), a precedence case (D0030 / D0082 shadows),
// and a suggestion case (`Did you mean …`).
// ---------------------------------------------------------------------------

#[test]
fn d0084_fires_on_off_enum_literal_comparison() {
    let src = "\
class Order(Schema):
    id: long
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"status\") == \"pendig\")
";
    let result = check(src);
    assert_has_code(&result, "D0084");
    assert_message_contains(&result, "D0084", "'pendig'");
    assert_message_contains(&result, "D0084", "status");
    assert_message_contains(&result, "D0084", "pending");
}

#[test]
fn d0084_silent_for_in_vocab_literal_comparison() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"status\") == \"pending\")
";
    assert_does_not_have_code(&check(src), "D0084");
}

#[test]
fn d0084_fires_when_lit_wraps_the_off_vocab_literal() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"status\") == lit(\"pendig\"))
";
    let result = check(src);
    assert_has_code(&result, "D0084");
    assert_message_contains(&result, "D0084", "pending");
}

#[test]
fn d0084_fires_on_swapped_lhs_rhs_compare() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(\"pendig\" == col(\"status\"))
";
    assert_has_code(&check(src), "D0084");
}

#[test]
fn d0084_precedence_d0030_unknown_column_blocks_enum_check() {
    // Q1c: D0030 > D0082 > D0084. An unknown column fires D0030; the
    // column never resolves so D0084 has nothing to check against.
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"stutus\") == \"pendig\")
";
    let result = check(src);
    assert_has_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0084");
}

#[test]
fn d0084_precedence_d0082_cross_type_blocks_enum_check() {
    // Q1c: a boolean RHS coerces against an enum-typed (Textual) LHS —
    // D0082 fires in strict mode; D0084 doesn't, because the RHS isn't
    // a string literal at all.
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"status\") == True)
";
    let result = check_strict(src);
    assert_has_code(&result, "D0082");
    assert_does_not_have_code(&result, "D0084");
}

#[test]
fn d0084_suggestion_picks_closest_within_levenshtein_threshold() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\", \"delivered\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"status\") == \"shippd\")
";
    let result = check(src);
    let diag = result
        .diagnostics_with_code("D0084")
        .into_iter()
        .next()
        .expect("D0084 must fire");
    assert_eq!(diag.suggestion.as_deref(), Some("shipped"));
}

#[test]
fn d0084_suggestion_breaks_ties_by_unicode_code_point_order() {
    // Both "pendinx" and "pendiny" are edit-distance-1 from "pendinz";
    // the spec's locked tiebreaker is Rust `str::cmp` (Unicode
    // code-point order), so "pendinx" wins. Falsifies a tiebreaker by
    // declaration order or a non-deterministic pick.
    let src = "\
class Order(Schema):
    status: enum[\"pendiny\", \"pendinx\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"status\") == \"pendinz\")
";
    let result = check(src);
    let diag = result
        .diagnostics_with_code("D0084")
        .into_iter()
        .next()
        .expect("D0084 must fire");
    assert_eq!(diag.suggestion.as_deref(), Some("pendinx"));
}

#[test]
fn d0084_isin_fires_per_off_vocab_arg_only() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"status\").isin(\"pending\", \"shippd\"))
";
    let result = check(src);
    common::assert_count(&result, "D0084", 1);
    assert_message_contains(&result, "D0084", "shippd");
}

#[test]
fn d0084_isin_silent_when_every_arg_in_vocab() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"status\").isin(\"pending\", \"shipped\"))
";
    assert_does_not_have_code(&check(src), "D0084");
}

#[test]
fn d0084_isin_unknown_column_precedence_blocks_enum_check() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"stutus\").isin(\"pendig\"))
";
    let result = check(src);
    assert_has_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0084");
}

#[test]
fn d0084_fillna_dict_value_fires_on_off_vocab() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.fillna({\"status\": \"unkown\"})
";
    let result = check(src);
    assert_has_code(&result, "D0084");
    assert_message_contains(&result, "D0084", "unkown");
}

#[test]
fn d0084_fillna_dict_value_silent_when_in_vocab() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.fillna({\"status\": \"pending\"})
";
    assert_does_not_have_code(&check(src), "D0084");
}

#[test]
fn d0084_with_column_lit_fires_on_off_vocab() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(\"status\", lit(\"delivered\"))
";
    let result = check(src);
    assert_has_code(&result, "D0084");
    assert_message_contains(&result, "D0084", "delivered");
}

#[test]
fn d0084_with_column_lit_silent_when_in_vocab() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(\"status\", lit(\"pending\"))
";
    assert_does_not_have_code(&check(src), "D0084");
}

#[test]
fn d0084_with_columns_dict_value_fires_per_off_vocab_entry() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumns({\"status\": lit(\"shippd\")})
";
    let result = check(src);
    assert_has_code(&result, "D0084");
    assert_message_contains(&result, "D0084", "shippd");
}

// ---------------------------------------------------------------------------
// PR-B branch-form expressions (Q9)
// ---------------------------------------------------------------------------

#[test]
fn d0084_branch_form_coalesce_fires_on_off_vocab_literal_at_sink() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(\"status\", coalesce(col(\"status\"), lit(\"shippd\")))
";
    let result = check(src);
    assert_has_code(&result, "D0084");
    assert_message_contains(&result, "D0084", "shippd");
}

#[test]
fn d0084_branch_form_nvl_fires_on_off_vocab_literal_at_sink() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(\"status\", nvl(col(\"status\"), lit(\"shippd\")))
";
    assert_has_code(&check(src), "D0084");
}

#[test]
fn d0084_branch_form_ifnull_fires_on_off_vocab_literal_at_sink() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(\"status\", ifnull(col(\"status\"), lit(\"shippd\")))
";
    assert_has_code(&check(src), "D0084");
}

#[test]
fn d0084_branch_form_nullif_fires_on_off_vocab_literal_at_sink() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(\"status\", nullif(col(\"status\"), lit(\"shippd\")))
";
    assert_has_code(&check(src), "D0084");
}

#[test]
fn d0084_branch_form_when_otherwise_fires_per_off_vocab_branch() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(
        \"status\",
        when(col(\"status\").isNull(), lit(\"shippd\")).otherwise(lit(\"delvr\"))
    )
";
    let result = check(src);
    common::assert_count(&result, "D0084", 2);
}

#[test]
fn d0084_branch_form_silent_when_every_literal_is_in_vocab() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(\"status\", coalesce(col(\"status\"), lit(\"pending\")))
";
    assert_does_not_have_code(&check(src), "D0084");
}

#[test]
fn coalesce_preserves_enum_constraint_when_all_branches_set_equal() {
    // Vacuity-falsifiable per round-2 review: chain the coalesce result
    // through a fresh `staged` column then a downstream `==` against an
    // off-vocab literal. If `coalesce_branch_result` were a no-op
    // returning `None`, `staged` would lose the enum constraint and the
    // downstream D0084 below would NOT fire — turning this test red.
    let src = "\
class Order(Schema):
    primary: enum[\"pending\", \"shipped\"]
    fallback: enum[\"shipped\", \"pending\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(
        \"staged\", coalesce(col(\"primary\"), col(\"fallback\"))
    ).filter(col(\"staged\") == \"delivered\")
";
    let result = check(src);
    assert_has_code(&result, "D0084");
    assert_message_contains(&result, "D0084", "delivered");
}

#[test]
fn coalesce_drops_enum_constraint_when_a_branch_is_plain_string() {
    // Vacuity-falsifiable per round-2 review: a plain-string branch
    // drops the constraint per Q9, so the chained downstream
    // `staged == 'off-vocab'` check must NOT fire D0084 (the constraint
    // is gone). A no-op `coalesce_branch_result` returning `None` would
    // also not fire, but the sibling positive test above pins the
    // preservation path — together they bracket the behavior.
    let src = "\
class Order(Schema):
    primary: enum[\"pending\", \"shipped\"]
    plain: string

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(
        \"staged\", coalesce(col(\"primary\"), col(\"plain\"))
    ).filter(col(\"staged\") == \"delivered\")
";
    assert_does_not_have_code(&check(src), "D0084");
}

// ---------------------------------------------------------------------------
// PR-B F.expr SQL parser extension (Q7)
// ---------------------------------------------------------------------------

#[test]
fn d0084_f_expr_equality_fires_on_off_vocab_literal() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(F.expr(\"status = 'pendig'\"))
";
    let result = check(src);
    assert_has_code(&result, "D0084");
    assert_message_contains(&result, "D0084", "pendig");
}

#[test]
fn d0084_f_expr_in_clause_fires_only_on_off_vocab_members() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(F.expr(\"status IN ('pending', 'shippd')\"))
";
    let result = check(src);
    common::assert_count(&result, "D0084", 1);
    assert_message_contains(&result, "D0084", "shippd");
}

#[test]
fn d0084_f_expr_silent_when_rhs_is_a_non_literal_identifier() {
    // SQL `status = some_var` — the RHS is a column reference, not a
    // string literal. Per Q7 conservative shape, no D0084 fires; the
    // existence check on `some_var` is its own D0030 (which we don't
    // care about here).
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]
    region: string

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(F.expr(\"status = region\"))
";
    assert_does_not_have_code(&check(src), "D0084");
}

#[test]
fn d0084_f_expr_silent_when_literal_in_vocab() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(F.expr(\"status = 'pending'\"))
";
    assert_does_not_have_code(&check(src), "D0084");
}

#[test]
fn d0084_f_expr_precedence_unknown_column_blocks_enum_check() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(F.expr(\"stutus = 'pendig'\"))
";
    let result = check(src);
    assert_has_code(&result, "D0030");
    // The column doesn't resolve, so the enum check has nothing to run
    // against — Q1c precedence keeps D0084 silent here.
    assert_does_not_have_code(&result, "D0084");
}

// ---------------------------------------------------------------------------
// Round-2 — set-equality / exhaustiveness / wrapper-propagation / labels
// ---------------------------------------------------------------------------

#[test]
fn return_type_check_treats_set_equal_enum_vocabs_as_compatible() {
    // Two schemas declare the same vocabulary in different orders. A
    // body returning the second-shaped schema against a declared
    // first-shaped return type is set-equal — no D0080. Falsifies a
    // derived `PartialEq` on `Enum(Vec<String>)` (order-sensitive)
    // being used at the return-type comparison site.
    let src = "\
class In(Schema):
    status: enum[\"pending\", \"shipped\"]

class Out(Schema):
    status: enum[\"shipped\", \"pending\"]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"status\"))
";
    let result = check(src);
    assert!(
        !result.has_code("D0080"),
        "set-equal enum vocabularies must compare compatible at the \
         return-type check; got: {:?}",
        result
            .diagnostics_with_code("D0080")
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>(),
    );
}

#[test]
fn return_type_check_rejects_non_set_equal_enum_vocabs() {
    let src = "\
class In(Schema):
    status: enum[\"pending\", \"shipped\"]

class Out(Schema):
    status: enum[\"pending\", \"delivered\"]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"status\"))
";
    assert_has_code(&check(src), "D0080");
}

#[test]
fn arithmetic_on_enum_typed_column_fires_d0081_in_strict_mode() {
    // The exhaustiveness gap surfaced in round 1: `enum_col + 1` is
    // arithmetic on a string-shaped column, so D0081 should fire under
    // strict mode. The previous inline `matches!()` filter omitted
    // `Enum`, letting this silently pass. Routing through
    // `type_family()` (single source of truth) closes the gap.
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(\"x\", col(\"status\") + 1)
";
    let result = check_strict(src);
    assert_has_code(&result, "D0081");
}

#[test]
fn optional_wrapped_enum_with_duplicates_keeps_specific_d0011_message() {
    // Round-1 important: `Optional[enum["a", "a"]]` previously fell
    // through to a generic D0011, dropping the duplicate-value cite.
    // The parser must thread the enum-parse error through the Optional
    // wrapper so the dedicated message survives.
    let src = "\
class Order(Schema):
    status: Optional[enum[\"pending\", \"pending\"]]
";
    let result = check(src);
    assert_has_code(&result, "D0011");
    assert_message_contains(&result, "D0011", "pending");
    assert_message_contains(&result, "D0011", "unique");
}

#[test]
fn enum_with_non_string_literal_entry_fires_dedicated_d0011_message() {
    // M1 / round-1 minor: a numeric or bare-name entry inside
    // `enum[...]` previously collapsed to a generic D0011. The dedicated
    // message names the misuse so the user knows the fix is to quote
    // the entries.
    let src = "\
class Order(Schema):
    status: enum[\"a\", 5]
";
    let result = check(src);
    assert_has_code(&result, "D0011");
    assert_message_contains(&result, "D0011", "string literals");
}

#[test]
fn enum_with_bare_name_entry_fires_dedicated_d0011_message() {
    let src = "\
some_var = \"x\"

class Order(Schema):
    status: enum[some_var]
";
    let result = check(src);
    assert_has_code(&result, "D0011");
    assert_message_contains(&result, "D0011", "string literals");
}

#[test]
fn label_renders_atomic_and_composite_kinds_unchanged() {
    // Vacuity guard against `label()` being a no-op wrapper that just
    // forwards to `as_str` — atomics must come back as their bare name
    // (matches the round-1 hover/completion/symbols rendering).
    assert_eq!(ColumnType::Int.label(), "Int");
    assert_eq!(ColumnType::String.label(), "String");
    assert_eq!(ColumnType::Array(None).label(), "array");
    assert_eq!(ColumnType::Map(None, None).label(), "map");
    assert_eq!(
        ColumnType::Nullable(Box::new(ColumnType::Int)).label(),
        "Int",
    );
}

#[test]
fn label_renders_enum_inline_up_to_five_values_then_truncates() {
    let small = ColumnType::enum_from_values(vec!["a".into(), "b".into()]).unwrap();
    assert_eq!(small.label(), "enum[\"a\", \"b\"]");

    let exactly_five = ColumnType::enum_from_values(
        ["a", "b", "c", "d", "e"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    )
    .unwrap();
    assert_eq!(
        exactly_five.label(),
        "enum[\"a\", \"b\"\
              , \"c\", \"d\", \"e\"]",
    );

    let seven = ColumnType::enum_from_values((0..7).map(|i| format!("v{i}")).collect()).unwrap();
    assert_eq!(
        seven.label(),
        "enum[\"v0\", \"v1\", \"v2\", \"v3\", \"v4\", ... (2 more)]",
    );
}

#[test]
fn label_peels_nullable_around_enum_for_rendering() {
    let inner = ColumnType::enum_from_values(vec!["a".into()]).unwrap();
    let wrapped = ColumnType::Nullable(Box::new(inner));
    // The `Nullable` wrapper is peeled for the kind-label render — same
    // posture as `as_str`. The nullability shows up elsewhere (e.g. the
    // `Display` trail) but not in the bare kind label.
    assert_eq!(wrapped.label(), "enum[\"a\"]");
}

// ---------------------------------------------------------------------------
// PR-B round 2 — Q9 D0040 in branch forms
// ---------------------------------------------------------------------------

#[test]
fn d0040_fires_on_coalesce_with_non_set_equal_enum_branches() {
    // Two enum-typed columns with disjoint vocabs combined under
    // `coalesce` — spec Q9 mandates D0040 here, the same code the
    // `Merge[A, B]` shared-column enum-mismatch surface uses. The
    // silent-degrade-to-None path round 1 took is exactly the bug
    // class the spec defends against.
    //
    // Vacuity: reverting `report_branch_form_enum_conflicts` to a no-op
    // makes this test red because nothing else fires D0040 here.
    let src = "\
class Order(Schema):
    primary: enum[\"pending\", \"shipped\"]
    fallback: enum[\"refunded\", \"cancelled\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(
        \"staged\", coalesce(col(\"primary\"), col(\"fallback\"))
    )
";
    assert_has_code(&check(src), "D0040");
}

#[test]
fn d0040_silent_on_coalesce_with_set_equal_enum_branches() {
    // Set-equal vocabs (declaration order differs) reconcile cleanly
    // through `enum_vocab_eq` — the constraint flows through, D0040
    // stays silent.
    let src = "\
class Order(Schema):
    primary: enum[\"pending\", \"shipped\"]
    fallback: enum[\"shipped\", \"pending\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(
        \"staged\", coalesce(col(\"primary\"), col(\"fallback\"))
    )
";
    assert_does_not_have_code(&check(src), "D0040");
}

#[test]
fn d0040_silent_on_coalesce_with_mixed_enum_and_string_branches() {
    // Per spec Q9: a plain-string branch in the mix intentionally drops
    // the constraint to plain string. No D0040 — the user explicitly
    // mixed in a non-enum source, so the degradation is the documented
    // behavior, not a silent bug.
    let src = "\
class Order(Schema):
    primary: enum[\"pending\", \"shipped\"]
    fallback: string

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(
        \"staged\", coalesce(col(\"primary\"), col(\"fallback\"))
    )
";
    assert_does_not_have_code(&check(src), "D0040");
}

#[test]
fn d0040_fires_on_when_otherwise_with_non_set_equal_enum_branches() {
    // Same shape as the coalesce case, walked through the
    // `F.when(...).otherwise(...)` chain instead. Pins down that the
    // round-2 emitter walks the when-chain branch types, not just the
    // coalesce arg list.
    let src = "\
class Order(Schema):
    primary: enum[\"pending\", \"shipped\"]
    fallback: enum[\"refunded\", \"cancelled\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(
        \"staged\",
        when(col(\"primary\").isNotNull(), col(\"primary\")).otherwise(col(\"fallback\")),
    )
";
    assert_has_code(&check(src), "D0040");
}

#[test]
fn d0040_silent_on_when_otherwise_with_set_equal_enum_branches() {
    let src = "\
class Order(Schema):
    primary: enum[\"pending\", \"shipped\"]
    fallback: enum[\"shipped\", \"pending\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(
        \"staged\",
        when(col(\"primary\").isNotNull(), col(\"primary\")).otherwise(col(\"fallback\")),
    )
";
    assert_does_not_have_code(&check(src), "D0040");
}

// ---------------------------------------------------------------------------
// PR-B round 2 — Nullable[enum[...]] integration coverage
// ---------------------------------------------------------------------------

#[test]
fn d0084_nullable_enum_fires_on_compare_equality() {
    // Nullable[enum[...]] at the `==` check site — PR-A's Nullable
    // wrapper must peel cleanly so the comparison still sees the enum
    // vocabulary. Falsifiable: drop the Nullable peel and the literal
    // would compare against `None`, silencing D0084.
    let src = "\
class Order(Schema):
    status: Optional[enum[\"pending\", \"shipped\"]]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"status\") == \"delivered\")
";
    assert_has_code(&check(src), "D0084");
}

#[test]
fn d0084_nullable_enum_silent_when_compared_to_null_literal() {
    // Comparing a `Nullable[enum]` to `lit(None)` is a null comparison,
    // not a vocabulary lookup — D0084 stays silent. Pins down the
    // wrapper-peel doesn't fabricate a NULL-in-vocab check.
    let src = "\
class Order(Schema):
    status: Optional[enum[\"pending\", \"shipped\"]]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"status\") == lit(None))
";
    assert_does_not_have_code(&check(src), "D0084");
}

#[test]
fn d0084_nullable_enum_fires_on_with_column_lit() {
    let src = "\
class Order(Schema):
    status: Optional[enum[\"pending\", \"shipped\"]]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(\"status\", lit(\"delivered\"))
";
    assert_has_code(&check(src), "D0084");
}

#[test]
fn d0084_nullable_enum_fires_on_fillna_dict_value() {
    let src = "\
class Order(Schema):
    status: Optional[enum[\"pending\", \"shipped\"]]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.fillna({\"status\": \"delivered\"})
";
    assert_has_code(&check(src), "D0084");
}

#[test]
fn d0084_nullable_enum_fires_in_f_expr_equality() {
    let src = "\
class Order(Schema):
    status: Optional[enum[\"pending\", \"shipped\"]]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(F.expr(\"status = 'delivered'\"))
";
    assert_has_code(&check(src), "D0084");
}

// ---------------------------------------------------------------------------
// PR-B round 2 — F.expr repeated-literal span anchoring
// ---------------------------------------------------------------------------

#[test]
fn d0084_f_expr_repeated_off_vocab_literal_anchors_at_distinct_spans() {
    // Round-1 bug: `find_quoted_literal_offset` substring-search
    // attributed both `'X'` occurrences to the first match, producing
    // identical spans (or hiding one). Round-2 threads a per-occurrence
    // cursor past consumed matches so each diagnostic lands on its
    // own quote pair.
    //
    // Vacuity: revert the cursor (always start at 0) and both
    // diagnostics collapse to the same column — the assertion below
    // goes red.
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(F.expr(\"status = 'X' OR status = 'X'\"))
";
    let result = check(src);
    common::assert_count(&result, "D0084", 2);
    let columns: Vec<usize> = result
        .diagnostics_with_code("D0084")
        .iter()
        .map(|d| d.column)
        .collect();
    assert!(
        columns[0] != columns[1],
        "expected two D0084 spans at distinct columns, got both at {}: {:?}",
        columns[0],
        result
            .diagnostics_with_code("D0084")
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>(),
    );
}

// ---------------------------------------------------------------------------
// PR-B round 2 — nullif preserves first-arg enum constraint
// ---------------------------------------------------------------------------

#[test]
fn nullif_preserves_first_arg_enum_constraint_downstream() {
    // Per spec Q9a, `nullif(col_enum, x)` is idempotent over the
    // constraint: the result is `Nullable(enum[...])`. Vacuity-
    // falsifiable: revert the `nullif` arm in `function_result_type`
    // to `None` and the downstream `staged == 'off-vocab'` literal-
    // vs-vocab check no longer sees an enum sink — D0084 disappears.
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.withColumn(
        \"staged\", nullif(col(\"status\"), lit(\"pending\"))
    ).filter(col(\"staged\") == \"delivered\")
";
    let result = check(src);
    assert_has_code(&result, "D0084");
    assert_message_contains(&result, "D0084", "delivered");
}

// ---------------------------------------------------------------------------
// PR-B round 2 — F.expr NOT IN symmetry with IN
// ---------------------------------------------------------------------------

#[test]
fn d0084_f_expr_not_in_fires_on_off_vocab_literal() {
    // `NOT IN ('a', 'X')` with `'X'` off the enum vocabulary is the
    // same bug class as `IN`: the literal can never equal any vocab
    // value (the predicate is always true for vocab rows), so the user
    // almost certainly meant a different word. We mirror IN here —
    // D0084 fires on the off-vocab literal.
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(F.expr(\"status NOT IN ('pending', 'shippd')\"))
";
    let result = check(src);
    common::assert_count(&result, "D0084", 1);
    assert_message_contains(&result, "D0084", "shippd");
}
