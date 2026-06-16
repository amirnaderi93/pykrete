#!/usr/bin/env bash
# v1.8 PR-A2 — CHANGELOG-binary string drift CI gate.
# v1.9 PR-A2 — extended with `text-numeric` blocks (live-extract numeric trust-claim verification).
#
# Closes v1.7 retro rule 6 (v1.7 PR-F's CHANGELOG quoted a warning text
# different from the actual main.rs:660 emission; PR-G patched after the
# fact) and v1.8 retro rule 7 (v1.8 PR-F's "106 fixtures" claim drifted
# from the live 112; architecture audit caught it after the fact).
#
# v1 gate: extracts fenced code blocks labeled stderr/stdout/text from
# CHANGELOG.md and grep-anchors each non-empty line to crates/pykrete/src/.
#
# v2 gate (this script, v1.9): adds the `text-numeric` label. Blocks marked
# `text-numeric` contain one `<number> <key>` line per claim. Each key maps
# to a known live-extract command; the script runs the command and compares
# the output to the claimed number. Drift fails CI loudly.
#
# Scope per v1.9-spec.md §2.2: fenced blocks only, four labels
# (stderr/stdout/text/text-numeric). README and inline backticks are still
# out of scope.

set -u

CHANGELOG="${CHANGELOG:-CHANGELOG.md}"
SRC_DIR="${SRC_DIR:-crates/pykrete/src}"
# Skip the live-extract step (text-numeric blocks). Useful on PR CI where
# pykrete-tests / a release-mode test build isn't available. Set via env
# var OR --skip-live-extract flag. Block syntax + key validity are still
# checked; only the command execution + compare is skipped.
SKIP_LIVE_EXTRACT="${PYKRETE_SKIP_LIVE_EXTRACT:-0}"
for arg in "$@"; do
    case "$arg" in
        --skip-live-extract) SKIP_LIVE_EXTRACT=1 ;;
    esac
done

if [ ! -f "$CHANGELOG" ]; then
    echo "changelog-grep: CHANGELOG file not found at '$CHANGELOG'" >&2
    exit 2
fi

if [ ! -d "$SRC_DIR" ]; then
    echo "changelog-grep: source directory not found at '$SRC_DIR'" >&2
    exit 2
fi

# Known numeric-claim table (v1.9 PR-A2). Maps a claim key to a shell
# pipeline that prints the live value to stdout. New keys MUST be added in
# the same PR that adds a CHANGELOG `text-numeric` line using them — see
# CONTRIBUTING.md "CHANGELOG conventions" + v1.9-spec §2.2.
#
# Bash 3.2 (default on macOS) lacks `declare -A`, so this is a case dispatch
# instead of an associative array.
numeric_claim_command() {
    case "$1" in
        probes)
            echo "python3 ../pykrete-tests/scripts/probes.py extract ../pykrete-tests/cross-codebase | jq '.probes | length'"
            ;;
        fixtures)
            echo "find ../pykrete-tests/cross-codebase \\( -path '*annotated*' -name '*.pyk' -o -path '*probes_negative*' -name '*.pyk' \\) | wc -l | tr -d ' '"
            ;;
        tests)
            echo "cargo test --release --workspace 2>&1 | grep -oE '[0-9]+ passed' | awk '{s+=\$1} END {print s}'"
            ;;
        donors)
            echo "find ../pykrete-tests/cross-codebase -maxdepth 1 -mindepth 1 -type d | wc -l | tr -d ' '"
            ;;
        *)
            return 1
            ;;
    esac
}

# Test hook: lets the self-test suite override the claim table without
# mucking with $PATH or stubbing real commands. Sourced AFTER the default
# definition so an override file's `numeric_claim_command` replaces it.
if [ -n "${PYKRETE_NUMERIC_CLAIM_TABLE_OVERRIDE:-}" ] && [ -f "${PYKRETE_NUMERIC_CLAIM_TABLE_OVERRIDE}" ]; then
    . "${PYKRETE_NUMERIC_CLAIM_TABLE_OVERRIDE}"
fi

# Use python3 to parse the fenced blocks because portable awk varies on
# regex-capture support and base64 line-wrapping. python3 ships with both
# macOS and the CI runner (ubuntu-latest); the script is otherwise pure
# bash + grep.
parsed=$(python3 - "$CHANGELOG" <<'PY'
import re
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    text = f.read()

# Multiline regex per v1.8-spec.md §2.2 + v1.9-spec.md §2.2 committee r2:
#     /^```(stderr|stdout|text|text-numeric)\n([\s\S]*?)^```/m
# Spec keeps the label optional; the script narrows to the four allow-
# listed labels for the gate so unlabeled fences don't get checked (a
# fenced unlabeled block is typically a code snippet, not binary output).
pattern = re.compile(
    r"^```(stderr|stdout|text-numeric|text)\n([\s\S]*?)^```",
    re.MULTILINE,
)

# Compute opening-fence line numbers by counting newlines up to the match.
out_lines = []
for m in pattern.finditer(text):
    label = m.group(1)
    content = m.group(2)
    start_line = text.count("\n", 0, m.start()) + 1
    # Emit one record per non-empty content line. Tab-separated:
    # <label>\t<opening_fence_line>\t<line>
    for line in content.splitlines():
        if line.strip() == "":
            continue
        # Tabs in source would corrupt the record separator; replace with
        # \t-literal so the bash split survives. grep -F still anchors the
        # restored string; we restore tabs before grep below.
        encoded = line.replace("\t", "\\t")
        out_lines.append(f"{label}\t{start_line}\t{encoded}")

sys.stdout.write("\n".join(out_lines))
PY
)

