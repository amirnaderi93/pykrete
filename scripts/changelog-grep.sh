#!/usr/bin/env bash
# v1.8 PR-A2 — CHANGELOG-binary string drift CI gate.
# Closes v1.7 retro rule 6 (v1.7 PR-F's CHANGELOG quoted a warning text
# different from the actual main.rs:660 emission; PR-G patched after the
# fact). The gate extracts fenced code blocks labeled stderr/stdout/text
# from CHANGELOG.md and grep-anchors each non-empty line to
# crates/pykrete/src/. Any drift fails CI loudly.
#
# Scope per v1.8-spec.md §2.2: fenced blocks only, three labels only.
# README and inline backticks are out of scope this cycle.

set -u

CHANGELOG="${CHANGELOG:-CHANGELOG.md}"
SRC_DIR="${SRC_DIR:-crates/pykrete/src}"

if [ ! -f "$CHANGELOG" ]; then
    echo "changelog-grep: CHANGELOG file not found at '$CHANGELOG'" >&2
    exit 2
fi

if [ ! -d "$SRC_DIR" ]; then
    echo "changelog-grep: source directory not found at '$SRC_DIR'" >&2
    exit 2
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

# Multiline regex per v1.8-spec.md §2.2 committee r2:
#     /^```(stderr|stdout|text)?\n([\s\S]*?)^```/m
# Spec keeps the label optional; the script narrows to the three allow-
# listed labels for the gate so unlabeled fences don't get checked (a
# fenced unlabeled block is typically a code snippet, not binary output).
pattern = re.compile(r"^```(stderr|stdout|text)\n([\s\S]*?)^```", re.MULTILINE)

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
    echo "changelog-grep: no fenced stderr/stdout/text blocks in $CHANGELOG (0 candidates; gate is a no-op until binary-output is quoted)."
    exit 0
fi

total_lines=0
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
echo "changelog-grep: checked $total_blocks block(s) across $total_lines line(s) against $SRC_DIR/."

if [ "$fail" -ne 0 ]; then
    echo "changelog-grep: CHANGELOG-binary string drift detected. See v1.7 retro rule 6 / v1.8-spec.md §2.2." >&2
    exit 1
fi

exit 0
