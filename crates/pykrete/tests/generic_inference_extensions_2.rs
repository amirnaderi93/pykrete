//! Generic-inference extensions, round 2 — chained class-method calls
//! and non-`G[T]`-shaped generic methods.
//!
//! v0.1.15 PR A landed multi-TypeVar binding and nested-generic
//! parameters (see `generic_inference_extensions.rs`). This file
//! exercises the two remaining shapes:
//!
//! - **Chained class-method calls** — `dal.with_path("/x").read(SRC)`.
//!   An intermediate method whose return annotation is the class
//!   itself (`-> "DataAccessLayer"`, `-> DataAccessLayer`, `-> Self`)
//!   preserves the class identity through the chain, so the trailing
//!   `.read(SRC)` still routes through generic inference.
//! - **`type[T]`-shaped parameters** — `def cast_to[T](self, _:
//!   type[T]) -> SparkFrame[T]`. The argument is a class object whose
//!   static type is `type[Schema]`, so the matcher binds `T` from the
//!   arg's identifier rather than its runtime value.
//!
//! As in PR A, the driver function `f` carries a typed parameter to
//! keep it in the analysis pass.

#![allow(non_snake_case)] // DataFrame, DataSource, etc. appear in test names.

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Chained class-method calls
// ---------------------------------------------------------------------------

/// `dal.with_path("/x").read(SOURCE)` — `with_path` returns the class
/// itself (as a forward-reference string), so the outer `.read` still
/// dispatches through `DataAccessLayer.read[T]` and binds `T=Orders`.
/// Pin via a downstream `.select(col("..."))` against the bound schema.
#[test]
fn chained_builder_method_preserves_class_identity_for_generic_inference() {
    let happy = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataSource[T]:
    def __init__(self, path): pass

class DataAccessLayer:
    def with_path(self, path: str) -> "DataAccessLayer": ...
    def read[T](self, source: DataSource[T]) -> SparkFrame[T]: ...

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def f(x: SparkFrame[Anchor], dal: DataAccessLayer):
    dal.with_path("/x").read(ORDERS_SRC).select(col("order_id"))
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

class DataAccessLayer:
    def with_path(self, path: str) -> "DataAccessLayer": ...
    def read[T](self, source: DataSource[T]) -> SparkFrame[T]: ...

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def f(x: SparkFrame[Anchor], dal: DataAccessLayer):
    dal.with_path("/x").read(ORDERS_SRC).select(col("order_nope"))
"#,
    );
    assert_count(&bad, "D0030", 1);
    assert_message_contains(&bad, "D0030", "order_nope");
}

/// Two intermediate self-returning methods stacked back-to-back —
/// `dal.with_path("/x").with_options({...}).read(SOURCE)`. Each
/// `with_*` preserves the class; the final `.read` still routes
/// through generic inference and binds `T=Orders`.
#[test]
fn chained_builder_methods_chain_multiple_times() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataSource[T]:
    def __init__(self, path): pass

class DataAccessLayer:
    def with_path(self, path: str) -> "DataAccessLayer": ...
    def with_options(self, opts) -> "DataAccessLayer": ...
    def read[T](self, source: DataSource[T]) -> SparkFrame[T]: ...

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def f(x: SparkFrame[Anchor], dal: DataAccessLayer):
    dal.with_path("/x").with_options({"a": 1}).read(ORDERS_SRC).select(col("order_id"))
"#,
    );
    assert_count(&result, "D0030", 0);
}

/// A typo in an intermediate `with_*` call's args shouldn't break
/// schema flow on the outer `.read(...)`. The intermediate's return
/// annotation is still the class — pykrete preserves the chain even
/// when the body of that call would be invalid at runtime.
#[test]
fn chained_builder_typo_in_intermediate_does_not_poison_outer() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataSource[T]:
    def __init__(self, path): pass

class DataAccessLayer:
    def with_path(self, path: str) -> "DataAccessLayer": ...
    def read[T](self, source: DataSource[T]) -> SparkFrame[T]: ...

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

# The literal `123` is a wrong-typed path arg; pykrete doesn't
# diagnose it here, but the chain through `.with_path` must still
# preserve the class so `.read(ORDERS_SRC)` resolves and downstream
# column refs land cleanly.
def f(x: SparkFrame[Anchor], dal: DataAccessLayer):
    dal.with_path(123).read(ORDERS_SRC).select(col("order_id"), col("order_nope"))
"#,
    );
    // The good column is silent and the typo'd one fires — proves the
    // chain DID preserve the schema (otherwise the result would be
    // Unknown and BOTH refs would be silent on D0030).
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "order_nope");
}

/// `do_something` returns a *different* class than the receiver, so
/// the chain must NOT carry the original class through. The outer
/// `.some_method(SOURCE)` should degrade to Unknown rather than
/// misroute through `DataAccessLayer.some_method` (which would be a
/// false positive on the column reference below).
#[test]
fn chained_builder_returning_different_class_does_not_chain() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataSource[T]:
    def __init__(self, path): pass

class OtherLayer:
    pass

class DataAccessLayer:
    def do_something(self) -> OtherLayer: ...
    def some_method[T](self, source: DataSource[T]) -> SparkFrame[T]: ...

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def f(x: SparkFrame[Anchor], dal: DataAccessLayer):
    # `do_something` returns OtherLayer — chain must break here.
    # Downstream `.some_method(...)` (which only exists on DAL) and a
    # made-up column reference must NOT emit D0030 — the schema is
    # genuinely Unknown across the class boundary.
    dal.do_something().some_method(ORDERS_SRC).select(col("made_up"))
"#,
    );
    assert_count(&result, "D0030", 0);
}

