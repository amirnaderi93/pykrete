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
printf '%s' "$help_out" | grep -qF -- "--snapshot" || extra="${extra}+missing_snapshot_in_help"
printf '%s' "$help_out" | grep -qF "Usage:" || extra="${extra}+missing_usage_header"
assert "I2: --help lists all flags + USAGE header" 0 "$help_rc" "$extra"

# === v1.13 PR-A1 — backtick-preservation tripwire tests ===
#
# These exercise the snapshot/tripwire layer that closes the 2-cycle
# PR-G regression at v1.11 + v1.12 (PR-G dev unwraps backticks on a
# historical batch number in pykrete-tests/README.md). The snapshot
# files are written under the temp repo's `scripts/` dir so the
# script's default SNAPSHOT_FILE resolves correctly under REPO_ROOT.

BT=$(printf '\140')

write_snapshot() {
    local repo="$1"
    shift
    mkdir -p "$repo/scripts"
    : > "$repo/scripts/trust-claim-sweep-checklist.snapshot.txt"
    for line in "$@"; do
        printf '%s\n' "$line" >> "$repo/scripts/trust-claim-sweep-checklist.snapshot.txt"
    done
}

# --- Case T1: pin present in surface + present in snapshot → pass ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README with current numbers: 261 probes across 120 fixtures.\n' > "$repo/README.md"
printf "Historical: %s255 probes%s and %s114 fixtures%s.\n" "$BT" "$BT" "$BT" "$BT" >> "$repo/README.md"
write_snapshot "$repo" \
    "README.md:${BT}255 probes${BT}" \
    "README.md:${BT}114 fixtures${BT}"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "BACKTICK-PRESERVATION-FAIL" "$repo/stderr" && extra=tripwire_should_be_clean
grep -qF "tripwire scanned 2 snapshot pin(s); 0 missing" "$repo/stdout" || extra="${extra}+missing_tripwire_summary"
assert "T1: pin present in surface + snapshot → tripwire clean" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case T2: pin in snapshot but unwrapped on tree → tripwire fails ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
# README mentions the number bare (no backticks). Snapshot says backticks
# were there at snapshot time. The tripwire fires.
# Use a number that is NOT a prior-release pin so the prior-release sweep
# stays clean and the only failure is the tripwire (isolates the check).
printf 'README with current 261 probes; legacy was 1234 probes (lost backticks).\n' > "$repo/README.md"
write_snapshot "$repo" \
    "README.md:${BT}1234 probes${BT}"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "BACKTICK-PRESERVATION-FAIL: README.md" "$repo/stderr" || extra=missing_tripwire_fail_line
grep -qF "${BT}1234 probes${BT}" "$repo/stderr" || extra="${extra}+missing_pin_in_message"
grep -qF "Restore the backticks or refresh the snapshot with --snapshot" "$repo/stderr" || extra="${extra}+missing_restore_hint"
assert "T2: snapshotted pin unwrapped on tree → tripwire fails" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case T3: pin present on tree + absent from snapshot → pass (additive) ---
# A new backticked pin that isn't yet snapshotted must not trip the
# tripwire — the snapshot is additive (PR-F's --snapshot refresh sweeps
# new pins in at cycle close).
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf "README: 261 probes; history mentions %s99 probes%s as a new pin.\n" "$BT" "$BT" > "$repo/README.md"
# Empty snapshot file (no pins recorded yet) — but tripwire needs SOMETHING
# in it to be "active". An empty file is the DEGRADED mode (T6); to test
# the additive case, write a snapshot that contains an UNRELATED pin which
# IS present on the tree.
printf "Footnote %s17 donors%s.\n" "$BT" "$BT" >> "$repo/README.md"
write_snapshot "$repo" \
    "README.md:${BT}17 donors${BT}"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "BACKTICK-PRESERVATION-FAIL" "$repo/stderr" && extra=additive_new_pin_should_not_trip
assert "T3: new backticked pin not yet snapshotted does NOT trip tripwire" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case T4: --snapshot writes snapshot + skips tripwire + exits 0 ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
# Author wants two backticked pins captured.
printf "Historical: %s99 probes%s and %s88 fixtures%s.\n" "$BT" "$BT" "$BT" "$BT" > "$repo/README.md"
# Seed an OLD snapshot whose pins are absent on the tree — proves that
# --snapshot REWRITES (not appends) and does NOT trip on stale entries.
write_snapshot "$repo" \
    "README.md:${BT}99999 probes${BT}"
