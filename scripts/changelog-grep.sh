#!/usr/bin/env bash
# v1.8 PR-A2 — CHANGELOG-binary string drift CI gate.
# v1.9 PR-A2 — extended with `text-numeric` blocks (live-extract numeric trust-claim verification).
# v1.10 PR-A2 — extended with prose-paragraph numeric scan (gate v3) +
#               `text-numeric-historical` label (skipped by the gate, for
#               release-pinned blocks whose numbers are immutable by design).
#
# Closes v1.7 retro rule 6 (v1.7 PR-F's CHANGELOG quoted a warning text
# different from the actual main.rs:660 emission; PR-G patched after the
# fact), v1.8 retro rule 7 (v1.8 PR-F's "106 fixtures" claim drifted
# from the live 112; architecture audit caught it after the fact), and
# v1.9 retro rule 12 (v1.9 PR-F shipped "183 positive + 72 negative" in a
# prose paragraph outside any fenced block; the v2 gate missed it).
#
# v1 gate: extracts fenced code blocks labeled stderr/stdout/text from
# CHANGELOG.md and grep-anchors each non-empty line to crates/pykrete/src/.
#
# v2 gate (v1.9): adds the `text-numeric` label. Blocks marked `text-numeric`
# contain one `<number> <key>` line per claim. Each key maps to a known
# live-extract command; the script runs the command and compares the output
# to the claimed number. Drift fails CI loudly.
#
# v3 gate (this script, v1.10): scans prose paragraphs (everything OUTSIDE
# fenced blocks) for <digit> <known-key> patterns. Each match is verified
# against the same known-claim table v2 uses. Single-backtick-wrapped
# numbers (e.g. backtick-183-positive-backtick) are the editor's escape
# hatch for historical / external-context numbers and are skipped.
#
# Scope per v1.10-spec.md §2.2.2: CHANGELOG.md only; fenced blocks (four
# labels) + prose numeric mentions of known keys. README and docs-site
# prose stay out of scope.

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
            # v1.12 PR-A2 — runner-perf fix. The default extract recompiles +
            # re-executes the full release-mode test suite (~5-10 min cold,
            # ~30s warm). In release-gate.yml the dedicated `Cargo test
            # (release mode)` step already runs the same suite and writes the
            # summed `<N> passed` count to $PYKRETE_TESTS_COUNT_FILE; when
            # that file is present we read it instead of re-running cargo,
            # eliminating the ~30 min duplicated cold-cache build. Locally
            # (no env var set) the fallback runs cargo as before.
            if [ -n "${PYKRETE_TESTS_COUNT_FILE:-}" ] && [ -f "${PYKRETE_TESTS_COUNT_FILE}" ]; then
                echo "cat \"${PYKRETE_TESTS_COUNT_FILE}\""
            else
                echo "cargo test --release --workspace 2>&1 | grep -oE '[0-9]+ passed' | awk '{s+=\$1} END {print s}'"
            fi
            ;;
        donors)
            echo "find ../pykrete-tests/cross-codebase -maxdepth 1 -mindepth 1 -type d | wc -l | tr -d ' '"
            ;;
        positive)
            # Positive probes = every probe whose kind is NOT EXPECTS (which
            # is the negative-probe kind; see pykrete-tests/scripts/probes.py).
            echo "python3 ../pykrete-tests/scripts/probes.py extract ../pykrete-tests/cross-codebase | jq '[.probes[] | select(.kind != \"EXPECTS\")] | length'"
            ;;
        negative)
            echo "python3 ../pykrete-tests/scripts/probes.py extract ../pykrete-tests/cross-codebase | jq '[.probes[] | select(.kind == \"EXPECTS\")] | length'"
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

