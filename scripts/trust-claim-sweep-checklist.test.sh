#!/usr/bin/env bash
# Unit tests for scripts/trust-claim-sweep-checklist.sh.
# Run from repo root: bash scripts/trust-claim-sweep-checklist.test.sh
# Each case writes a fixture repo skeleton, invokes the real script with
# REPO_ROOT pointed at the temp dir + an explicit --current-version, and
# asserts exit code + (where relevant) the PRIOR-RELEASE-NUMBER-LEAKED
# line shape.

set -u

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE="$SCRIPT_DIR/trust-claim-sweep-checklist.sh"

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

# Build a CHANGELOG with v1.10 + v1.9 historical pins. Used by most cases
# as "the prior-release reference."
CHANGELOG_BASELINE='# Changelog

## [1.10.0]
Pins for v1.10.

```text-numeric-historical
261 probes
186 positive
75 negative
120 fixtures
1738 tests
17 donors
```

## [1.9.0]
Pins for v1.9.

```text-numeric-historical
255 probes
114 fixtures
1650 tests
17 donors
```
'

new_repo() {
    local tmp
    tmp=$(mktemp -d)
    mkdir -p "$tmp/docs-site/src/content/docs"
    mkdir -p "$tmp/editors/vscode"
    printf 'CHANGELOG\n' > "$tmp/CHANGELOG.md"
    printf 'README\n' > "$tmp/README.md"
    printf '# vscode CHANGELOG\n' > "$tmp/editors/vscode/CHANGELOG.md"
    printf '# vscode README\n' > "$tmp/editors/vscode/README.md"
    printf '%s' "$tmp"
}

run_gate() {
    local repo="$1"
    local current="$2"
    shift 2
    REPO_ROOT="$repo" bash "$GATE" --current-version "$current" "$@" > "$repo/stdout" 2> "$repo/stderr"
    LAST_RC=$?
    LAST_REPO="$repo"
    return 0
}