# Place the source-of-truth backticked pins on a surface the snapshot
# routine scans. The snapshot routine scans pykrete-tests/README.md +
# editors/vscode/CHANGELOG.md only — not the root README. Use the
# vscode CHANGELOG.
printf "## 0.10.0\nHistorical: %s99 probes%s and %s88 fixtures%s.\n" "$BT" "$BT" "$BT" "$BT" > "$repo/editors/vscode/CHANGELOG.md"
REPO_ROOT="$repo" bash "$GATE" --snapshot --skip-pykrete-tests > "$repo/stdout" 2> "$repo/stderr"
LAST_RC=$?
extra=ok
grep -qF "wrote 2 snapshot pin(s)" "$repo/stdout" || extra=missing_snapshot_write_summary
grep -qF "editors/vscode/CHANGELOG.md:${BT}99 probes${BT}" "$repo/scripts/trust-claim-sweep-checklist.snapshot.txt" || extra="${extra}+missing_pin1"
grep -qF "editors/vscode/CHANGELOG.md:${BT}88 fixtures${BT}" "$repo/scripts/trust-claim-sweep-checklist.snapshot.txt" || extra="${extra}+missing_pin2"
grep -qF "99999 probes" "$repo/scripts/trust-claim-sweep-checklist.snapshot.txt" && extra="${extra}+stale_pin_not_rewritten"
grep -q "BACKTICK-PRESERVATION-FAIL" "$repo/stderr" && extra="${extra}+snapshot_ran_tripwire"
grep -q "tripwire scanned" "$repo/stdout" && extra="${extra}+snapshot_ran_tripwire_in_stdout"
assert "T4: --snapshot writes fresh snapshot + skips tripwire" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case T5: pin wrapped in DOUBLE backticks recognized as wrapped ---
# The tripwire's grep -F search for `<single-backtick>pin<single-backtick>`
# must NOT false-match on a `` ``pin`` `` (double-backtick) wrapping, AND
# the surface containing a double-backtick wrap must still satisfy the
# snapshot (the inner single-backtick pair is a substring of the double-
# wrap, so it appears verbatim).
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
# Double-backtick wrap: `` `1.7.0` ``
printf "Historical: %s%s1.7.0%s%s in markdown-escaped form.\n" "$BT$BT" "" "$BT$BT" "" > "$repo/README.md"
# Wait: the brief uses `` ``1.7.0`` `` (double-backtick on each side).
# Build that explicitly:
printf "Double-wrap: %s%s1.7.0%s%s here.\n" "$BT$BT" "" "$BT$BT" "" >> "$repo/README.md"
# Snapshot the inner single-backtick `1.7.0`. The outer double-wrap renders
# the inner single-backticks as literal characters, so grep -F finds them.
write_snapshot "$repo" \
    "README.md:${BT}1.7.0${BT}"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "BACKTICK-PRESERVATION-FAIL" "$repo/stderr" && extra=double_wrap_should_satisfy_tripwire
assert "T5: double-backtick wrap satisfies tripwire (inner single-bt is substring)" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case T6: snapshot file missing → exit 0 (degraded mode, warn) ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: 261 probes.\n' > "$repo/README.md"
# No snapshot file at all.
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "backtick-preservation snapshot not found" "$repo/stderr" || extra=missing_degraded_warn
grep -q "BACKTICK-PRESERVATION-FAIL" "$repo/stderr" && extra="${extra}+should_not_fail_on_missing_snapshot"
assert "T6: missing snapshot file → exit 0 with degraded-mode warning" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case T7: snapshot file empty → exit 0 (degraded mode, warn) ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: 261 probes.\n' > "$repo/README.md"
mkdir -p "$repo/scripts"
: > "$repo/scripts/trust-claim-sweep-checklist.snapshot.txt"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "backtick-preservation snapshot at scripts/trust-claim-sweep-checklist.snapshot.txt is empty" "$repo/stderr" || extra=missing_empty_warn
grep -q "BACKTICK-PRESERVATION-FAIL" "$repo/stderr" && extra="${extra}+should_not_fail_on_empty_snapshot"
assert "T7: empty snapshot file → exit 0 with degraded-mode warning" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case T8: malformed snapshot line → tripwire fails fast ---
# A snapshot line without a colon-separator is unparseable. The tripwire
# treats this as a build-time bug in the snapshot file and exits 1
# without false-positive on the unparseable entry.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: 261 probes.\n' > "$repo/README.md"
write_snapshot "$repo" \
    "garbage-line-no-colon-separator"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "malformed snapshot line" "$repo/stderr" || extra=missing_malformed_error
assert "T8: malformed snapshot line fails fast (exit 1)" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case T9: snapshot points to missing surface → tripwire fails ---
# If the snapshot references a file that no longer exists in the tree
# (file deleted in a refactor), the tripwire must fail loud, not silently
# pass. The skip-pykrete-tests carve-out (T9b) is the documented exception.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: 261 probes.\n' > "$repo/README.md"
write_snapshot "$repo" \
    "deleted/path.md:${BT}1234 probes${BT}"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "BACKTICK-PRESERVATION-FAIL: deleted/path.md" "$repo/stderr" || extra=missing_deleted_surface_warn
grep -qF "snapshotted surface not present in tree" "$repo/stderr" || extra="${extra}+missing_explanation"
assert "T9: snapshot references missing surface → tripwire fails loud" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case T9b: --skip-pykrete-tests honors the carve-out for sibling ---
# Daily PR CI (no sibling checkout) must not trip on pykrete-tests
# entries in the snapshot — passing --skip-pykrete-tests suppresses the
# missing-surface error specifically for `../pykrete-tests/` prefixed
# entries (and only for those).
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: 261 probes.\n' > "$repo/README.md"
write_snapshot "$repo" \
    "../pykrete-tests/README.md:${BT}1234 probes${BT}"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "BACKTICK-PRESERVATION-FAIL" "$repo/stderr" && extra=should_skip_pykrete_tests_entry
