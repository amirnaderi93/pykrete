//! Generic-inference extensions — multi-TypeVar binding and
//! nested-generic parameter shapes.
//!
//! v0.1.14 only inferred `def m[T](x: G[T]) -> G[T]`-shaped methods:
//! one TypeVar, one outer subscript on each side. This file exercises
//! the two extensions added in v0.1.15:
//!
//! - **Multiple type parameters** — `def f[A, B](x: G[A], y: G[B]) -> R[A,B]`.
//!   Each parameter slot binds its own TypeVar independently; the return is
//!   substituted per-TypeVar. An incompatible re-binding of the same
//!   TypeVar degrades to Unknown rather than fabricates a join.
//! - **Nested generic parameters** — `List[G[T]]`, `Optional[G[T]]`,
//!   `Dict[str, G[T]]`, `List[List[G[T]]]`. The matcher strips collection
//!   wrappers on the parameter side, walks into the matching argument
//!   literal, and binds T from the element schemas.
//!
//! Tests give the driver function `f` a typed parameter
//! (`x: DataFrame[Anchor]`) — pykrete only walks the body of functions
//! with at least one DataFrame-typed slot, so the parameter exists only
//! to keep `f` in the analysis pass. The body never references `x`.

#![allow(non_snake_case)] // DataFrame, DataSource, etc. appear in test names.

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Multiple type parameters
// ---------------------------------------------------------------------------

/// `def f[A, B](x: DataSource[A], y: DataSource[B]) -> Merge[A, B]` binds
/// A from x's source-schema, B from y's, and the return resolves to the
/// concatenated columns of both. We verify by chaining a `.select` that
/// references one column from each side — both must resolve, and a typo
/// on either side must surface as D0030.
#[test]
fn multi_typevar_binds_each_param_independently() {
    let preamble = r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int
    price: int

class Refunds(Schema):
    refund_id: int
    amount: int

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")
REFUNDS_SRC: DataSource[Refunds] = DataSource("/r")

def join_pair[A, B](x: DataSource[A], y: DataSource[B]) -> DataFrame[Merge[A, B]]:
    pass
"#;
    // Happy path — one column from each side. No D0030.
    let happy = check(&format!(
        r#"{preamble}
def f(x: DataFrame[Anchor]):
    result = join_pair(ORDERS_SRC, REFUNDS_SRC)
    result.select(col("order_id"), col("refund_id"))
"#
    ));
    assert_count(&happy, "D0030", 0);

    // Typo surfaces — both schemas reached the result, so the typo isn't
    // in either, and exactly one D0030 fires.
    let bad = check(&format!(
        r#"{preamble}
def f(x: DataFrame[Anchor]):
    join_pair(ORDERS_SRC, REFUNDS_SRC).select(col("nope"))
"#
    ));
    assert_count(&bad, "D0030", 1);
    assert_message_contains(&bad, "D0030", "nope");
}

/// Both args carry the same schema. A and B both bind Orders; the
/// concatenated result is just Orders' columns once (Merge de-dupes).
#[test]
fn multi_typevar_with_same_value_resolves_consistently() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int
    price: int

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def pair[A, B](x: DataSource[A], y: DataSource[B]) -> DataFrame[Merge[A, B]]:
    pass

def f(x: DataFrame[Anchor]):
    pair(ORDERS_SRC, ORDERS_SRC).select(col("order_id"), col("price"))
"#,
    );
    assert_count(&result, "D0030", 0);
}

/// A typo on a downstream-only column doesn't poison the binding of
/// either TypeVar — real-column references on both sides still resolve.
/// Exactly one D0030 (the typo) fires.
#[test]
fn multi_typevar_typo_in_call_does_not_misbind() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class Refunds(Schema):
    refund_id: int

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")
REFUNDS_SRC: DataSource[Refunds] = DataSource("/r")

def pair[A, B](x: DataSource[A], y: DataSource[B]) -> DataFrame[Merge[A, B]]:
    pass

def f(x: DataFrame[Anchor]):
    pair(ORDERS_SRC, REFUNDS_SRC).select(
        col("order_id"),
        col("refund_id"),
        col("nope"),
    )
