#!/usr/bin/env bash
# Unit tests for scripts/changelog-grep.sh.
# Run from repo root: bash scripts/changelog-grep.test.sh
# Each case writes a fixture CHANGELOG + fake source tree, invokes the
# real script with CHANGELOG/SRC_DIR overrides, and asserts exit code +
# (where relevant) stdout/stderr content.

set -u

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE="$SCRIPT_DIR/changelog-grep.sh"

if [ ! -x "$GATE" ]; then
    echo "FAIL: $GATE missing or not executable"
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

run_case() {
    local changelog_content="$1"
    local src_content="$2"
    local tmp
    tmp=$(mktemp -d)
    printf '%s' "$changelog_content" > "$tmp/CHANGELOG.md"
    mkdir -p "$tmp/src"
    printf '%s' "$src_content" > "$tmp/src/main.rs"
    CHANGELOG="$tmp/CHANGELOG.md" SRC_DIR="$tmp/src" bash "$GATE" > "$tmp/stdout" 2> "$tmp/stderr"
    local rc=$?
    LAST_TMP="$tmp"
    LAST_RC=$rc
    return 0
}

# --- Case 1: fenced stderr block with content present in source → PASS ---
run_case '# CHANGELOG
A line of prose.

```stderr
pykrete: migrate default is now --check; pass --apply to rewrite in place (v1.7+)
```
' 'fn main() {
    eprintln!(
        "pykrete: migrate default is now --check; pass --apply to rewrite in place (v1.7+)"
    );
}
'
extra=ok
grep -q "checked 1 block(s) across 1 source-anchored line(s)" "$LAST_TMP/stdout" || extra=missing_summary
assert "stderr block with present content passes" 0 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 2: fenced stderr block with content NOT in source → FAIL with clear message ---
run_case '# CHANGELOG
```stderr
v1.7+: pykrete migrate is dry-run by default. Use --apply to write changes.
```
' 'fn main() {
    eprintln!("totally different string");
}
'
extra=ok
grep -qF "MISMATCH: CHANGELOG.md fenced-block 'stderr'" "$LAST_TMP/stderr" || extra=missing_mismatch_msg
grep -q "v1.7+: pykrete migrate is dry-run by default" "$LAST_TMP/stderr" || extra="${extra}+missing_excerpt"
assert "stderr block with absent content fails with clear message" 1 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 3: unlabeled fenced block → IGNORED (not in allow-list) ---
run_case '# CHANGELOG
```
this line does not exist anywhere in source
```
' 'fn main() {
    eprintln!("hello");
}
'
extra=ok
grep -q "no fenced stderr/stdout/text/text-numeric blocks" "$LAST_TMP/stdout" || extra=should_be_no_op
assert "unlabeled fenced block is ignored" 0 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 4: inline backtick string → IGNORED (only fenced blocks) ---
run_case '# CHANGELOG
A `nonexistent inline string never in source` quoted inline.
' 'fn main() {}
'
extra=ok
grep -q "no fenced stderr/stdout/text/text-numeric blocks" "$LAST_TMP/stdout" || extra=should_be_no_op
assert "inline backtick string is ignored" 0 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 5: multi-line fenced block — all lines checked ---
run_case '# CHANGELOG
```stdout
line one of output
line two of output
line three of output
```
' 'fn main() {
    println!("line one of output");
    println!("line two of output");
    println!("line three of output");
}
'
extra=ok
grep -q "checked 1 block(s) across 3 source-anchored line(s)" "$LAST_TMP/stdout" || extra=wrong_line_count
assert "multi-line block checks each line" 0 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 5b: multi-line fenced block with ONE missing line → FAIL ---
run_case '# CHANGELOG
```stdout
line one of output
line two of output
this line is missing from source
```
' 'fn main() {
    println!("line one of output");
    println!("line two of output");
}
'
extra=ok
grep -q "this line is missing from source" "$LAST_TMP/stderr" || extra=missed_the_missing_line
assert "multi-line block fails on first missing line" 1 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 6: empty fenced block → no false alarm ---
run_case '# CHANGELOG
```stderr
```
' 'fn main() {}
'
extra=ok
grep -q "checked 0 block(s)" "$LAST_TMP/stdout" || grep -q "no fenced stderr/stdout/text/text-numeric blocks" "$LAST_TMP/stdout" || extra=unexpected_summary
assert "empty fenced block produces no false alarm" 0 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 7: fenced block labeled `python` (not allow-listed) → IGNORED ---
run_case '# CHANGELOG
```python
def code_snippet_not_binary_output():
    pass
```
' 'fn main() {}
'
extra=ok
grep -q "no fenced stderr/stdout/text/text-numeric blocks" "$LAST_TMP/stdout" || extra=python_label_not_ignored
assert "python-labeled fenced block is ignored" 0 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 8: fenced text block with content present → PASS ---
run_case '# CHANGELOG
```text
specific output token
```
' 'fn render() { print!("specific output token"); }
'
assert "text-labeled fenced block honored" 0 "$LAST_RC"
rm -rf "$LAST_TMP"