# v1.12 PR-A2 — optional per-step timing for runner-perf debugging. Set
# PYKRETE_GREP_TIMING=1 to emit wall-clock seconds per live-extract claim
# (text-numeric + prose). Default off — silent on routine runs.
GREP_TIMING="${PYKRETE_GREP_TIMING:-0}"
run_claim_cmd() {
    local key="$1"
    local cmd="$2"
    if [ "$GREP_TIMING" = "1" ]; then
        local start_ns end_ns elapsed_ms
        start_ns=$(date +%s%N 2>/dev/null || echo "")
        actual=$(bash -c "$cmd" 2>/dev/null || true)
        end_ns=$(date +%s%N 2>/dev/null || echo "")
        # BSD `date` (macOS) doesn't implement %N — it returns a literal
        # `<seconds>N`. Guard arithmetic against non-numeric values so the
        # timing block stays silent on macOS instead of erroring out.
        case "$start_ns" in *[!0-9]*) start_ns="" ;; esac
        case "$end_ns" in *[!0-9]*) end_ns="" ;; esac
        if [ -n "$start_ns" ] && [ -n "$end_ns" ]; then
            elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))
            echo "changelog-grep: timing: claim=$key elapsed_ms=$elapsed_ms" >&2
        fi
    else
        actual=$(bash -c "$cmd" 2>/dev/null || true)
    fi
}

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
#
# `text-numeric-historical` is intentionally NOT in the alternation —
# blocks with that label render in the rendered CHANGELOG but skip the
# live-extract gate. Use it for release-pinned trust-claim blocks whose
# numbers are immutable by design (e.g. the v1.9.0 section pinned at the
# v1.9.0 tag commit). The `any_fence` regex below still strips these from
# prose-scan input, so digits inside historical blocks won't trip gate v3.
pattern = re.compile(
    r"^```(stderr|stdout|text-numeric|text)\n([\s\S]*?)^```",
    re.MULTILINE,
)
# Strip ALL fenced blocks (any label, labeled or not — including
# `text-numeric-historical`) before prose scan, so prose-scan never
# doubles up on `text-numeric` lines and never trips on code-snippet
# or historical-pin digits.
any_fence = re.compile(r"^```[^\n]*\n[\s\S]*?^```", re.MULTILINE)

# v1.10 PR-A2 gate v3 — prose-paragraph numeric scan.
# Pattern from v1.10-spec.md §2.2.2 — case-sensitive on the key per spec.
# Leading `(?<![A-Za-z0-9_])` rejects digits glued to an identifier prefix
# (e.g. `0091` inside `D0091 probes`) so D-code mentions don't trip the gate.
prose_pat = re.compile(
    r"(?<![A-Za-z0-9_])(\d+)\s+(positive|negative|probes|fixtures|tests|donors)\b"
)
# Single-backtick escape hatch. The pattern matches a span between two
# single backticks (not double / triple); newlines disallowed so a stray
# unclosed backtick can't swallow the rest of the file. Built via chr(96)
# so the bash 3.2 heredoc pre-parser sees no unbalanced literal backticks.
_BT = chr(96)
backtick_span = re.compile(_BT + r"[^" + _BT + r"\n]+" + _BT)

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

# Prose scan: replace fenced blocks with newline-preserving blanks so line
# numbers stay aligned. Backtick-spans (escape hatch) are replaced with a
# non-whitespace sentinel of the same length so the prose pattern can't
# reach across them — a digit on one side of a code span and a key on the
# other isn't a semantic claim. `\s+` won't bridge the sentinel.
def blank_keep_lines(match):
    return "\n" * match.group(0).count("\n")

def sentinel_mask(match):
    raw = match.group(0)
    # Preserve embedded newlines; replace every other char with `_`.
    return "".join(c if c == "\n" else "_" for c in raw)

prose_text = any_fence.sub(blank_keep_lines, text)
prose_text = backtick_span.sub(sentinel_mask, prose_text)