/// Regression — the existing non-chained call `dal.read(SOURCE)`
/// (without any intermediate `with_*`) must still resolve to
/// `SparkFrame[Orders]`.
#[test]
fn non_chained_direct_call_still_works() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataSource[T]:
    def __init__(self, path): pass

class DataAccessLayer:
    def read[T](self, source: DataSource[T]) -> SparkFrame[T]: ...

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def f(x: SparkFrame[Anchor], dal: DataAccessLayer):
    dal.read(ORDERS_SRC).select(col("order_id"))
"#,
    );
    assert_count(&result, "D0030", 0);
}

// ---------------------------------------------------------------------------
// `type[T]`-shaped generic methods
// ---------------------------------------------------------------------------

/// `def cast_to[T](self, _: type[T]) -> SparkFrame[T]` called as
/// `dal.cast_to(Orders)` binds `T=Orders` from the arg's identifier
/// (its static type is `type[Orders]`). The result resolves to
/// `SparkFrame[Orders]`, so a real-column select lands cleanly.
#[test]
fn cast_to_typeof_T_binds_from_class_identifier() {
    let happy = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataAccessLayer:
    def cast_to[T](self, _: type[T]) -> SparkFrame[T]: ...

def f(x: SparkFrame[Anchor], dal: DataAccessLayer):
    dal.cast_to(Orders).select(col("order_id"))
"#,
    );
    assert_count(&happy, "D0030", 0);

    let bad = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataAccessLayer:
    def cast_to[T](self, _: type[T]) -> SparkFrame[T]: ...

def f(x: SparkFrame[Anchor], dal: DataAccessLayer):
    dal.cast_to(Orders).select(col("order_nope"))
"#,
    );
    assert_count(&bad, "D0030", 1);
    assert_message_contains(&bad, "D0030", "order_nope");
}

/// The arg `some_var` isn't a known schema — `T` stays unbound, the
/// result is Unknown, and a downstream typo emits 0 D0030 (no false
/// positive). The unknown-arg case must NOT silently bind T to
/// something else.
#[test]
fn cast_to_with_unknown_class_arg_degrades_gracefully() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class DataAccessLayer:
    def cast_to[T](self, _: type[T]) -> SparkFrame[T]: ...

def f(x: SparkFrame[Anchor], dal: DataAccessLayer, some_var):
    dal.cast_to(some_var).select(col("made_up"))
"#,
    );
    assert_count(&result, "D0030", 0);
}

/// Two `type[T]` slots called with *different* schema identifiers —
/// `def f[T](a: type[T], b: type[T])` invoked as `f(Orders, Products)`.
/// The matcher's `record_binding` poisons T on the conflict instead of
/// arbitrarily picking one schema, so the result degrades to Unknown
/// rather than silently fabricating either Orders or Products as the
/// answer. Both downstream `col("order_id")` and `col("sku")` end up
/// against an Unknown receiver — no D0030 for either, no false
/// positives either way.
///
/// This is defensive against future helper drift in `record_binding`:
/// if the refactor regressed to first-wins or last-wins, one of the
/// columns would mis-resolve and start firing D0030.
#[test]
fn type_T_sticky_poisoning_on_conflicting_bindings() {
    let result = check(
        r#"
class Orders(Schema):
    order_id: int

class Products(Schema):
    sku: string

class DataAccessLayer:
    def merge_into[T](self, a: type[T], b: type[T]) -> SparkFrame[T]: ...

def f(dal: DataAccessLayer):
    dal.merge_into(Orders, Products).select(col("order_id"), col("sku"))
"#,
    );
    // T is poisoned by the conflict; the result is Unknown. Neither
    // column ref fires — pinning the no-false-positive guarantee
    // `record_binding` is meant to provide. If T silently bound to
    // EITHER Orders or Products, one of these refs would fire D0030.
    assert_count(&result, "D0030", 0);
}

/// The arg is a string literal — not a class identifier. T stays
/// unbound, result is Unknown, downstream typo emits 0 D0030.
#[test]
fn cast_to_with_non_class_arg_degrades() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class DataAccessLayer:
    def cast_to[T](self, _: type[T]) -> SparkFrame[T]: ...

def f(x: SparkFrame[Anchor], dal: DataAccessLayer):
    dal.cast_to("a_string_literal").select(col("made_up"))
"#,
    );
    assert_count(&result, "D0030", 0);
}

/// Combines both extensions: `dal.with_path("/x").cast_to(Orders)`.
/// The `with_path` chain preserves the class; `cast_to(Orders)` binds
/// `T=Orders` from the class identifier. Downstream column refs
/// resolve cleanly.
#[test]
fn cast_to_combined_with_chained_builder() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataAccessLayer:
    def with_path(self, path: str) -> "DataAccessLayer": ...
    def cast_to[T](self, _: type[T]) -> SparkFrame[T]: ...

def f(x: SparkFrame[Anchor], dal: DataAccessLayer):
    dal.with_path("/x").cast_to(Orders).select(col("order_id"))
"#,
    );
    assert_count(&result, "D0030", 0);
}

/// Regression — the original `DataSource[T]`-shaped generic still
/// binds and substitutes correctly after the matcher gained the
/// `type[T]` branch.
#[test]
fn existing_generic_dispatch_still_works() {
    let result = check(
        r#"
class Anchor(Schema):
    a: int

class Orders(Schema):
    order_id: int

class DataSource[T]:
    def __init__(self, path): pass

class DataAccessLayer:
    def read[T](self, source: DataSource[T]) -> SparkFrame[T]: ...

ORDERS_SRC: DataSource[Orders] = DataSource("/o")

def f(x: SparkFrame[Anchor], dal: DataAccessLayer):
    dal.read(ORDERS_SRC).select(col("order_id"))
"#,
    );
    assert_count(&result, "D0030", 0);
}
