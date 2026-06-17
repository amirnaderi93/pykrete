#!/usr/bin/env bash
# v1.11 PR-A1 — trust-claim sweep checklist.
#
# Sibling to scripts/changelog-grep.sh. Where changelog-grep verifies
# CHANGELOG.md's numeric claims against the live extract (docs-vs-source),
# this gate verifies that the docs surfaces (README, docs-site, VS Code
# extension surfaces, pykrete-tests README) don't still carry the PRIOR
# release's numbers (docs-vs-history). Closes v1.10 retro rules 1 + 7 and
# the 5-cycle PR-F-miscount pattern (v1.6 / v1.7 / v1.8 / v1.9 / v1.10).
#
# PR-F dev's flow:
#   1. Update Cargo.toml + extension package.json to vN.M.0.
#   2. Sweep README / docs-site / VS Code surfaces from vN.(M-1) numbers
#      to live vN.M numbers.
#   3. Run this gate. It re-reads Cargo.toml (vN.M), parses CHANGELOG.md
#      for v(N.M-1)'s `text-numeric-historical` block, and greps the
#      surface set for any prior-release number left behind.
#   4. Exit 0 = sweep clean; open PR-F. Exit 1 = drift; fix and re-run.

set -u

# --- version detection ---------------------------------------------------

CURRENT_VERSION=""
SKIP_PYKRETE_TESTS=0
CHANGELOG="${CHANGELOG:-CHANGELOG.md}"
REPO_ROOT="${REPO_ROOT:-.}"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --current-version=*)
            CURRENT_VERSION="${1#--current-version=}"
            ;;
        --current-version)
            shift
            CURRENT_VERSION="${1:-}"
            ;;
        --skip-pykrete-tests)
            SKIP_PYKRETE_TESTS=1
            ;;
        -h|--help)
            sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "trust-claim-sweep: unknown argument '$1'" >&2
            exit 2
            ;;
    esac
    shift
done

# Cargo.toml is the canonical source for "what release is shipping" —
# it's what crates.io / cargo install sees. The marker file is a
# cycle-coordination signal for the extension-version-guard workflow,
# not for this gate. Explicit --current-version overrides both.
if [ -z "$CURRENT_VERSION" ]; then
    if [ -f "$REPO_ROOT/Cargo.toml" ]; then
        CURRENT_VERSION=$(grep -E '^version[[:space:]]*=' "$REPO_ROOT/Cargo.toml" | head -1 | sed -E 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')
    fi
fi
if [ -z "$CURRENT_VERSION" ] && [ -f "$REPO_ROOT/.github/centralized-bump-cycle.marker" ]; then
    # Marker is `vN.M`; strip the leading `v` and append `.0` if no patch.
    raw=$(tr -d '[:space:]' < "$REPO_ROOT/.github/centralized-bump-cycle.marker")
    CURRENT_VERSION="${raw#v}"
    case "$CURRENT_VERSION" in
        *.*.*) ;;
        *)     CURRENT_VERSION="${CURRENT_VERSION}.0" ;;
    esac
fi

if [ -z "$CURRENT_VERSION" ]; then
    echo "trust-claim-sweep: could not determine current version (no --current-version, no Cargo.toml, no marker)" >&2
    exit 2
fi

# Split into major.minor.patch.
CURRENT_MAJOR=$(printf '%s' "$CURRENT_VERSION" | cut -d. -f1)
CURRENT_MINOR=$(printf '%s' "$CURRENT_VERSION" | cut -d. -f2)
if [ -z "$CURRENT_MAJOR" ] || [ -z "$CURRENT_MINOR" ]; then
    echo "trust-claim-sweep: malformed current version '$CURRENT_VERSION' (expected MAJOR.MINOR.PATCH)" >&2
    exit 2
fi

PRIOR_MINOR=$((CURRENT_MINOR - 1))
PRIOR_VERSION="$CURRENT_MAJOR.$PRIOR_MINOR.0"

if [ "$PRIOR_MINOR" -lt 0 ]; then
    echo "trust-claim-sweep: current version $CURRENT_VERSION has no prior minor on the v$CURRENT_MAJOR line; nothing to sweep." >&2
    exit 0
fi

# --- extract prior-release pinned numbers from CHANGELOG -----------------

if [ ! -f "$REPO_ROOT/$CHANGELOG" ]; then
    echo "trust-claim-sweep: CHANGELOG file not found at '$REPO_ROOT/$CHANGELOG'" >&2
    exit 2
fi