# --- Case 9: multiple fenced blocks, mixed labels, mixed pass/fail ---
run_case '# CHANGELOG
```stderr
present-stderr-line
```
prose between blocks
```stdout
absent-stdout-line
```
' 'fn main() { eprintln!("present-stderr-line"); }
'
extra=ok
grep -q "absent-stdout-line" "$LAST_TMP/stderr" || extra=missed_failure
assert "mixed blocks fail on the bad one only" 1 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 10: missing CHANGELOG → usage error exit 2 ---
CHANGELOG="/tmp/nonexistent-$$-changelog.md" SRC_DIR="$SCRIPT_DIR" bash "$GATE" > /tmp/stdout.$$ 2> /tmp/stderr.$$
rc=$?
extra=ok
grep -q "CHANGELOG file not found" /tmp/stderr.$$ || extra=missing_error_msg
assert "missing CHANGELOG returns exit 2" 2 "$rc" "$extra"
rm -f /tmp/stdout.$$ /tmp/stderr.$$

# --- Case 11: missing SRC_DIR → usage error exit 2 ---
tmp=$(mktemp -d)
printf 'no fences here\n' > "$tmp/CHANGELOG.md"
CHANGELOG="$tmp/CHANGELOG.md" SRC_DIR="/tmp/nonexistent-$$-src" bash "$GATE" > /tmp/stdout.$$ 2> /tmp/stderr.$$
rc=$?
extra=ok
grep -q "source directory not found" /tmp/stderr.$$ || extra=missing_error_msg
assert "missing SRC_DIR returns exit 2" 2 "$rc" "$extra"
rm -f /tmp/stdout.$$ /tmp/stderr.$$
rm -rf "$tmp"

# --- Case 12 (self-verify regression): v1.7 PR-G drift class — fenced-block variant ---
# v1.7 PR-F's CHANGELOG quoted a stderr that didn't exist in main.rs:660. The
# actual v1.7 drift used INLINE backticks at CHANGELOG L24 (which this gate
# does NOT cover by design — see CONTRIBUTING.md "CHANGELOG conventions").
# This case emulates the same drift CLASS normalized to the fenced-block shape
# this gate covers: future CHANGELOG editors who follow the fenced-block
# convention get drift caught; editors who use inline backticks do not.
run_case '# CHANGELOG
```stderr
pykrete: migrate default is now --check; pass --apply to rewrite in place (v1.7+)
```
' 'fn migrate_warn() {
    eprintln!("v1.7+: pykrete migrate is dry-run by default. Use --apply to write changes.");
}
'
extra=ok
grep -q "MISMATCH" "$LAST_TMP/stderr" || extra=missed_drift
grep -q "v1.7 retro rule 6" "$LAST_TMP/stderr" || extra="${extra}+missing_rule_pointer"
assert "v1.7 PR-G drift class (fenced-block variant) is caught" 1 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# v1.9 PR-A2 — text-numeric block cases.
# Each case stubs the numeric_claim_command via PYKRETE_NUMERIC_CLAIM_TABLE_OVERRIDE
# so the tests stay deterministic without depending on a pykrete-tests
# checkout being present beside this worktree.

run_numeric_case() {
    local changelog_content="$1"
    local override="$2"
    local tmp
    tmp=$(mktemp -d)
    printf '%s' "$changelog_content" > "$tmp/CHANGELOG.md"
    mkdir -p "$tmp/src"
    printf 'fn main() {}\n' > "$tmp/src/main.rs"
    printf '%s\n' "$override" > "$tmp/override.sh"
    CHANGELOG="$tmp/CHANGELOG.md" SRC_DIR="$tmp/src" \
        PYKRETE_NUMERIC_CLAIM_TABLE_OVERRIDE="$tmp/override.sh" \
        bash "$GATE" > "$tmp/stdout" 2> "$tmp/stderr"
    local rc=$?
    LAST_TMP="$tmp"
    LAST_RC=$rc
    return 0
}

