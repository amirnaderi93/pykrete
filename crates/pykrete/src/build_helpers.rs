//! Pure-logic helpers for `build.rs` — extracted so they can be
//! exercised by an integration test under `tests/build_helpers.rs`.
//!
//! `build.rs` itself is a build script; its inline `#[cfg(test)] mod
//! tests` does NOT run under `cargo test --workspace` (build scripts
//! are compiled standalone; their test modules are dead code from CI's
//! perspective). v1.9 PR-A1 review (R2): extract the pure logic here so
//! both the build script and the test harness consume the same source.
//!
//! `build.rs` pulls this module in via `#[path = "src/build_helpers.rs"]
//! mod build_helpers;` at build time. `lib.rs` re-exports it as
//! `#[doc(hidden)] pub mod build_helpers;` so integration tests can
//! reach it. No I/O, no `std::env`, no `OUT_DIR` references live here —
//! the build-script wiring stays in `build.rs`.

pub const GATE: &str = "receiver_is_pandas_inherited";

/// Replace every `// ...\n` line comment with spaces (preserving line
/// breaks so error byte offsets stay roughly meaningful). String
/// literals are honored — `"// not a comment"` stays intact. Block
/// comments `/* ... */` are NOT handled (Rust's `///` doc comments are
/// line comments, and `expr.rs` has no block comments). Without this,
/// the build.rs would extract method names quoted INSIDE doc comments
/// describing the arm shape — false positives.
pub fn strip_line_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut in_str = false;
    let mut esc = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            out.push(c as char);
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
            out.push('"');
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

/// Walk the source looking for `receiver_is_pandas_inherited` occurrences.
/// For each match, scan forward through the AND-conjoined predicate
/// (across newlines) and extract every quoted method-name from either a
/// `matches!(method, "X" | "Y" | ...)` arm or a `method == "X"` arm.
///
/// Hand-rolled rather than regex'd to avoid pulling `regex` into
/// pykrete's build closure. The patterns are simple enough — both
/// shapes are deliberately constrained in `expr.rs` per the tripwire
/// comment there.
pub fn extract_methods(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while let Some(rel) = find_from(source, i, GATE) {
        let after = rel + GATE.len();
        let mut j = after;
        let body_start = scan_to_body_or_semi(bytes, j);
        let region = &source[j..body_start];
        j = body_start;

        let mut found_any = false;

        if let Some(m_start) = region.find("matches!") {
            let after_macro = m_start + "matches!".len();
            if let Some(paren_open_rel) = region[after_macro..].find('(') {
                let paren_open_abs = after_macro + paren_open_rel + 1;
                if let Some(paren_close_abs) = find_matching_paren(region, paren_open_abs) {
                    let inner = &region[paren_open_abs..paren_close_abs];
                    for lit in extract_string_literals(inner) {
                        out.push(lit);
                        found_any = true;
                    }
                }
            }
        }

        if let Some(eq_idx) = find_method_eq(region) {
            let rest = &region[eq_idx..];
            if let Some(lit) = extract_first_string_literal(rest) {
                out.push(lit);
                found_any = true;
            }
        }

        // Defensive: if we found NEITHER shape in the predicate region,
        // it likely means a new shape was introduced. Fail the build
        // loudly rather than silently dropping the arm. Skip the
        // `let receiver_is_pandas_inherited = ...` binding line — the
        // skip clause checks the bytes immediately PRECEDING the gate
        // for a word-bounded `let` so a future arm with an unrelated
        // `=` in its predicate (e.g. `&& let Some(kw) = ...`) still
        // trips the panic. v1.9 PR-A1 tightening (arch-I1).
        if !found_any && !preceded_by_let(source, rel) {
            panic!(
                "build.rs: found `{GATE}` at byte offset {rel} but neither \
                 `matches!(method, ...)` nor `method == \"...\"` followed. \
                 Update build.rs's extract_methods OR adjust the new arm \
                 shape in operations/expr.rs to match an existing pattern. \
                 Region: {:?}",
                region
            );
        }

        i = j.max(rel + 1);
    }
    out
}

pub fn find_from(haystack: &str, start: usize, needle: &str) -> Option<usize> {
    haystack[start..].find(needle).map(|r| start + r)
}

/// True when the bytes immediately preceding `at` in `source` are the
/// keyword `let` at a word boundary — i.e. trailing whitespace is
/// skipped, then `let` is matched only if the byte BEFORE it is start-
/// of-file, whitespace, or one of `{`/`}`/`;`. The word-boundary check
/// blocks substring matches like `inlet receiver_is_pandas_inherited`
/// from silently bypassing the panic in `extract_methods`.
pub fn preceded_by_let(source: &str, at: usize) -> bool {
    let prefix = &source[..at];
    let trimmed = prefix.trim_end();
    if !trimmed.ends_with("let") {
        return false;
    }
    let before_let = &trimmed[..trimmed.len() - "let".len()];
    match before_let.chars().last() {
        None => true,
        Some(c) => c.is_whitespace() || matches!(c, '{' | '}' | ';'),
    }
}

/// Advance from `i` to the byte index of the next `{` (arm body open) or
/// `;` (statement terminator) at the same brace-depth — whichever comes
/// first. Used to bound the predicate-region scan so we don't pull
/// literals out of the arm body itself.
pub fn scan_to_body_or_semi(bytes: &[u8], start: usize) -> usize {
    let mut depth_paren: i32 = 0;
    let mut depth_bracket: i32 = 0;
    let mut in_str = false;
    let mut esc = false;
    let mut i = start;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'(' => depth_paren += 1,
            b')' => depth_paren -= 1,
            b'[' => depth_bracket += 1,
            b']' => depth_bracket -= 1,
            b'{' | b';' if depth_paren == 0 && depth_bracket == 0 => return i,
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// Find the index of the `)` that closes the `(` at `open` (which is the
/// position JUST AFTER the open paren — i.e. of the first byte inside).
/// Returns the position of the matching `)` (i.e. one past the inside).
pub fn find_matching_paren(s: &str, open_after: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 1;
    let mut in_str = false;
    let mut esc = false;
    let mut i = open_after;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Scan `s` and collect every double-quoted string literal inside it.
/// Used for the `matches!` arm list.
pub fn extract_string_literals(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let mut j = i + 1;
            let mut buf = String::new();
            let mut esc = false;
            while j < bytes.len() {
                let c = bytes[j];
                if esc {
                    buf.push(c as char);
                    esc = false;
                } else if c == b'\\' {
                    esc = true;
                } else if c == b'"' {
                    break;
                } else {
                    buf.push(c as char);
                }
                j += 1;
            }
            out.push(buf);
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

pub fn extract_first_string_literal(s: &str) -> Option<String> {
    extract_string_literals(s).into_iter().next()
}

/// Find the byte index where `method` `==` `"..."` starts inside the
/// region. Tolerant of whitespace/newlines between the three tokens.
pub fn find_method_eq(region: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = region[from..].find("method") {
        let abs = from + rel;
        let after = abs + "method".len();
        let rest = region[after..].trim_start();
        if rest.starts_with("==") {
            let eq_end = after + (region.len() - after - rest.len()) + 2;
            let after_eq = region[eq_end..].trim_start();
            if after_eq.starts_with('"') {
                return Some(abs);
            }
        }
        from = abs + 1;
    }
    None
}