"#,
    );
    // If A or B had failed to bind, both real-column references would
    // also miss and we'd see 3 diagnostics. Exactly one means both
    // bindings landed cleanly.
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

/// If one argument's schema can't be inferred (an untyped local), the
/// other TypeVar should still bind. Tested via a real-column reference
/// on the resolvable side: no D0030 means B bound to Refunds.
#[test]
fn multi_typevar_one_arg_unknown_other_resolves() {
    let happy = check(
        r#"
class Anchor(Schema):
    a: int

class Refunds(Schema):
    refund_id: int

class DataSource[T]:
    def __init__(self, path): pass

REFUNDS_SRC: DataSource[Refunds] = DataSource("/r")

def pair[A, B](x: DataSource[A], y: DataSource[B]) -> DataFrame[Merge[A, B]]:
    pass

def f(x: DataFrame[Anchor], opaque):
    pair(opaque, REFUNDS_SRC).select(col("refund_id"))
"#,
    );
    assert_count(&happy, "D0030", 0);

    let bad = check(
        r#"
class Anchor(Schema):
    a: int

class Refunds(Schema):
    refund_id: int

class DataSource[T]:
    def __init__(self, path): pass

REFUNDS_SRC: DataSource[Refunds] = DataSource("/r")

def pair[A, B](x: DataSource[A], y: DataSource[B]) -> DataFrame[Merge[A, B]]:
    pass

def f(x: DataFrame[Anchor], opaque):
    pair(opaque, REFUNDS_SRC).select(col("refund_nope"))
"#,
    );
    assert_count(&bad, "D0030", 1);
    assert_message_contains(&bad, "D0030", "refund_nope");
}

/// The substituted result must reflect every TypeVar's binding — every
/// column declared on both schemas should be present. The last column
/// on the second schema (`issued_at`) is the diagnostic-rich proof: a
/// truncated substitution would miss it.
#[test]
fn multi_typevar_result_substitution() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int
    price: int

class Refunds(Schema):
    refund_id: int
    amount: int
    issued_at: string

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")
REFUNDS_SRC: DataSource[Refunds] = DataSource("/r")

def pair[A, B](x: DataSource[A], y: DataSource[B]) -> DataFrame[Merge[A, B]]:
    pass

def f(x: DataFrame[Anchor]):
    pair(ORDERS_SRC, REFUNDS_SRC).select(
        col("order_id"),
        col("price"),
        col("refund_id"),
        col("amount"),
        col("issued_at"),
    )
"#,
    );
    assert_count(&result, "D0030", 0);
}

// ---------------------------------------------------------------------------
// Nested generics — multiple subscript levels
// ---------------------------------------------------------------------------

/// `def f[T](src: List[DataSource[T]]) -> DataFrame[T]` — the matcher
/// strips the outer `List` and binds T from each element's
/// `DataSource[T]`. A single-element list of a typed constant works;
/// downstream column references resolve against the bound schema.
#[test]
fn nested_generic_list_of_datasource_binds_T() {
    let happy = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def merge_sources[T](sources: List[DataSource[T]]) -> DataFrame[T]:
    pass

def f(x: DataFrame[Anchor]):
    merge_sources([ORDERS_SRC]).select(col("order_id"))
"#,
    );
    assert_count(&happy, "D0030", 0);

    let bad = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def merge_sources[T](sources: List[DataSource[T]]) -> DataFrame[T]:
    pass

def f(x: DataFrame[Anchor]):
    merge_sources([ORDERS_SRC]).select(col("order_nope"))
"#,
    );
    assert_count(&bad, "D0030", 1);
    assert_message_contains(&bad, "D0030", "order_nope");
}

/// `Optional[DataSource[T]]` — strip Optional, bind T from the arg's
/// own subscript shape. Tested with a real value (the `None` form is a
/// degenerate edge case that simply binds nothing).
#[test]
fn nested_generic_optional_datasource_binds_T() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def maybe_load[T](src: Optional[DataSource[T]]) -> DataFrame[T]:
    pass

def f(x: DataFrame[Anchor]):
    maybe_load(ORDERS_SRC).select(col("order_id"))
