#!/usr/bin/env bash
# Unit tests for scripts/changelog-cite-check.sh.
# Run from repo root: bash scripts/changelog-cite-check.test.sh
# Each case writes a fixture CHANGELOG + a tiny REPO_ROOT tree,
# invokes the script with overrides, and asserts exit code / stderr.

set -u

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE="$SCRIPT_DIR/changelog-cite-check.sh"

if [ ! -f "$GATE" ]; then
    echo "FAIL: $GATE missing"
    exit 2
fi

pass=0
fail=0
declare -a failures=()

assert() {
    local name="$1"
    local expected_exit="$2"
    local actual_exit="$3"
    local extra_check_result="${4:-ok}"

    if [ "$actual_exit" = "$expected_exit" ] && [ "$extra_check_result" = "ok" ]; then
        pass=$((pass + 1))
        echo "PASS: $name"
    else
        fail=$((fail + 1))
        failures+=("$name (expected exit $expected_exit, got $actual_exit; extra=$extra_check_result)")
        echo "FAIL: $name (expected exit $expected_exit, got $actual_exit; extra=$extra_check_result)"
    fi
}

make_repo() {
    local tmp
    tmp=$(mktemp -d)
    mkdir -p "$tmp/crates/pykrete/src" "$tmp/scripts"
    echo "$tmp"
}

# --- Case 1: clean — real cite to existing file/line → pass ---
tmp=$(make_repo)
# 5 lines, cite at line 3
printf 'line1\nline2\nfn my_function() {}\nline4\nline5\n' > "$tmp/crates/pykrete/src/main.rs"
cat > "$tmp/CHANGELOG.md" <<'EOF'
# CHANGELOG
## [Unreleased]
- See `my_function` at `crates/pykrete/src/main.rs:3` for details.
EOF
CHANGELOG="$tmp/CHANGELOG.md" REPO_ROOT="$tmp" bash "$GATE" > "$tmp/stdout" 2> "$tmp/stderr"
rc=$?
extra=ok
grep -q "1 cite(s) verified" "$tmp/stdout" || extra=missing_summary
grep -q "0 failure(s)" "$tmp/stdout" || extra="${extra}+missing_failure_count"
assert "clean cite passes" 0 "$rc" "$extra"
rm -rf "$tmp"

# --- Case 2: file missing → exit 1 ---
tmp=$(make_repo)
cat > "$tmp/CHANGELOG.md" <<'EOF'
# CHANGELOG
## [Unreleased]
- See `crates/pykrete/src/nonexistent.rs:42` for details.
EOF
CHANGELOG="$tmp/CHANGELOG.md" REPO_ROOT="$tmp" bash "$GATE" > "$tmp/stdout" 2> "$tmp/stderr"
rc=$?
extra=ok
grep -qF "file not found" "$tmp/stderr" || extra=missing_msg
assert "missing file fails" 1 "$rc" "$extra"
rm -rf "$tmp"

# --- Case 3: line out of range → exit 1 ---
tmp=$(make_repo)
printf 'line1\nline2\nline3\n' > "$tmp/crates/pykrete/src/main.rs"
cat > "$tmp/CHANGELOG.md" <<'EOF'
# CHANGELOG
## [Unreleased]
- See `crates/pykrete/src/main.rs:99` for details.
EOF
CHANGELOG="$tmp/CHANGELOG.md" REPO_ROOT="$tmp" bash "$GATE" > "$tmp/stdout" 2> "$tmp/stderr"
rc=$?
extra=ok
grep -qF "line 99 out of range" "$tmp/stderr" || extra=missing_msg
assert "out-of-range line fails" 1 "$rc" "$extra"
rm -rf "$tmp"

# --- Case 4: historical section drift skipped → exit 0 ---
# A cite inside [1.9.0] (older than current [1.10.0]) is preserved-as-
# artifact and not verified.
tmp=$(make_repo)
printf 'line1\nline2\nline3\n' > "$tmp/crates/pykrete/src/main.rs"
cat > "$tmp/CHANGELOG.md" <<'EOF'
# CHANGELOG
## [Unreleased]
no cite here

## [1.10.0] - 2026-06-17
no cite here either