PRIOR_NUMBERS=$(python3 - "$REPO_ROOT/$CHANGELOG" "$PRIOR_VERSION" "$CURRENT_VERSION" <<'PY'
import re
import sys

path = sys.argv[1]
prior = sys.argv[2]
current = sys.argv[3]
with open(path, "r", encoding="utf-8") as f:
    text = f.read()

def pins_in(version):
    section_pat = re.compile(
        r"^## \[" + re.escape(version) + r"\][^\n]*\n([\s\S]*?)(?=^## \[|\Z)",
        re.MULTILINE,
    )
    m = section_pat.search(text)
    if not m:
        return {}
    section = m.group(1)
    block_pat = re.compile(
        r"^```text-numeric-historical\n([\s\S]*?)^```",
        re.MULTILINE,
    )
    out = {}
    allowed = ("probes", "positive", "negative", "fixtures", "tests", "donors")
    for bm in block_pat.finditer(section):
        for line in bm.group(1).splitlines():
            line = line.strip()
            if not line:
                continue
            parts = line.split(None, 1)
            if len(parts) < 2:
                continue
            num, key = parts[0], parts[1].split()[0]
            if not num.isdigit() or key not in allowed:
                continue
            out[key] = num
    return out

prior_pins = pins_in(prior)
current_pins = pins_in(current)

# Emit only the pairs whose number differs from the current pin (or has
# no current counterpart). Pins that didn't change between releases are
# not "prior leaks" — they're still the live current number.
for key, num in prior_pins.items():
    if current_pins.get(key) == num:
        continue
    print(f"{num} {key}")
PY
)

if [ -z "$PRIOR_NUMBERS" ]; then
    echo "trust-claim-sweep: no prior-release pins found for v$PRIOR_VERSION in $CHANGELOG; skipping (nothing to compare against)."
    exit 0
fi

# --- assemble surface set ------------------------------------------------

SURFACES=()

add_surface() {
    if [ -f "$REPO_ROOT/$1" ]; then
        SURFACES+=("$1")
    fi
}

add_surface "README.md"
add_surface "CHANGELOG.md"
add_surface "editors/vscode/CHANGELOG.md"
add_surface "editors/vscode/README.md"

if [ -d "$REPO_ROOT/docs-site/src/content" ]; then
    while IFS= read -r f; do
        rel="${f#$REPO_ROOT/}"
        SURFACES+=("$rel")
    done < <(find "$REPO_ROOT/docs-site/src/content" -type f \( -name '*.md' -o -name '*.mdx' \) | sort)
fi

# Sibling pykrete-tests checkout. The README isn't always present (release-
# gate CI checks it out; daily PR CI doesn't).
PYKRETE_TESTS_README="$REPO_ROOT/../pykrete-tests/README.md"
if [ "$SKIP_PYKRETE_TESTS" = "1" ]; then
    :
elif [ -f "$PYKRETE_TESTS_README" ]; then
    SURFACES+=("../pykrete-tests/README.md")
else
    echo "trust-claim-sweep: pykrete-tests sibling not present at $PYKRETE_TESTS_README; skipping that surface. Pass --skip-pykrete-tests to silence." >&2
fi

# --- scan each surface for prior-release-number leaks --------------------

# Carve-outs (per spec §2):
#   1. CHANGELOG.md sections labeled `## [v1.X.0]` where X <= prior minor —
#      these are historical sections and are expected to carry historical
#      numbers verbatim.
#   2. Anything wrapped in single-backtick spans (escape hatch, consistent
#      with v1.10 PR-A2 gate v3).
#   3. Anything inside a fenced block labeled `text-numeric-historical`
#      (release-pinned blocks whose numbers are immutable by design).
#
# The number-as-D-code carve-out from v1.10 PR-A2 (regex prefix anchor
# `(?<![A-Za-z0-9_])`) carries forward so `D0091` doesn't match `0091` as
# a prior-release number.

fail=0
total_scanned=0
total_leaks=0

while IFS= read -r line; do
    [ -z "$line" ] && continue
    PRIOR_NUMBERS_ENV="$PRIOR_NUMBERS" \
    CURRENT_VERSION_ENV="$CURRENT_VERSION" \
    PRIOR_VERSION_ENV="$PRIOR_VERSION" \
    PRIOR_MINOR_ENV="$PRIOR_MINOR" \
    CURRENT_MAJOR_ENV="$CURRENT_MAJOR" \
    SURFACE_PATH="$REPO_ROOT/$line" \
    SURFACE_DISPLAY="$line" \
    python3 - <<'PY'
import os
import re
import sys