assert "T9b: --skip-pykrete-tests honors carve-out for sibling-prefix entries" 0 "$LAST_RC" "$extra"
# --- v1.13 PR-V1: vscode CHANGELOG per-section masking ---
# Locks in the existing `surface_display.endswith("CHANGELOG.md")` branch's
# behavior for editors/vscode/CHANGELOG.md (first `## ` = current; second
# onward = masked). Closes the 3-cycle manual-backtick workaround at
# PR-G v1.10 / v1.11 / v1.12.

# Use a CHANGELOG with current=1.13.0 + prior=1.12.0 pins so the test fires
# against the post-bump scenario the brief describes.
CHANGELOG_V113='# Changelog

## [1.13.0]
Pins for v1.13.

```text-numeric-historical
279 probes
138 fixtures
17 donors
```

## [1.12.0]
Pins for v1.12.

```text-numeric-historical
271 probes
130 fixtures
17 donors
```
'

# --- Case V1.1: vscode CHANGELOG with current + historical sections is CLEAN
# Current section (## 0.11.0) carries the current pin (279); historical
# section (## 0.10.0) carries the prior pin (271). The header-second-
# onward mask hides 271; 279 matches current so it's not flagged either.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V113" > "$repo/CHANGELOG.md"
{
    printf '# Changelog\n\n'
    printf '## 0.11.0\n'
    printf 'Tracks v1.13.0: 279 probes across 138 fixtures.\n\n'
    printf '## 0.10.0\n'
    printf 'Tracks v1.12.0: 271 probes across 130 fixtures.\n'
} > "$repo/editors/vscode/CHANGELOG.md"
printf 'README: 279 probes.\n' > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "PRIOR-RELEASE-NUMBER-LEAKED" "$repo/stderr" && extra=vscode_historical_should_be_masked
assert "V1.1: vscode CHANGELOG historical section is masked (271 not flagged)" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case V1.2: vscode CHANGELOG with only one `## ` heading → no mask ---
# A single-section vscode CHANGELOG has no historical content to mask. The
# whole body stays scannable; any prior-pin number leaks normally.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V113" > "$repo/CHANGELOG.md"
{
    printf '# Changelog\n\n'
    printf '## 0.11.0\n'
    printf 'Stale narrative: 271 probes here MUST leak (no historical to mask under).\n'
} > "$repo/editors/vscode/CHANGELOG.md"
printf 'README: 279 probes.\n' > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -qF "PRIOR-RELEASE-NUMBER-LEAKED: editors/vscode/CHANGELOG.md" "$repo/stderr" || extra=single_section_should_have_fired
assert "V1.2: single-## vscode CHANGELOG leaks (no historical to mask)" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case V1.3: empty vscode CHANGELOG → degraded clean ---
# Empty file: no headers, no body, no leaks. Exit 0.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V113" > "$repo/CHANGELOG.md"
: > "$repo/editors/vscode/CHANGELOG.md"
printf 'README: 279 probes.\n' > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "PRIOR-RELEASE-NUMBER-LEAKED" "$repo/stderr" && extra=empty_file_should_be_clean
assert "V1.3: empty vscode CHANGELOG is degraded clean" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case V1.4: leak in CURRENT (first `## `) section MUST fire ---
# Regression guard: a stale prior pin inside the first `## ` block of the
# vscode CHANGELOG is NOT masked by the per-section scope.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V113" > "$repo/CHANGELOG.md"
{
    printf '# Changelog\n\n'
    printf '## 0.11.0\n'
    printf 'Tracks v1.13.0 release: stale 271 probes inside CURRENT — MUST FIRE.\n\n'
    printf '## 0.10.0\n'
    printf 'Historical block (masked).\n'
} > "$repo/editors/vscode/CHANGELOG.md"
printf 'README: 279 probes.\n' > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -qF "PRIOR-RELEASE-NUMBER-LEAKED: editors/vscode/CHANGELOG.md" "$repo/stderr" || extra=current_section_leak_should_fire
grep -qF "'271 probes' is v1.12.0's number" "$repo/stderr" || extra="${extra}+missing_version_context"
assert "V1.4: leak in CURRENT vscode section fires PRIOR-RELEASE-NUMBER-LEAKED" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case V1.5: level-3 (`### 0.X.Y`) headers do NOT divide sections ---
# Documents the format constraint from the script comment: only `## `
# (level-2) headers are recognized as section dividers. A vscode CHANGELOG
# that uses `### 0.X.Y` instead would have the whole body treated as a
# single section (no mask), so a prior-pin number in a "historical"-feeling
# block would still leak. PR-V1 chose to lock in the level-2-only contract
# rather than recognize multiple header levels; the negative-space test
# guards against accidental level-3 misuse going undetected.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V113" > "$repo/CHANGELOG.md"
{
    printf '# Changelog\n\n'
    printf '### 0.11.0\n'
    printf 'Pseudo-current using level 3.\n\n'
    printf '### 0.10.0\n'
    printf 'Pseudo-historical using level 3: 271 probes leaks because no `## ` divides.\n'
} > "$repo/editors/vscode/CHANGELOG.md"
printf 'README: 279 probes.\n' > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -qF "PRIOR-RELEASE-NUMBER-LEAKED: editors/vscode/CHANGELOG.md" "$repo/stderr" || extra=level3_should_not_mask
assert "V1.5: level-3 (### 0.X.Y) headers do NOT divide sections (format constraint)" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# === v1.14 PR-A1 — backticked-claim-stale scanner tests (BCS1—BCS7) ===
#
# Closes the v1.13 docs-sync audit 8-blocker pattern (per
# project-v23-retrospective rule 9). A backticked `<num> <key>` in a
# tracked surface must EITHER match a current text-numeric pin OR live
# inside a text-numeric-historical fenced block. CHANGELOG.md sections
# from the SECOND `## ` header onward are also masked (the per-CHANGELOG
# mask is shared with the prior-leak sweep).