for ln, line in enumerate(prose_text.splitlines(), start=1):
    for pm in prose_pat.finditer(line):
        claimed = pm.group(1)
        key = pm.group(2)
        # `prose` is the record-type marker (analogous to label for fenced
        # blocks). Encode tabs same as above for split safety.
        encoded = f"{claimed} {key}".replace("\t", "\\t")
        out_lines.append(f"prose\t{ln}\t{encoded}")

sys.stdout.write("\n".join(out_lines))
PY
)

if [ -z "$parsed" ]; then
    # Prose-scan ran and found 0 unbacktick-wrapped known-key matches (all
    # prose numeric mentions are either escape-hatched or out of allow-
    # list); no fenced blocks were present either. Distinct from "gate
    # didn't run" — the scan happened, the candidate set was empty.
    echo "changelog-grep: scanned $CHANGELOG; 0 fenced stderr/stdout/text/text-numeric blocks + 0 prose numeric claims to verify (all prose claims are backtick-escape-hatched or historical)."
    exit 0
fi

total_lines=0
total_numeric_lines=0
total_prose_lines=0
fail=0
# Bash 3.2 ships on macOS without `declare -A`; track unique block
# identifiers via a newline-delimited string instead.
seen_blocks=""

while IFS=$'\t' read -r label start_line line; do
    [ -z "$label" ] && continue
    if [ "$label" != "prose" ]; then
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
    fi

    if [ "$label" = "prose" ]; then
        total_prose_lines=$((total_prose_lines + 1))
        restored="${line//\\t/$'\t'}"
        # shellcheck disable=SC2086
        set -- $restored
        claimed="$1"
        claim_key="$2"
        # Python emitted `<digit> <known-key>`, so both fields are present
        # and `<digit>` is numeric by construction (regex enforced). Defensive
        # re-check anyway — guards against a future python-side bug.
        case "$claimed" in
            ''|*[!0-9]*)
                echo "PROSE-MISMATCH: CHANGELOG.md line $start_line first token '$claimed' is not a positive integer" >&2
                fail=1
                continue
                ;;
        esac
        if ! cmd=$(numeric_claim_command "$claim_key"); then
            # Unknown key in prose: per spec §2.2.2 ignored ("only known
            # keys are gated"). Python pre-filtered to the known-key set,
            # so this branch is reachable only if the prose pattern grows
            # ahead of the claim table — keep silent to honor spec scope.
            continue
        fi
        if [ "$SKIP_LIVE_EXTRACT" = "1" ]; then
            # Syntax + key allowlist passed; live compare deferred to the
            # release-gate workflow.
            continue
        fi
        run_claim_cmd "$claim_key" "$cmd"
        actual=$(printf '%s' "$actual" | tr -d '[:space:]')
        if [ -z "$actual" ]; then
            echo "PROSE-MISMATCH: CHANGELOG.md line $start_line live extract produced no output. Command: $cmd" >&2
            fail=1
            continue
        fi
        if [ "$claimed" != "$actual" ]; then
            echo "PROSE-MISMATCH: line $start_line: '$claimed $claim_key' vs live $actual (command: $cmd)" >&2
            fail=1
        fi
        continue
    fi

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
        run_claim_cmd "$claim_key" "$cmd"
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
suffix=""
if [ "$SKIP_LIVE_EXTRACT" = "1" ] && { [ "$total_numeric_lines" -gt 0 ] || [ "$total_prose_lines" -gt 0 ]; }; then
    suffix=" (live extract SKIPPED)"
fi
echo "changelog-grep: checked $total_blocks block(s) across $total_lines source-anchored line(s) against $SRC_DIR/ + $total_numeric_lines text-numeric claim(s) + $total_prose_lines prose numeric claim(s)$suffix."

if [ "$fail" -ne 0 ]; then
    echo "changelog-grep: CHANGELOG drift detected. See v1.7 retro rule 6 + v1.8 retro rule 7 + v1.9 retro rule 12 / v1.10-spec.md §2.2.2." >&2
    exit 1
fi

exit 0