"#,
    );
    assert_count(&result, "D0030", 0);
}

/// `Dict[str, DataSource[T]]` — the matcher recurses into the dict's
/// values (key annotation is ignored). All values must agree on T.
#[test]
fn nested_generic_dict_of_datasource_binds_T() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int
    price: int

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def from_named[T](sources: Dict[str, DataSource[T]]) -> DataFrame[T]:
    pass

def f(x: DataFrame[Anchor]):
    from_named({"a": ORDERS_SRC, "b": ORDERS_SRC}).select(
        col("order_id"),
        col("price"),
    )
"#,
    );
    assert_count(&result, "D0030", 0);
}

/// `List[List[DataSource[T]]]` — three levels deep. The matcher must
/// strip both list layers before reaching the `G[T]` base case.
#[test]
fn nested_generic_two_level_nesting() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def deep[T](sources: List[List[DataSource[T]]]) -> DataFrame[T]:
    pass

def f(x: DataFrame[Anchor]):
    deep([[ORDERS_SRC]]).select(col("order_id"))
"#,
    );
    assert_count(&result, "D0030", 0);
}

/// Incompatible T across list elements must degrade to Unknown rather
/// than pick one. With degradation, a downstream typo on the result
/// emits no D0030 (we don't know the schema). If a schema had been
/// wrongly picked, the made-up column would fire.
#[test]
fn nested_generic_incompatible_T_degrades_to_unknown() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class Refunds(Schema):
    refund_id: int

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")
REFUNDS_SRC: DataSource[Refunds] = DataSource("/r")

def merge_sources[T](sources: List[DataSource[T]]) -> DataFrame[T]:
    pass

def f(x: DataFrame[Anchor]):
    # ORDERS_SRC binds T=Orders; REFUNDS_SRC tries to bind T=Refunds —
    # conflict. T degrades to Unknown.
    merge_sources([ORDERS_SRC, REFUNDS_SRC]).select(col("made_up"))
"#,
    );
    assert_count(&result, "D0030", 0);
}

/// Three-element conflict via list nesting must STAY Unknown across
/// further elements. A buggy "remove on conflict" without poisoning lets
/// element 3 silently re-bind T to ORDERS_SRC's schema, after which a
/// downstream typo would fire a false D0030. With sticky poisoning, T
/// stays Unknown for the whole call and a made-up column emits 0 D0030.
#[test]
fn nested_generic_3plus_elements_with_revival_stays_unknown() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class Refunds(Schema):
    refund_id: int

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")
REFUNDS_SRC: DataSource[Refunds] = DataSource("/r")

def merge_sources[T](srcs: List[DataSource[T]]) -> DataFrame[T]:
    pass

def f(x: DataFrame[Anchor]):
    # Element 1 binds T=Orders. Element 2 conflicts and poisons T.
    # Element 3 must NOT revive T=Orders — sticky poisoning required.
    result = merge_sources([ORDERS_SRC, REFUNDS_SRC, ORDERS_SRC])
    result.select(col("made_up_only_on_orders"))
"#,
    );
    assert_count(&result, "D0030", 0);
}

/// Same revival hazard, but across three positional parameter slots
/// instead of three list elements: `def g[T](x: G[T], y: G[T], z: G[T])`.
/// Param 2 conflicts with param 1 and must poison T; param 3 must NOT
/// revive a binding. With sticky poisoning, T stays Unknown and a
/// downstream typo emits 0 D0030.
#[test]
fn multi_typevar_3plus_param_slots_with_revival_stays_unknown() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class Refunds(Schema):
    refund_id: int

class DataSource[T]:
    def __init__(self, path): pass

ORDERS_SRC: DataSource[Orders] = DataSource("/o")
REFUNDS_SRC: DataSource[Refunds] = DataSource("/r")

def merge_three[T](x: DataSource[T], y: DataSource[T], z: DataSource[T]) -> DataFrame[T]:
    pass

def f(x: DataFrame[Anchor]):
    result = merge_three(ORDERS_SRC, REFUNDS_SRC, ORDERS_SRC)
    result.select(col("any_name"))
"#,
    );
    assert_count(&result, "D0030", 0);
}