# Use a CHANGELOG with a CURRENT text-numeric block + historical blocks
# so the new scanner has both a "truth source" (current pins) and an
# explicit "intentionally historical" carve-out to validate against.
CHANGELOG_V114='# Changelog

## [1.13.0]
Current pins.

```text-numeric
289 probes
148 fixtures
17 donors
```

## [1.12.0]
Historical pins.

```text-numeric-historical
279 probes
138 fixtures
17 donors
```

## [1.10.0]
Older historical pins (two minors back; not the immediate-prior).

```text-numeric-historical
261 probes
120 fixtures
17 donors
```
'

# --- Case BCS1: stale backticked number in README fails ---
# `500 probes` is neither the current pin nor present in any
# text-numeric-historical block. Per the brief, stale backticked
# numbers in README fail with the BACKTICKED-CLAIM-STALE diagnostic.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V114" > "$repo/CHANGELOG.md"
printf "README: legacy %s500 probes%s claim that is neither current nor in a historical block.\n" "$BT" "$BT" > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -qF "BACKTICKED-CLAIM-STALE: README.md" "$repo/stderr" || extra=missing_stale_line
grep -qF "${BT}500 probes${BT}" "$repo/stderr" || extra="${extra}+missing_pin_in_message"
grep -qF "(non-current, non-historical)" "$repo/stderr" || extra="${extra}+missing_explanation"
assert "BCS1: stale backticked number in README fails BACKTICKED-CLAIM-STALE" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case BCS2: backticked number matching current pin passes ---
# `289 probes` IS the current pin. A backticked occurrence outside any
# historical block is valid (escape hatch (a) in the brief).
repo=$(new_repo)
printf '%s' "$CHANGELOG_V114" > "$repo/CHANGELOG.md"
printf "README: today's run reports %s289 probes%s across the fixtures.\n" "$BT" "$BT" > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "BACKTICKED-CLAIM-STALE" "$repo/stderr" && extra=current_pin_should_be_clean
assert "BCS2: backticked number matching current pin passes" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case BCS3: backticked number in text-numeric-historical block passes ---
# A backticked `279 probes` written INSIDE a text-numeric-historical
# fenced block on a surface is intentionally-historical and must NOT
# fire the stale-claim gate (escape hatch (b) in the brief). The
# text-numeric-historical mask carve-out applies to non-CHANGELOG
# surfaces too.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V114" > "$repo/CHANGELOG.md"
{
    printf 'Some prose mentioning the current %s289 probes%s.\n\n' "$BT" "$BT"
    printf '```text-numeric-historical\n'
    printf "Bumped from %s279 probes%s to %s289 probes%s.\n" "$BT" "$BT" "$BT" "$BT"
    printf '```\n'
} > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "BACKTICKED-CLAIM-STALE" "$repo/stderr" && extra=historical_block_should_be_masked
assert "BCS3: backticked number inside text-numeric-historical block passes" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case BCS4: non-numeric backticks (function names) ignored ---
# Backticked spans that aren't `<digits> <allowed-key>` shaped are
# orthogonal to this gate. Function names, file paths, CLI flags, etc.
# must never trip the scanner regardless of content.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V114" > "$repo/CHANGELOG.md"
{
    printf "README mentions %sscan_backticked_stale_numbers()%s and %s--current-version%s.\n" "$BT" "$BT" "$BT" "$BT"
    printf "Also %sscripts/trust-claim-sweep-checklist.sh%s.\n" "$BT" "$BT"
} > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "BACKTICKED-CLAIM-STALE" "$repo/stderr" && extra=non_numeric_should_not_trip
assert "BCS4: non-numeric backticked spans (fn names, flags, paths) ignored" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case BCS5: backticked number with non-tracked key ignored ---
# A backticked `<num> <other-word>` where the key is NOT in the
# text-numeric vocabulary (probes / positive / negative / fixtures /
# tests / donors) is out of scope for this gate. Example: a sentence
# citing `12 columns` or `42 rules` — those numbers aren't pinned in
# the text-numeric block and shouldn't false-positive.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V114" > "$repo/CHANGELOG.md"
printf "README cites %s12 columns%s and %s42 rules%s for context.\n" "$BT" "$BT" "$BT" "$BT" > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "BACKTICKED-CLAIM-STALE" "$repo/stderr" && extra=non_tracked_key_should_not_trip
assert "BCS5: backticked number with non-tracked key (columns, rules) ignored" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case BCS6: stale backticked claim in CURRENT CHANGELOG section fires ---
# A backticked `<num> <key>` inside the FIRST `## ` section of CHANGELOG.md
# (the current-release section) that does NOT match the cycle's
# text-numeric block must fire — the per-CHANGELOG mask only blanks
# everything from the SECOND `## ` onward, so the current section is
# scannable. Confirms the intended behavior per reviewer Question 1.
repo=$(new_repo)
{
    printf '# Changelog\n\n'
    printf '## [1.13.0]\n'
    printf "Stale claim inside CURRENT section: %s500 probes%s.\n\n" "$BT" "$BT"
    printf '```text-numeric\n'
    printf '289 probes\n148 fixtures\n17 donors\n'
    printf '```\n\n'
    printf '## [1.12.0]\n'
    printf 'Historical.\n\n'
    printf '```text-numeric-historical\n279 probes\n138 fixtures\n17 donors\n```\n'
} > "$repo/CHANGELOG.md"
printf 'README.\n' > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -qF "BACKTICKED-CLAIM-STALE: CHANGELOG.md" "$repo/stderr" || extra=missing_stale_changelog_line
grep -qF "${BT}500 probes${BT}" "$repo/stderr" || extra="${extra}+missing_pin_in_message"
assert "BCS6: stale backticked claim in CURRENT CHANGELOG section fires" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case BCS7: stale backticked pin in CHANGELOG.md historical section masked ---
# A stale backticked `<num> <key>` inside the SECOND `## ` section
# onwards (the historical section) of CHANGELOG.md must be masked by
# the existing per-CHANGELOG mask — historical CHANGELOG sections are
# immutable by design and cite their own pinned numbers verbatim. The
# mask carve-out shared with the prior-leak sweep also covers the
# stale-claim scanner. Confirms intended behavior per reviewer Question 2.
repo=$(new_repo)
{
    printf '# Changelog\n\n'
    printf '## [1.13.0]\n'
    printf 'Current.\n\n'
    printf '```text-numeric\n'
    printf '289 probes\n148 fixtures\n17 donors\n'
    printf '```\n\n'
    printf '## [1.12.0]\n'
    printf "Historical narrative mentions %s500 probes%s as a stale (masked) claim.\n\n" "$BT" "$BT"
    printf '```text-numeric-historical\n279 probes\n138 fixtures\n17 donors\n```\n'
} > "$repo/CHANGELOG.md"
printf 'README.\n' > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "BACKTICKED-CLAIM-STALE" "$repo/stderr" && extra=historical_changelog_should_be_masked
assert "BCS7: stale backticked pin in CHANGELOG.md historical section masked" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# === v1.15 PR-A1 — unbackticked-marketing-table scanner tests (MT1—MT7) ==
#
# Closes the v1.14 architecture-auditor finding (project-v24-retrospective).
# A bare `<num> <key>` inside a markdown-table row must EITHER match a
# current text-numeric pin OR live inside a text-numeric-historical
# fenced block — otherwise it's a stale-marketing-table claim. Bare numbers
# in prose paragraphs stay governed by the prior-leak sweep (existing
# behavior preserved).
#
# Reuses the CHANGELOG_V114 fixture with current=1.13.0 (289 probes pin) +
# historical=1.12.0 (279 probes pin).