surface_path = os.environ["SURFACE_PATH"]
surface_display = os.environ["SURFACE_DISPLAY"]
prior_pairs = []
for ln in os.environ["PRIOR_NUMBERS_ENV"].splitlines():
    ln = ln.strip()
    if not ln:
        continue
    parts = ln.split(None, 1)
    if len(parts) == 2:
        prior_pairs.append((parts[0], parts[1]))
current_version = os.environ["CURRENT_VERSION_ENV"]
prior_version = os.environ["PRIOR_VERSION_ENV"]
prior_minor = int(os.environ["PRIOR_MINOR_ENV"])
current_major = os.environ["CURRENT_MAJOR_ENV"]

try:
    with open(surface_path, "r", encoding="utf-8") as f:
        text = f.read()
except OSError as exc:
    print(f"trust-claim-sweep: could not read {surface_display}: {exc}", file=sys.stderr)
    sys.exit(2)

# Mask out carve-out regions before scanning. We replace masked content
# with newline-preserving blanks so line numbers stay aligned.

def blank_keep_lines(match):
    return "\n" * match.group(0).count("\n")

def sentinel_mask(match):
    raw = match.group(0)
    return "".join(c if c == "\n" else "_" for c in raw)

# Carve-out 3: `text-numeric-historical` fenced blocks (immutable pins).
hist_fence = re.compile(
    r"^```text-numeric-historical\n[\s\S]*?^```",
    re.MULTILINE,
)
text = hist_fence.sub(blank_keep_lines, text)

# Carve-out 1: historical CHANGELOG sections. Applies to any file ending
# in CHANGELOG.md. The first level-2 header (`## ...`) is the current
# section; everything from the SECOND level-2 header onwards is
# historical and masked out. This uniformly handles the root
# CHANGELOG.md (Unreleased + most-recent) and editors/vscode/CHANGELOG.md
# (no Unreleased; just versioned sections).
if surface_display.endswith("CHANGELOG.md"):
    header_pat = re.compile(r"^## [^\n]*$", re.MULTILINE)
    headers = list(header_pat.finditer(text))
    if len(headers) >= 2:
        start = headers[1].start()
        masked_chars = list(text)
        for j in range(start, len(text)):
            if masked_chars[j] != "\n":
                masked_chars[j] = "_"
        text = "".join(masked_chars)

# Carve-out 2: single-backtick spans. Same shape as v1.10 PR-A2 gate v3.
# Built via chr(96) so a stray bash pre-parser sees no unbalanced literal
# backticks if this script is ever embedded in a heredoc.
_BT = chr(96)
backtick_span = re.compile(_BT + r"[^" + _BT + r"\n]+" + _BT)
text = backtick_span.sub(sentinel_mask, text)

# Scan for each prior `<number> <key>` pair. Regex anchor
# `(?<![A-Za-z0-9_])` (from v1.10 PR-A2) so digits glued to identifiers
# (e.g. `D0091` for the digit `0091`) don't false-match.
leaks_found = 0
for num, key in prior_pairs:
    pat = re.compile(
        r"(?<![A-Za-z0-9_])" + re.escape(num) + r"\s+" + re.escape(key) + r"\b"
    )
    for m in pat.finditer(text):
        line_no = text.count("\n", 0, m.start()) + 1
        print(
            f"PRIOR-RELEASE-NUMBER-LEAKED: {surface_display}:{line_no}: "
            f"'{num} {key}' is v{prior_version}'s number, not v{current_version}'s",
            file=sys.stderr,
        )
        leaks_found += 1

sys.exit(1 if leaks_found > 0 else 0)
PY
    rc=$?
    total_scanned=$((total_scanned + 1))
    if [ "$rc" -eq 1 ]; then
        fail=1
    elif [ "$rc" -ne 0 ]; then
        echo "trust-claim-sweep: scanner aborted on $line (exit $rc)" >&2
        fail=1
    fi
done <<EOF
$(printf '%s\n' "${SURFACES[@]}")
EOF

prior_count=$(printf '%s\n' "$PRIOR_NUMBERS" | grep -c '^.' || true)
echo "trust-claim-sweep: scanned $total_scanned surface(s) against $prior_count prior-release pin(s) for v$PRIOR_VERSION (current v$CURRENT_VERSION)."

if [ "$fail" -ne 0 ]; then
    echo "trust-claim-sweep: prior-release numbers leaked into trust-claim surfaces. Wrap intentionally-historical mentions in single-backticks, OR update the surface to the live v$CURRENT_VERSION numbers. Closes v1.6 / v1.7 / v1.8 / v1.9 / v1.10 PR-F-miscount pattern (v1.10 retro rules 1 + 7)." >&2
    exit 1
fi

exit 0