if [ -z "$parsed" ]; then
    echo "changelog-grep: no fenced stderr/stdout/text/text-numeric blocks in $CHANGELOG (0 candidates; gate is a no-op until binary-output is quoted)."
    exit 0
fi

total_lines=0
total_numeric_lines=0
fail=0
# Bash 3.2 ships on macOS without `declare -A`; track unique block
# identifiers via a newline-delimited string instead.
seen_blocks=""

while IFS=$'\t' read -r label start_line line; do
    [ -z "$label" ] && continue
    key="$label@$start_line"
    case "
$seen_blocks
" in
        *"
$key
"*) ;;
        *) seen_blocks="$seen_blocks
$key" ;;
    esac

    if [ "$label" = "text-numeric" ]; then
        total_numeric_lines=$((total_numeric_lines + 1))
        # Expected shape: `<number> <claim-key>` (e.g. `253 probes`).
        # Trailing prose after the key is allowed (for human readability)
        # but the first whitespace-separated token must be the number and
        # the second must be the known key.
        restored="${line//\\t/$'\t'}"
        # shellcheck disable=SC2086
        set -- $restored
        if [ "$#" -lt 2 ]; then
            echo "MISMATCH: CHANGELOG.md text-numeric block (opened at line $start_line) malformed line: '$restored' (expected '<number> <key>')" >&2
            fail=1
            continue
        fi
        claimed="$1"
        claim_key="$2"
        case "$claimed" in
            ''|*[!0-9]*)
                echo "MISMATCH: CHANGELOG.md text-numeric block (opened at line $start_line) first token '$claimed' is not a positive integer (expected '<number> <key>')" >&2
                fail=1
                continue
                ;;
        esac
        if ! cmd=$(numeric_claim_command "$claim_key"); then
            echo "MISMATCH: CHANGELOG.md text-numeric block (opened at line $start_line) unknown numeric-claim key: '$claim_key'. Add it to numeric_claim_command in scripts/changelog-grep.sh in this PR (see CONTRIBUTING.md 'CHANGELOG conventions')." >&2
            fail=1
            continue
        fi
        if [ "$SKIP_LIVE_EXTRACT" = "1" ]; then
            continue
        fi
        actual=$(bash -c "$cmd" 2>/dev/null || true)
        actual=$(printf '%s' "$actual" | tr -d '[:space:]')
        if [ -z "$actual" ]; then
            echo "MISMATCH: CHANGELOG.md text-numeric '$claim_key' (opened at line $start_line) live extract produced no output. Command: $cmd" >&2
            fail=1
            continue
        fi
        if [ "$claimed" != "$actual" ]; then
            echo "MISMATCH: text-numeric '$claim_key' expected $claimed but live was $actual (CHANGELOG.md block opened at line $start_line; command: $cmd)" >&2
            fail=1
        fi
        continue
    fi

    total_lines=$((total_lines + 1))
    # Restore literal tabs (encoded \t → tab) before anchoring.
    restored="${line//\\t/$'\t'}"
    if ! grep -rFq -- "$restored" "$SRC_DIR"; then
        excerpt=$(printf '%s' "$restored" | cut -c1-80)
        echo "MISMATCH: CHANGELOG.md fenced-block '$label' (opened at line $start_line) content '$excerpt' not found in $SRC_DIR/" >&2
        fail=1
    fi
done <<EOF
$parsed
EOF

total_blocks=$(printf '%s' "$seen_blocks" | grep -c '^.' || true)
if [ "$SKIP_LIVE_EXTRACT" = "1" ] && [ "$total_numeric_lines" -gt 0 ]; then
    echo "changelog-grep: checked $total_blocks block(s) across $total_lines source-anchored line(s) against $SRC_DIR/ + $total_numeric_lines text-numeric claim(s) (live extract SKIPPED)."
else
    echo "changelog-grep: checked $total_blocks block(s) across $total_lines source-anchored line(s) against $SRC_DIR/ + $total_numeric_lines text-numeric claim(s)."
fi

if [ "$fail" -ne 0 ]; then
    echo "changelog-grep: CHANGELOG-binary string drift detected. See v1.7 retro rule 6 + v1.8 retro rule 7 / v1.9-spec.md §2.2." >&2
    exit 1
fi

exit 0