# --- Case MT1: bare table-row pin matching historical block passes ---
# `261 probes` is in the v1.10.0 text-numeric-historical block (two minors
# back; not the immediate-prior v1.12.0 the prior-leak sweep checks). A
# bare occurrence in a markdown-table row is a trajectory-table cell;
# the validity rule's (b) clause grants immunity.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V114" > "$repo/CHANGELOG.md"
{
    printf '# Trajectory\n\n'
    printf '| Version | Pandas claim | Verifiable? |\n'
    printf '|---|---|---|\n'
    printf '| v1.10.0 | "stuff" | yes — 261 probes total across 17 donors |\n'
    printf '| v1.13.0 | "more stuff" | yes — 289 probes total across 17 donors |\n'
} > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "MARKETING-TABLE-CLAIM-STALE" "$repo/stderr" && extra=historical_table_row_should_pass
assert "MT1: bare table-row pin matching historical block passes" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case MT2: bare table-row pin NOT in any block fires ---
# `500 probes` is neither current nor historical. The scanner fires
# MARKETING-TABLE-CLAIM-STALE with the bare-pin and the line number.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V114" > "$repo/CHANGELOG.md"
{
    printf '# Trajectory\n\n'
    printf '| Version | Verifiable? |\n'
    printf '|---|---|\n'
    printf '| v1.13.0 | yes — 289 probes |\n'
    printf '| v1.99.0 | yes — 500 probes (stale) |\n'
} > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -qF "MARKETING-TABLE-CLAIM-STALE: README.md" "$repo/stderr" || extra=missing_stale_line
grep -qF "'500 probes' bare in table row" "$repo/stderr" || extra="${extra}+missing_pin_in_message"
grep -qF "(non-current, non-historical)" "$repo/stderr" || extra="${extra}+missing_explanation"
assert "MT2: bare table-row pin not in any block fires MARKETING-TABLE-CLAIM-STALE" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case MT3: bare pin in prose paragraph (not a table) does NOT fire ---
# Existing behavior preserved: bare numbers in prose are governed by the
# prior-leak sweep (catches the specific prior-release number) plus the
# CHANGELOG grep gate v3 (catches prose-paragraph numerics in CHANGELOG).
# This scanner is INTENTIONALLY narrowed to table-row contexts only.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V114" > "$repo/CHANGELOG.md"
printf 'README prose with bare 500 probes mention (would be a leak if scanned).\n' > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "MARKETING-TABLE-CLAIM-STALE" "$repo/stderr" && extra=prose_should_not_be_scanned
assert "MT3: bare pin in prose paragraph does NOT fire (table-only scope)" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case MT4: backticked table-row pin handled by backticked-claim-stale ---
# A backticked `<num> <key>` is owned by the BACKTICKED-CLAIM-STALE scanner
# (existing behavior). The unbackticked scanner's preceding-backtick anchor
# excludes the backticked form so this scanner sees nothing — confirms the
# two scanners don't double-fire on the same pin.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V114" > "$repo/CHANGELOG.md"
{
    printf '# Trajectory\n\n'
    printf '| Version | Verifiable? |\n'
    printf '|---|---|\n'
    printf "| v1.99.0 | yes — %s500 probes%s |\n" "$BT" "$BT"
} > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "MARKETING-TABLE-CLAIM-STALE" "$repo/stderr" && extra=backticked_should_not_double_fire
# Confirm backticked-claim-stale DID fire (it's the right scanner for this pin).
grep -qF "BACKTICKED-CLAIM-STALE: README.md" "$repo/stderr" || extra="${extra}+backticked_stale_should_have_fired"
assert "MT4: backticked table-row pin owned by backticked-claim-stale (no double-fire)" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case MT5: bare table-row pin matching current passes (escape hatch a) ---
# `289 probes` IS the current pin. Identical to MT1 but exercising the (a)
# clause of the validity rule rather than (b).
repo=$(new_repo)
printf '%s' "$CHANGELOG_V114" > "$repo/CHANGELOG.md"
{
    printf '# Trajectory\n\n'
    printf '| Version | Verifiable? |\n'
    printf '|---|---|\n'
    printf '| v1.13.0 (current) | yes — 289 probes across 148 fixtures |\n'
} > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "MARKETING-TABLE-CLAIM-STALE" "$repo/stderr" && extra=current_table_row_should_pass
assert "MT5: bare table-row pin matching current pin passes" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case MT6: separator row (`|---|`) is harmless ---
# Pipe-table separator rows match the table-row heuristic but contain
# no `<num> <key>` patterns. Confirm they don't false-match.
repo=$(new_repo)
printf '%s' "$CHANGELOG_V114" > "$repo/CHANGELOG.md"
{
    printf '# Pretty empty table\n\n'
    printf '| col1 | col2 |\n'
    printf '|---|---|\n'
    printf '| a | b |\n'
} > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "MARKETING-TABLE-CLAIM-STALE" "$repo/stderr" && extra=separator_should_not_match
assert "MT6: separator row (|---|) is harmless" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case MT7: bare table-row pin in CHANGELOG.md historical section masked ---
# The per-CHANGELOG mask from the 2nd `## ` onward shared with the prior-
# leak + stale-claim scanners also applies to the marketing-table scanner.
# A bare-in-table-row stale pin inside a historical CHANGELOG section is
# masked.
repo=$(new_repo)
{
    printf '# Changelog\n\n'
    printf '## [1.13.0]\n'
    printf 'Current.\n\n'
    printf '```text-numeric\n'
    printf '289 probes\n148 fixtures\n17 donors\n'
    printf '```\n\n'
    printf '## [1.12.0]\n'
    printf 'Historical narrative with a stale TABLE-ROW pin (masked):\n\n'
    printf '| Version | Verifiable? |\n'
    printf '|---|---|\n'
    printf '| v1.99.0 | yes — 500 probes (masked) |\n'
    printf '\n```text-numeric-historical\n279 probes\n138 fixtures\n17 donors\n```\n'
} > "$repo/CHANGELOG.md"
printf 'README.\n' > "$repo/README.md"
run_gate "$repo" 1.13.0 --skip-pykrete-tests
extra=ok
grep -q "MARKETING-TABLE-CLAIM-STALE" "$repo/stderr" && extra=historical_changelog_table_row_should_be_masked
assert "MT7: bare table-row pin in CHANGELOG.md historical section masked" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# === v1.16 PR-A1 — --expected-failures.json allowlist tests (EF1—EF7) ===
#
# The allowlist gives PR-F a declared, LOGGED escape hatch for surfaces
# PR-G resolves this cycle. Each entry carries a MANDATORY expiresAfter
# version: once CURRENT_VERSION exceeds it the entry is STALE and the gate
# fails LOUD (v1.15 retro rule 2 — countdown, not dumping-ground). Reuses
# the CHANGELOG_BASELINE fixture (current=1.10.0; v1.9's `255 probes` +
# `114 fixtures` are the prior-leak pins).

