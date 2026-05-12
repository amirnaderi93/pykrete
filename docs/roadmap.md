# Roadmap

What's planned beyond v0.1, in rough priority order. This document is a living plan; it's updated as the project moves.

## After v0.1

Items the v0.1 spec defers, in the order we'll likely tackle them after the v0.1 release tag:

### Dotted column access in nested structs

`col("address.street")` should resolve through nested schemas. Today the dotted string is treated as a single flat field name, so it always fails `D0030` even when the path is valid. Fix: walk the segments — for non-final segments, look the field up, assert its type is a nested `Schema`, recurse with the remainder.

### Multi-file support

Cross-file `import` resolution. Today every analysis is single-file. Real codebases span dozens of files with schemas imported from a shared module. Need:

- Project root detection.
- Import resolution that follows the same Python rules as the runtime.
- Schema lookups that cross file boundaries.
- Incremental rechecking when one file changes.

### LSP / VS Code extension

Run dathon as a language server. Hover for inferred schemas, jump-to-definition for column references, diagnostics published into the editor's problem panel.

## Generic-inference extensions

Iteration 21 introduced generic-function inference for the simplest shape: a single type variable `T` appearing in both a parameter slot `GenericClass[T]` and a return slot `GenericClass[T]`. Real-world generic patterns often want more. Listed here so they're not forgotten:

### Multiple type parameters

```python
def join[A, B](left: DataFrame[A], right: DataFrame[B]) -> DataFrame[Joined[A, B]]: ...
```

Today: only one type variable per generic-method call is bound. Two-param methods don't infer at all.

### Nested generics

```python
def lift[T](xs: List[DataSource[T]]) -> List[DataFrame[T]]: ...
```

Today: dathon's matcher only handles one level of subscript (`G[T]`). Nested forms (`List[DataSource[T]]`) aren't recognized.

### Chained class-method calls

```python
return builder.with_path("/x").read[T](RAW_ORDERS)
```

Today: only direct method calls on a class-instance name are dispatched through the generic-inference path. A method call whose receiver is itself a call result (the `builder.with_path(...)` here) isn't treated as a class instance, so the outer `.read(...)` is skipped.

### Class-level constants

```python
class DataSources:
    RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")
```

Today: only **module-level** annotated constants are tracked in the constants registry. Class-attribute constants (the more common pattern in the user's real codebase) aren't yet picked up. Need to walk class bodies for `AnnAssign` and qualify the names (`DataSources.RAW_ORDERS`).

### Generic methods that aren't `[T] -> G[T]`-shaped

```python
def cast_to[T](self, _: type[T]) -> DataFrame[T]: ...
```

Methods that take a *type* as a value-level argument (e.g. `df.cast_to(RawOrders)`) need a different inference path — the type variable is bound from a value whose static type is `type[T]`, not from a `G[T]`-shaped slot.

## Quality-of-life items

- **Better error messages with hints** — "Did you mean 'X'?" suggestions on `D0030` via Levenshtein distance over the schema's field names.
- **`dathon.json` config file** — strictness modes, file/dir excludes, per-rule severity overrides.
- **`cargo install` packaging + Homebrew tap** — easier local install than `cargo run --`.
- **Performance pass** — benchmark on large codebases; explore parallel file checks once multi-file support lands.