# --- Case 1: clean repo (v1.10 numbers in README, v1.10 as current) → 0 ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README with current numbers: 261 probes across 120 fixtures.\n' > "$repo/README.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "scanned" "$repo/stdout" || extra=missing_summary
assert "clean: current numbers in README pass" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 2: drift — v1.9 number left in README → exit 1 with leak line ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README still says: 255 probes (stale!).\n' > "$repo/README.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "PRIOR-RELEASE-NUMBER-LEAKED: README.md" "$repo/stderr" || extra=missing_leak_line
grep -qF "'255 probes' is v1.9.0's number, not v1.10.0's" "$repo/stderr" || extra="${extra}+missing_version_context"
assert "drift: v1.9 number in README fails with PRIOR-RELEASE-NUMBER-LEAKED" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 3: same number inside historical CHANGELOG section → exit 0 ---
# The v1.9 historical section is allowed to carry "255 probes" verbatim.
# The script masks anything from the 2nd `## ` onwards in any CHANGELOG.md.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: 261 probes.\n' > "$repo/README.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "PRIOR-RELEASE-NUMBER-LEAKED" "$repo/stderr" && extra=should_have_been_clean
assert "historical: same number inside v1.9.0 section passes" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 4: backtick-wrapped prior number → exit 0 (escape hatch) ---
BT=$(printf '\140')
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: history mentions %s255 probes%s as a footnote.\n' "$BT" "$BT" > "$repo/README.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "PRIOR-RELEASE-NUMBER-LEAKED" "$repo/stderr" && extra=should_have_been_clean
assert "escape hatch: backtick-wrapped prior number passes" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 5: leak in README + docs-site → both reported ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf '255 probes here in README.\n' > "$repo/README.md"
printf '255 probes here in docs.\n' > "$repo/docs-site/src/content/docs/index.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "PRIOR-RELEASE-NUMBER-LEAKED: README.md" "$repo/stderr" || extra=missing_readme_leak
grep -qF "PRIOR-RELEASE-NUMBER-LEAKED: docs-site/src/content/docs/index.md" "$repo/stderr" || extra="${extra}+missing_docs_leak"
assert "multi-file: leaks in README + docs each reported on own line" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 6: multi-key leak (255 probes AND 114 fixtures) → both reported ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'Stale: 255 probes and 114 fixtures.\n' > "$repo/README.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "'255 probes'" "$repo/stderr" || extra=missing_probes_leak
grep -qF "'114 fixtures'" "$repo/stderr" || extra="${extra}+missing_fixtures_leak"
assert "multi-key: probes + fixtures both reported" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 7: D-code prefix (D01650) doesn't match 1650 → exit 0 ---
# `1650 tests` is the v1.9 pin. `D01650` shares the digits but the
# (?<![A-Za-z0-9_]) prefix anchor rejects identifier-prefixed digits.
# Synthetic D-code chosen for unambiguity — pykrete D-codes are 4 digits
# so this won't collide with a real code.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'Reference to D01650 tests for the fictional discriminator.\n' > "$repo/README.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "PRIOR-RELEASE-NUMBER-LEAKED" "$repo/stderr" && extra=should_have_been_clean
assert "D-code prefix: 'D01650 tests' does NOT match '1650 tests'" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 8: --skip-pykrete-tests works when sibling absent → exit 0 ---
# Already exercised implicitly by every test above (the temp repo has no
# sibling pykrete-tests dir). The explicit assertion here is that the
# warning line is suppressed when --skip-pykrete-tests is set.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'Clean README: 261 probes.\n' > "$repo/README.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "pykrete-tests sibling not present" "$repo/stderr" && extra=warning_should_be_suppressed
assert "--skip-pykrete-tests suppresses the absent-sibling warning" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 9: pin unchanged between releases is NOT a leak ---
# `17 donors` appears in BOTH the v1.10 + v1.9 historical blocks. The
# script must compare prior to current and skip identical pairs.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'Across 17 donors (the live current number).\n' > "$repo/README.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "PRIOR-RELEASE-NUMBER-LEAKED" "$repo/stderr" && extra=unchanged_pin_should_not_fire
assert "unchanged pin (17 donors v1.9==v1.10) is NOT flagged" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 10: text-numeric-historical block inside README is masked ---
# A docs surface can carry an explicit historical pin block (same label
# as CHANGELOG). The script masks these too — content inside the block
# is allowed to be stale by design.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
{
    printf 'Some prose.\n\n'
    printf '```text-numeric-historical\n'
    printf '255 probes\n'
    printf '```\n\n'
    printf 'Live: 261 probes.\n'
} > "$repo/README.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "PRIOR-RELEASE-NUMBER-LEAKED" "$repo/stderr" && extra=historical_block_should_be_masked
assert "text-numeric-historical fenced block content is masked" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 11: missing prior CHANGELOG section → exit 0 with skip note ---
repo=$(new_repo)
printf '# Changelog\n\n## [1.10.0]\nNo prior section in CHANGELOG.\n' > "$repo/CHANGELOG.md"
printf '255 probes (would be a leak if prior pins were findable).\n' > "$repo/README.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "no prior-release pins found" "$repo/stdout" || extra=missing_no_pins_note
assert "missing prior CHANGELOG section short-circuits clean" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 12: --current-version with no value steals next flag (B1.1) ---
# Repro of v1.10 PR-V1 R2 flag-stealing: a bare `--current-version` followed
# by `--skip-pykrete-tests` must NOT silently consume the next flag and
# exit 0 against an empty prior. Expected exit 2 with the clear error.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README.\n' > "$repo/README.md"
REPO_ROOT="$repo" bash "$GATE" --current-version --skip-pykrete-tests > "$repo/stdout" 2> "$repo/stderr"
LAST_RC=$?
extra=ok
grep -qF -- "--current-version requires a version value" "$repo/stderr" || extra=missing_value_error
assert "B1.1: bare --current-version followed by flag exits 2 (not silent 0)" 2 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 13: --current-version=invalid fails fast (B1.2) ---
# Repro of v1.10 PR-V1 R2 invalid-value silent-clean: a non-numeric version
# must NOT pass the emptiness check and then blow up inside arithmetic.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README.\n' > "$repo/README.md"
REPO_ROOT="$repo" bash "$GATE" --current-version=invalid --skip-pykrete-tests > "$repo/stdout" 2> "$repo/stderr"
LAST_RC=$?
extra=ok
grep -qF -- "--current-version must be X.Y.Z" "$repo/stderr" || extra=missing_xyz_error
assert "B1.2: --current-version=invalid exits 2 with X.Y.Z error" 2 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 14: --current-version=1.11 (2-part) fails fast (B1.3) ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README.\n' > "$repo/README.md"
REPO_ROOT="$repo" bash "$GATE" --current-version=1.11 --skip-pykrete-tests > "$repo/stdout" 2> "$repo/stderr"
LAST_RC=$?
extra=ok
grep -qF -- "--current-version must be X.Y.Z" "$repo/stderr" || extra=missing_xyz_error
assert "B1.3: --current-version=1.11 (2-part) exits 2" 2 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 15: space-form --current-version 1.11.0 works (B1.4 positive) ---
# Confirms the new flag-rejection didn't break the well-formed space form.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README with current numbers: 261 probes.\n' > "$repo/README.md"
REPO_ROOT="$repo" bash "$GATE" --current-version 1.10.0 --skip-pykrete-tests > "$repo/stdout" 2> "$repo/stderr"
LAST_RC=$?
extra=ok
grep -qF "scanned" "$repo/stdout" || extra=missing_summary
assert "B1.4: space-form --current-version 1.10.0 works as expected" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 16: malformed CHANGELOG that crashes parser exits 2 (I1) ---
# Force the Python parser to crash by handing it a non-UTF-8 byte. The
# tmpfile+exit-code check must turn this into exit 2, NOT silent 0.
repo=$(new_repo)
# Write bytes that violate UTF-8 decode strict mode.
printf '\xff\xfe garbage\n' > "$repo/CHANGELOG.md"
printf 'README.\n' > "$repo/README.md"
REPO_ROOT="$repo" bash "$GATE" --current-version=1.10.0 --skip-pykrete-tests > "$repo/stdout" 2> "$repo/stderr"
LAST_RC=$?
extra=ok
grep -qF "CHANGELOG parser failed" "$repo/stderr" || extra=missing_parser_failed_msg
assert "I1: CHANGELOG parser crash exits 2 (not silent 0)" 2 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case 17: --help lists --current-version + --skip-pykrete-tests (I2) ---
help_out=$(bash "$GATE" --help 2>&1)
help_rc=$?
extra=ok
printf '%s' "$help_out" | grep -qF -- "--current-version=X.Y.Z" || extra=missing_current_version_in_help
printf '%s' "$help_out" | grep -qF -- "--skip-pykrete-tests" || extra="${extra}+missing_skip_in_help"
printf '%s' "$help_out" | grep -qF "Usage:" || extra="${extra}+missing_usage_header"
assert "I2: --help lists all flags + USAGE header" 0 "$help_rc" "$extra"

# --- summary ---
echo
echo "=========================================="
echo "  trust-claim-sweep tests: $pass passed, $fail failed"
echo "=========================================="
if [ "$fail" -ne 0 ]; then
    for f in "${failures[@]}"; do
        echo "  - $f"
    done
    exit 1
fi
exit 0