write_ef() {
    # write_ef <repo> <json-body>
    printf '%s' "$2" > "$1/ef.json"
}

# --- Case EF1: active entry suppresses a matching fire → exit 0 ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README still says: 255 probes (deferred to a follow-up).\n' > "$repo/README.md"
write_ef "$repo" '{ "entries": [ { "surface": "README.md", "pattern": "255 probes", "reason": "known stale; resolved by follow-up", "expiresAfter": "1.10.0" } ] }'
run_gate "$repo" 1.10.0 --skip-pykrete-tests --expected-failures ef.json
extra=ok
grep -qF "EXPECTED-FAILURE-SUPPRESSED: README.md: 255 probes (expiresAfter 1.10.0)" "$repo/stderr" || extra=missing_suppressed_line
grep -q "PRIOR-RELEASE-NUMBER-LEAKED" "$repo/stderr" && extra="${extra}+suppressed_fire_should_not_survive"
assert "EF1: active entry suppresses matching fire (exit 0, logged)" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case EF2: expired entry fails LOUD even though the fire matches ---
# expiresAfter 1.9.0 < current 1.10.0 → EXPECTED-FAILURE-EXPIRED, exit 1.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README still says: 255 probes.\n' > "$repo/README.md"
write_ef "$repo" '{ "entries": [ { "surface": "README.md", "pattern": "255 probes", "reason": "was deferred, now overdue", "expiresAfter": "1.9.0" } ] }'
run_gate "$repo" 1.10.0 --skip-pykrete-tests --expected-failures ef.json
extra=ok
grep -qF "EXPECTED-FAILURE-EXPIRED: README.md entry expiresAfter 1.9.0 but current is 1.10.0" "$repo/stderr" || extra=missing_expired_line
assert "EF2: expired entry (expiresAfter < current) fails LOUD (exit 1)" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case EF3: active entry matching no fire on a clean repo → warn, exit 0 ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README clean: 261 probes.\n' > "$repo/README.md"
write_ef "$repo" '{ "entries": [ { "surface": "README.md", "pattern": "255 probes", "reason": "held for PR-G but already resolved", "expiresAfter": "1.10.0" } ] }'
run_gate "$repo" 1.10.0 --skip-pykrete-tests --expected-failures ef.json
extra=ok
grep -qF "EXPECTED-FAILURE-DEAD: README.md: 255 probes matched no actual fire (expiresAfter 1.10.0)" "$repo/stderr" || extra=missing_dead_line
assert "EF3: dead entry (no matching fire) warns but does not fail (exit 0)" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case EF4: malformed allowlist JSON fails fast (exit 2) ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README.\n' > "$repo/README.md"
write_ef "$repo" '{ not valid json '
run_gate "$repo" 1.10.0 --skip-pykrete-tests --expected-failures ef.json
extra=ok
grep -qF "could not parse --expected-failures" "$repo/stderr" || extra=missing_parse_error
assert "EF4: malformed allowlist JSON fails fast (exit 2)" 2 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case EF5: partial suppression — one fire suppressed, one survives → exit 1 ---
# README leaks BOTH 255 probes AND 114 fixtures (v1.9 pins). The allowlist
# only holds 255 probes; 114 fixtures survives and fails the gate.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'Stale: 255 probes and 114 fixtures.\n' > "$repo/README.md"
write_ef "$repo" '{ "entries": [ { "surface": "README.md", "pattern": "255 probes", "reason": "only this one is deferred", "expiresAfter": "1.10.0" } ] }'
run_gate "$repo" 1.10.0 --skip-pykrete-tests --expected-failures ef.json
extra=ok
grep -qF "EXPECTED-FAILURE-SUPPRESSED: README.md: 255 probes" "$repo/stderr" || extra=missing_suppressed
grep -qF "'114 fixtures' is v1.9.0's number" "$repo/stderr" || extra="${extra}+surviving_fire_should_be_reemitted"
assert "EF5: partial suppression — unheld fire survives (exit 1)" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case EF6: expired entry fails even when the repo is otherwise clean ---
# Expiry is a countdown independent of whether the entry matched a fire —
# an overdue entry is a hard error regardless of repo state.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README clean: 261 probes.\n' > "$repo/README.md"
write_ef "$repo" '{ "entries": [ { "surface": "README.md", "pattern": "255 probes", "reason": "overdue", "expiresAfter": "1.9.0" } ] }'
run_gate "$repo" 1.10.0 --skip-pykrete-tests --expected-failures ef.json
extra=ok
grep -qF "EXPECTED-FAILURE-EXPIRED: README.md entry expiresAfter 1.9.0" "$repo/stderr" || extra=missing_expired_line
assert "EF6: expired entry fails even on a clean repo (countdown enforcement)" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case EF7: --expected-failures pointing at a missing file → exit 2 ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README.\n' > "$repo/README.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests --expected-failures does-not-exist.json
extra=ok
grep -qF -- "--expected-failures file not found" "$repo/stderr" || extra=missing_notfound_error
assert "EF7: missing allowlist file fails fast (exit 2)" 2 "$LAST_RC" "$extra"
rm -rf "$repo"