# --- Case 13: text-numeric block with correct probe count → PASS ---
run_numeric_case '# CHANGELOG
```text-numeric
253 probes
```
' 'numeric_claim_command() {
    case "$1" in
        probes) echo "echo 253" ;;
        *) return 1 ;;
    esac
}'
extra=ok
grep -q "checked 1 block(s)" "$LAST_TMP/stdout" || extra=missing_summary
grep -q "1 text-numeric claim" "$LAST_TMP/stdout" || extra="${extra}+missing_numeric_count"
assert "text-numeric block with correct count passes" 0 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 14: text-numeric block with WRONG fixture count → FAIL with clear MISMATCH ---
# Emulates the v1.8 PR-F blocker: CHANGELOG L30 claimed "106 fixtures" but
# live was 112. The v2 gate catches this structurally.
run_numeric_case '# CHANGELOG
```text-numeric
106 fixtures
```
' 'numeric_claim_command() {
    case "$1" in
        fixtures) echo "echo 112" ;;
        *) return 1 ;;
    esac
}'
extra=ok
grep -qF "MISMATCH: text-numeric 'fixtures' expected 106 but live was 112" "$LAST_TMP/stderr" || extra=missing_mismatch_msg
assert "text-numeric block with wrong fixture count fails (v1.8 PR-F blocker emulation)" 1 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 15: text-numeric block with UNKNOWN key → FAIL with clear message ---
run_numeric_case '# CHANGELOG
```text-numeric
999 unknownthing
```
' 'numeric_claim_command() {
    case "$1" in
        probes) echo "echo 253" ;;
        *) return 1 ;;
    esac
}'
extra=ok
grep -qF "unknown numeric-claim key: 'unknownthing'" "$LAST_TMP/stderr" || extra=missing_unknown_msg
assert "text-numeric block with unknown key fails clearly" 1 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 16: text-numeric block with MALFORMED line (no number) → FAIL ---
run_numeric_case '# CHANGELOG
```text-numeric
notanumber probes
```
' 'numeric_claim_command() {
    case "$1" in
        probes) echo "echo 253" ;;
        *) return 1 ;;
    esac
}'
extra=ok
grep -q "is not a positive integer" "$LAST_TMP/stderr" || extra=missing_malformed_msg
assert "text-numeric block with malformed line fails clearly" 1 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

# --- Case 17: --skip-live-extract honored — unknown key still fails, but valid keys don't run commands ---
tmp=$(mktemp -d)
printf '%s' '# CHANGELOG
```text-numeric
999 probes
```
' > "$tmp/CHANGELOG.md"
mkdir -p "$tmp/src"
printf 'fn main() {}\n' > "$tmp/src/main.rs"
# No override file: default table is used. probes command would fail in
# this temp dir (no ../pykrete-tests). --skip-live-extract bypasses the
# run, so the gate passes despite the wrong number.
CHANGELOG="$tmp/CHANGELOG.md" SRC_DIR="$tmp/src" \
    bash "$GATE" --skip-live-extract > "$tmp/stdout" 2> "$tmp/stderr"
rc=$?
extra=ok
grep -q "live extract SKIPPED" "$tmp/stdout" || extra=missing_skip_msg
assert "--skip-live-extract bypasses command execution" 0 "$rc" "$extra"
rm -rf "$tmp"

# --- Case 18: --skip-live-extract still validates the key table ---
tmp=$(mktemp -d)
printf '%s' '# CHANGELOG
```text-numeric
123 nosuchkey
```
' > "$tmp/CHANGELOG.md"
mkdir -p "$tmp/src"
printf 'fn main() {}\n' > "$tmp/src/main.rs"
CHANGELOG="$tmp/CHANGELOG.md" SRC_DIR="$tmp/src" \
    PYKRETE_SKIP_LIVE_EXTRACT=1 \
    bash "$GATE" > "$tmp/stdout" 2> "$tmp/stderr"
rc=$?
extra=ok
grep -qF "unknown numeric-claim key: 'nosuchkey'" "$tmp/stderr" || extra=missing_unknown_msg
assert "--skip-live-extract still validates the claim-key allowlist" 1 "$rc" "$extra"
rm -rf "$tmp"

# --- Case 19: mixed text + text-numeric block — both label paths exercised together ---
run_numeric_case '# CHANGELOG
```text
present-text-line
```
prose between blocks
```text-numeric
253 probes
112 fixtures
```
' 'numeric_claim_command() {
    case "$1" in
        probes) echo "echo 253" ;;
        fixtures) echo "echo 112" ;;
        *) return 1 ;;
    esac
}'
# Source for the text block needs to contain "present-text-line" — but the
# default run_numeric_case stubs src/main.rs without it. Patch it here:
printf 'fn main() { println!("present-text-line"); }\n' > "$LAST_TMP/src/main.rs"
CHANGELOG="$LAST_TMP/CHANGELOG.md" SRC_DIR="$LAST_TMP/src" \
    PYKRETE_NUMERIC_CLAIM_TABLE_OVERRIDE="$LAST_TMP/override.sh" \
    bash "$GATE" > "$LAST_TMP/stdout" 2> "$LAST_TMP/stderr"
LAST_RC=$?
extra=ok
grep -q "2 text-numeric claim" "$LAST_TMP/stdout" || extra=wrong_numeric_count
grep -q "1 source-anchored line" "$LAST_TMP/stdout" || extra="${extra}+wrong_source_count"
assert "mixed text + text-numeric blocks both checked" 0 "$LAST_RC" "$extra"
rm -rf "$LAST_TMP"

echo
echo "Test summary: $pass passed, $fail failed"
if [ "$fail" -ne 0 ]; then
    printf '  - %s\n' "${failures[@]}"
    exit 1
fi
exit 0
