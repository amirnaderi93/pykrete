//! Per-file registry of `createOrReplaceTempView` registrations.
//!
//! Spark's `df.createOrReplaceTempView("name")` stashes `df` under a
//! global name that subsequent `spark.sql("SELECT … FROM name")` calls
//! can reference. pykrete mirrors that — when a chain ending in
//! `.createOrReplaceTempView(...)` resolves the receiver to a known
//! schema, the view name is mapped to that schema view here. Later
//! `spark.sql("…")` calls in the same file look the view name up to
//! check the query's column references and produce a result schema.
//!
//! Scope is **per-file by design**: a tempView created in file A is
//! invisible to file B. Spark's runtime view catalog *is* session-wide,
//! but pykrete deliberately doesn't follow that — cross-file resolution
//! would require global flow analysis (and would make analysis
//! order-dependent). Within-file is the common case in practice; the
//! cross-file case degrades silently (the FROM table name doesn't match
//! any registered view, so the query falls back to the existing
//! best-effort projection-column inference).
//!
//! Held in a [`RefCell`] so registrations can happen through the
//! otherwise-immutable `&BodyContext` that flows through `analyze_expr`.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::schema::SchemaView;

/// A per-file table of tempView name → schema. Constructed empty by the
/// file-analysis driver and shared by reference across the function-body
/// contexts in that file.
#[derive(Default)]
pub struct TempViewRegistry<'a> {
    views: RefCell<HashMap<String, SchemaView<'a>>>,
}

impl<'a> TempViewRegistry<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `view_name` against `schema`. A subsequent registration
    /// of the same name overwrites — matching Spark's "createOrReplace"
    /// semantics. Called only when the receiver schema is known
    /// (Declared or Derived); an Unknown receiver is a no-op at the
    /// call site, so it never reaches here.
    pub fn register(&self, view_name: &str, schema: SchemaView<'a>) {
        self.views
            .borrow_mut()
            .insert(view_name.to_string(), schema);
    }

    /// The schema registered for `view_name`, if any.
    pub fn lookup(&self, view_name: &str) -> Option<SchemaView<'a>> {
        self.views.borrow().get(view_name).cloned()
    }
}