# === v1.16 PR-A1 — roadmap-header guard tests (RG1—RG5) =================
#
# Per v1.16-spec §1.iv.1. The guard asserts BOTH (a) the highest
# "## Where we are (vX.Y.Z)" header in pandas-roadmap.md AND (b) the highest
# "## Shipped in vX.Y" section across the three roadmap docs match
# CURRENT_VERSION. Fixtures keep roadmap files header-only (no `<num> <key>`
# pins) so only the roadmap guard is exercised, not the prior-leak/stale/
# table scanners.

write_roadmap_pandas() {
    # write_roadmap_pandas <repo> <body>
    mkdir -p "$1/docs-site/src/content/docs/about"
    printf '%s' "$2" > "$1/docs-site/src/content/docs/about/pandas-roadmap.md"
}

# --- Case RG1: header + shipped section match current → passes ---
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: 261 probes.\n' > "$repo/README.md"
write_roadmap_pandas "$repo" '# Pandas roadmap

## Where we are (v1.10.0)

Current state.

## Shipped in v1.10

Recent work.
'
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "ROADMAP-HEADER-DRIFT" "$repo/stderr" && extra=should_be_clean
grep -qF "roadmap-header guard scanned" "$repo/stdout" || extra="${extra}+missing_guard_summary"
assert "RG1: header + shipped section match current → passes" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case RG2: stale 'Where we are' header fires (a) ---
# Header v1.9.0 ≠ current 1.10.0; shipped section is current so only (a) fires.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: 261 probes.\n' > "$repo/README.md"
write_roadmap_pandas "$repo" '# Pandas roadmap