## [1.9.0] - 2026-06-16
- This historical cite at `crates/pykrete/src/gone.rs:42` is preserved.
EOF
CHANGELOG="$tmp/CHANGELOG.md" REPO_ROOT="$tmp" bash "$GATE" > "$tmp/stdout" 2> "$tmp/stderr"
rc=$?
extra=ok
grep -q "1 historical cite(s) skipped" "$tmp/stdout" || extra=missing_historical_count
assert "historical section drift skipped" 0 "$rc" "$extra"
rm -rf "$tmp"

# --- Case 5: anchor heuristic mismatch → exit 0 + warn ---
tmp=$(make_repo)
# Source has unrelated content; CHANGELOG sentence has identifier-shaped
# keywords that DON'T appear in the 5-line source window.
printf 'totally_unrelated_token_a\ntotally_unrelated_token_b\ntotally_unrelated_token_c\n' \
    > "$tmp/crates/pykrete/src/main.rs"
cat > "$tmp/CHANGELOG.md" <<'EOF'
# CHANGELOG
## [Unreleased]
- The `materialize_planner_widget` lives at `crates/pykrete/src/main.rs:2`.
EOF
CHANGELOG="$tmp/CHANGELOG.md" REPO_ROOT="$tmp" bash "$GATE" > "$tmp/stdout" 2> "$tmp/stderr"
rc=$?
extra=ok
grep -qF "anchor heuristic did not match" "$tmp/stderr" || extra=missing_warn
grep -q "0 failure(s)" "$tmp/stdout" || extra="${extra}+still_failed"
assert "anchor heuristic mismatch warns but does not fail" 0 "$rc" "$extra"
rm -rf "$tmp"

# --- Case 6: multiple citations on same line → both verified ---
tmp=$(make_repo)
printf 'l1\nl2\nl3\nl4\nl5\n' > "$tmp/crates/pykrete/src/main.rs"
printf 'a\nb\nc\nd\ne\n' > "$tmp/scripts/foo.sh"
cat > "$tmp/CHANGELOG.md" <<'EOF'
# CHANGELOG
## [Unreleased]
- See `crates/pykrete/src/main.rs:2` and `scripts/foo.sh:4` for details.
EOF
CHANGELOG="$tmp/CHANGELOG.md" REPO_ROOT="$tmp" bash "$GATE" > "$tmp/stdout" 2> "$tmp/stderr"
rc=$?
extra=ok
grep -q "2 cite(s) verified" "$tmp/stdout" || extra=missing_count
assert "two cites on same line both verified" 0 "$rc" "$extra"
rm -rf "$tmp"

# --- Case 7: bare basename uniquely resolves under crates/ → exit 0 ---
tmp=$(make_repo)
printf 'l1\nl2\nl3\nl4\nl5\n' > "$tmp/crates/pykrete/src/main.rs"
cat > "$tmp/CHANGELOG.md" <<'EOF'
# CHANGELOG
## [Unreleased]
- Quoted bare cite `main.rs:2` should resolve under crates/.
EOF
CHANGELOG="$tmp/CHANGELOG.md" REPO_ROOT="$tmp" bash "$GATE" > "$tmp/stdout" 2> "$tmp/stderr"
rc=$?
extra=ok
grep -q "1 cite(s) verified" "$tmp/stdout" || extra=missing_count
assert "bare basename uniquely resolves" 0 "$rc" "$extra"
rm -rf "$tmp"

# --- Case 8: non-citation file:line prose not parsed ---
# "see lines 12-15" or "next 20 lines" doesn't include .ext, so it's
# not interpreted as a citation.
tmp=$(make_repo)
cat > "$tmp/CHANGELOG.md" <<'EOF'
# CHANGELOG
## [Unreleased]
- See lines 12-15 for details. Refactored module:42 stuff. No cite here.
EOF
CHANGELOG="$tmp/CHANGELOG.md" REPO_ROOT="$tmp" bash "$GATE" > "$tmp/stdout" 2> "$tmp/stderr"
rc=$?
extra=ok
grep -q "0 cite(s) verified" "$tmp/stdout" || extra=parsed_non_cite
grep -q "0 failure(s)" "$tmp/stdout" || extra="${extra}+failed"
assert "non-citation file:line prose not parsed" 0 "$rc" "$extra"
rm -rf "$tmp"

echo
echo "Results: $pass passed, $fail failed"
if [ "$fail" -ne 0 ]; then
    echo "Failures:"
    for f in "${failures[@]}"; do
        echo "  - $f"
    done
    exit 1
fi
exit 0