## Where we are (v1.9.0)

Two cycles stale.

## Shipped in v1.10

Recent work.
'
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "ROADMAP-HEADER-DRIFT: docs-site/src/content/docs/about/pandas-roadmap.md" "$repo/stderr" || extra=missing_drift_line
grep -qF "'Where we are (v1.9.0)' header is v1.9.0 but CURRENT_VERSION is v1.10.0" "$repo/stderr" || extra="${extra}+missing_header_context"
assert "RG2: stale 'Where we are' header fires ROADMAP-HEADER-DRIFT (exit 1)" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case RG3: stale 'Shipped in' section fires (b) ---
# Header v1.10.0 is current so (a) passes; highest shipped is v1.9 → (b) fires.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: 261 probes.\n' > "$repo/README.md"
write_roadmap_pandas "$repo" '# Pandas roadmap

## Where we are (v1.10.0)

Current header.

## Shipped in v1.9

No v1.10 shipped section anywhere.
'
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -qF "highest 'Shipped in v1.9' section is v1.9 but CURRENT_VERSION is v1.10.0" "$repo/stderr" || extra=missing_shipped_drift
assert "RG3: stale highest 'Shipped in' section fires ROADMAP-HEADER-DRIFT (exit 1)" 1 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case RG4: expected-failure entry suppresses the roadmap drift ---
# Demonstrates the §1.iii.2 items-1+4 interlock: the guard fires on the
# stale header, the allowlist holds it (expiresAfter 1.10.0 == current).
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: 261 probes.\n' > "$repo/README.md"
write_roadmap_pandas "$repo" '# Pandas roadmap

## Where we are (v1.9.0)

Stale, but held by the allowlist.

## Shipped in v1.10

Recent.
'
write_ef "$repo" '{ "entries": [ { "surface": "docs-site/src/content/docs/about/pandas-roadmap.md", "pattern": "Where we are", "reason": "header drift; resolved by PR-G", "expiresAfter": "1.10.0" } ] }'
run_gate "$repo" 1.10.0 --skip-pykrete-tests --expected-failures ef.json
extra=ok
grep -qF "EXPECTED-FAILURE-SUPPRESSED: docs-site/src/content/docs/about/pandas-roadmap.md: Where we are (expiresAfter 1.10.0)" "$repo/stderr" || extra=missing_suppressed
grep -q "ROADMAP-HEADER-DRIFT" "$repo/stderr" && extra="${extra}+drift_should_be_suppressed"
assert "RG4: expected-failure entry suppresses roadmap drift (exit 0)" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

# --- Case RG5: (b) is a cross-file max — a current shipped section in ANY
# roadmap doc satisfies (b) even if pandas-roadmap.md lags. Mirrors the live
# repo where about/roadmap.md carries the current section.
repo=$(new_repo)
printf '%s' "$CHANGELOG_BASELINE" > "$repo/CHANGELOG.md"
printf 'README: 261 probes.\n' > "$repo/README.md"
write_roadmap_pandas "$repo" '# Pandas roadmap

## Where we are (v1.10.0)

Current header.

## Shipped in v1.9

Pandas page lags on the shipped section.
'
printf '# Roadmap\n\n## Shipped in v1.10 — current cycle\n\nThe umbrella roadmap carries it.\n' > "$repo/docs-site/src/content/docs/about/roadmap.md"
run_gate "$repo" 1.10.0 --skip-pykrete-tests
extra=ok
grep -q "ROADMAP-HEADER-DRIFT" "$repo/stderr" && extra=cross_file_shipped_should_satisfy_b
assert "RG5: current 'Shipped in' section in any roadmap doc satisfies (b)" 0 "$LAST_RC" "$extra"
rm -rf "$repo"

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
