#!/usr/bin/env bash
# v1.11 PR-A1 — trust-claim sweep checklist.
# v1.13 PR-A1 — backtick-preservation tripwire (--snapshot + tripwire).
#
# Sibling to scripts/changelog-grep.sh. Where changelog-grep verifies
# CHANGELOG.md's numeric claims against the live extract (docs-vs-source),
# this gate verifies that the docs surfaces (README, docs-site, VS Code
# extension surfaces, pykrete-tests README) don't still carry the PRIOR
# release's numbers (docs-vs-history). Closes v1.10 retro rules 1 + 7 and
# the 5-cycle PR-F-miscount pattern (v1.6 / v1.7 / v1.8 / v1.9 / v1.10).
#
# v1.13 PR-A1 layers a second check on top: a backtick-preservation
# tripwire that reads scripts/trust-claim-sweep-checklist.snapshot.txt
# and fails if any pin that was single-backticked at snapshot time
# (e.g., `261 probes` in pykrete-tests/README.md) has been unwrapped on
# the current revision. Closes the 2-cycle PR-G regression at v1.11 /
# v1.12 where a docs-sync auditor closure PR stripped backticks on a
# historical batch count and the cycle paid a re-write round.
#
# PR-F dev's flow:
#   1. Update Cargo.toml + extension package.json to vN.M.0.
#   2. Sweep README / docs-site / VS Code surfaces from vN.(M-1) numbers
#      to live vN.M numbers.
#   3. Run this gate. It re-reads Cargo.toml (vN.M), parses CHANGELOG.md
#      for v(N.M-1)'s `text-numeric-historical` block, and greps the
#      surface set for any prior-release number left behind. Then runs
#      the backtick-preservation tripwire against the snapshot file.
#   4. Exit 0 = sweep + tripwire clean; open PR-F. Exit 1 = drift; fix
#      and re-run.
#   5. Run `--snapshot` to refresh the snapshot when the cycle's new
#      historical pins are committed (per v1.13-spec §2.1.1).

set -u

# --- version detection ---------------------------------------------------

CURRENT_VERSION=""
SKIP_PYKRETE_TESTS=0
CHANGELOG="${CHANGELOG:-CHANGELOG.md}"
REPO_ROOT="${REPO_ROOT:-.}"
SNAPSHOT_MODE=0
SNAPSHOT_FILE="${SNAPSHOT_FILE:-scripts/trust-claim-sweep-checklist.snapshot.txt}"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --current-version=*)
            CURRENT_VERSION="${1#--current-version=}"
            ;;
        --current-version)
            shift
            # Sibling-flag-stealing guard: a space-separated --current-version
            # with no following value (or a flag-shaped value like
            # --skip-pykrete-tests) must not silently swallow the next argv
            # under `set -u`. Reject explicitly so callers see an error
            # instead of an empty-prior false-clean (v1.10 PR-V1 R2 pattern).
            next="${1:-}"
            if [ -z "$next" ] || [ "${next#--}" != "$next" ] || { [ "${next#-}" != "$next" ] && [ "$next" != "-" ]; }; then
                echo "trust-claim-sweep: --current-version requires a version value (e.g., 1.11.0); got '$next'" >&2
                exit 2
            fi
            CURRENT_VERSION="$next"
            ;;
        --skip-pykrete-tests)
            SKIP_PYKRETE_TESTS=1
            ;;
        --snapshot)
            SNAPSHOT_MODE=1
            ;;
        -h|--help)
            cat <<'USAGE'
Usage: trust-claim-sweep-checklist.sh [OPTIONS]

Verifies that docs surfaces (README, docs-site, VS Code extension, sibling
pykrete-tests README) don't still carry the PRIOR release's pinned numbers
from CHANGELOG.md. Run by PR-F dev after sweeping vN.(M-1) -> vN.M numbers.

Also runs a backtick-preservation tripwire (v1.13 PR-A1): every
non-`--snapshot` invocation re-reads the snapshot file and fails if any
single-backticked historical pin (e.g., `` `261 probes` ``) recorded at
snapshot time has been unwrapped on the current revision. Closes the
2-cycle PR-G regression at v1.11 / v1.12.

Options:
  --current-version=X.Y.Z   Override version detection (PR-F passes this).
                            Also accepts the space form: --current-version X.Y.Z.
  --skip-pykrete-tests      Skip the sibling pykrete-tests/README.md surface
                            (used by daily PR CI; release-gate CI doesn't skip).
  --snapshot                Re-derive the backtick-preservation snapshot from
                            the current surface set and write it to
                            SNAPSHOT_FILE; skip both the prior-release-number
                            sweep AND the tripwire check.
  -h, --help                Show this message.

Environment overrides:
  REPO_ROOT                 Repo root to scan (default: .).
  CHANGELOG                 CHANGELOG path relative to REPO_ROOT (default: CHANGELOG.md).
  SNAPSHOT_FILE             Snapshot path relative to REPO_ROOT (default:
                            scripts/trust-claim-sweep-checklist.snapshot.txt).

Exit codes:
  0   sweep clean (or no prior pins to compare against; or --snapshot succeeded).
  1   one or more PRIOR-RELEASE-NUMBER-LEAKED or BACKTICK-PRESERVATION-FAIL
      lines emitted to stderr.
  2   misuse (bad flag, malformed version, missing CHANGELOG, etc.).
USAGE
            exit 0
            ;;
        *)
            echo "trust-claim-sweep: unknown argument '$1'" >&2
            exit 2
            ;;
    esac
    shift
done

# --- shared surface assembly --------------------------------------------
#
# Used by both the prior-leak sweep AND by --snapshot / tripwire. Populates
# SURFACES with repo-relative paths (so leak / tripwire messages cite the
# user-readable path, not REPO_ROOT-rooted absolutes).
SURFACES=()

assemble_surfaces() {
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
    fi
}

# --- --snapshot early branch --------------------------------------------
#
# Snapshot mode is independent of CHANGELOG / version state — it just
# enumerates the current single-backticked `<digits> <key>` pins across
# the surface set and rewrites SNAPSHOT_FILE. Cycle-close PR-F runs this
# after sweeping vN.(M-1) → vN.M numbers; the diff is reviewable.
if [ "$SNAPSHOT_MODE" = "1" ]; then
    # SNAPSHOT_SURFACES is a narrow subset of the full surface set: only
    # the files where the 2-cycle PR-G regression actually happens (a
    # backtick-strip in a historical migration-status / migration-batch
    # row). Restricting the snapshot to these surfaces avoids snapshot-
    # churn from CHANGELOG-current-section edits on every PR-F, which
    # would defeat the tripwire's purpose. The CHANGELOG's historical
    # sections are already protected by the existing header-section
    # carve-out in the prior-leak sweep above; the tripwire is the
    # second line of defense on the pykrete-tests README + vscode
    # CHANGELOG, which carry no analogous masking.
    SNAPSHOT_SURFACES=()
    PYKRETE_TESTS_README="$REPO_ROOT/../pykrete-tests/README.md"
    if [ "$SKIP_PYKRETE_TESTS" != "1" ] && [ -f "$PYKRETE_TESTS_README" ]; then
        SNAPSHOT_SURFACES+=("../pykrete-tests/README.md")
    fi
    if [ -f "$REPO_ROOT/editors/vscode/CHANGELOG.md" ]; then
        SNAPSHOT_SURFACES+=("editors/vscode/CHANGELOG.md")
    fi

    SNAPSHOT_PATH="$REPO_ROOT/$SNAPSHOT_FILE"
    mkdir -p "$(dirname "$SNAPSHOT_PATH")"
    : > "$SNAPSHOT_PATH"
    for s in "${SNAPSHOT_SURFACES[@]:-}"; do
        [ -z "$s" ] && continue
        # `<digits> <key>` between matched single backticks. Same vocabulary
        # as the CHANGELOG `text-numeric-historical` block keys so a
        # snapshot of N is exactly the set of "historical batch counts a
        # PR-G author might unwrap by accident" — the 2-cycle regression
        # surface.
        python3 - "$REPO_ROOT/$s" "$s" <<'PY' >> "$SNAPSHOT_PATH"
import re
import sys

surface_path = sys.argv[1]
surface_display = sys.argv[2]
try:
    with open(surface_path, "r", encoding="utf-8") as f:
        text = f.read()
except OSError:
    sys.exit(0)

bt = chr(96)
# `<digits> <key>` with key in the same vocabulary as the
# text-numeric-historical block keys. De-dup per surface so a pin
# appearing N times in the file produces ONE snapshot line.
pat = re.compile(
    bt + r"(\d+ (?:probes|positive|negative|fixtures|tests|donors))" + bt
)
seen = set()
for m in pat.finditer(text):
    pin = m.group(1)
    if pin in seen:
        continue
    seen.add(pin)
    print(f"{surface_display}:{bt}{pin}{bt}")
PY
    done
    # Stable order across runs: sort the snapshot so a re-run with no
    # surface changes produces a byte-identical file (clean PR diffs).
    sort -o "$SNAPSHOT_PATH" "$SNAPSHOT_PATH"
    line_count=$(grep -c '^.' "$SNAPSHOT_PATH" || true)
    echo "trust-claim-sweep: wrote $line_count snapshot pin(s) to $SNAPSHOT_FILE."
    exit 0
fi

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

# Numeric well-formedness: must be X.Y.Z (digits only). Catches both
# `--current-version=invalid` and `--current-version=1.11` (2-part).
# Without this, the arithmetic below (CURRENT_MINOR - 1) blows up under
# `set -u` and the script silently exits 0 via the empty-prior path.
if ! [[ "$CURRENT_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "trust-claim-sweep: --current-version must be X.Y.Z (got '$CURRENT_VERSION')" >&2
    exit 2
fi

# Split into major.minor.patch.
CURRENT_MAJOR=$(printf '%s' "$CURRENT_VERSION" | cut -d. -f1)
CURRENT_MINOR=$(printf '%s' "$CURRENT_VERSION" | cut -d. -f2)

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

# Parser crash detection: if the Python parser raises, command-substitution
# would set PRIOR_NUMBERS="" and the [ -z ] check below would treat it as
# "no prior pins" and exit 0 (silent false-clean). Capture the exit code
# explicitly via a tmpfile so a crash is fail-loud.
_TC_PINS_TMP=$(mktemp 2>/dev/null || mktemp -t trust-claim-sweep)
trap 'rm -f "$_TC_PINS_TMP"' EXIT

python3 - "$REPO_ROOT/$CHANGELOG" "$PRIOR_VERSION" "$CURRENT_VERSION" > "$_TC_PINS_TMP" <<'PY'
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
        r"^```text-numeric(?:-historical)?\n([\s\S]*?)^```",
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
_TC_PINS_RC=$?
if [ "$_TC_PINS_RC" -ne 0 ]; then
    echo "trust-claim-sweep: CHANGELOG parser failed (exit $_TC_PINS_RC) for '$REPO_ROOT/$CHANGELOG'" >&2
    exit 2
fi
PRIOR_NUMBERS=$(cat "$_TC_PINS_TMP")
SKIP_PRIOR_SWEEP=0

if [ -z "$PRIOR_NUMBERS" ]; then
    echo "trust-claim-sweep: no prior-release pins found for v$PRIOR_VERSION in $CHANGELOG; skipping (nothing to compare against)."
    # Tripwire still runs below — empty prior pins is independent of the
    # backtick-preservation snapshot, which has its own state.
    SKIP_PRIOR_SWEEP=1
fi

# --- assemble surface set ------------------------------------------------
#
# Surface helper is defined above (shared with --snapshot mode). The
# absent-sibling warning is emitted here (non-snapshot path only) so the
# snapshot run stays quiet on PR-F's clean rewrites.
assemble_surfaces
PYKRETE_TESTS_README="$REPO_ROOT/../pykrete-tests/README.md"
if [ "$SKIP_PYKRETE_TESTS" != "1" ] && [ ! -f "$PYKRETE_TESTS_README" ]; then
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

if [ "$SKIP_PRIOR_SWEEP" = "1" ]; then
    # Short-circuit the per-surface scan loop; tripwire still runs.
    SURFACES=()
fi

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
#
# NOTE: docs-site tables with version-row layouts (e.g. a "Releases" page
# listing `1.9.0 | 255 probes` rows) must use backtick-wrap or a
# `text-numeric-historical` fenced block to flag the row content as
# intentionally-historical. The header-second-onward mask only fires on
# files named `CHANGELOG.md`; docs-site `*.md` / `*.mdx` get no implicit
# historical carve-out.
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
$(printf '%s\n' "${SURFACES[@]:-}")
EOF

if [ "$SKIP_PRIOR_SWEEP" != "1" ]; then
    prior_count=$(printf '%s\n' "$PRIOR_NUMBERS" | grep -c '^.' || true)
    echo "trust-claim-sweep: scanned $total_scanned surface(s) against $prior_count prior-release pin(s) for v$PRIOR_VERSION (current v$CURRENT_VERSION)."
fi

if [ "$fail" -ne 0 ]; then
    echo "trust-claim-sweep: prior-release numbers leaked into trust-claim surfaces. Wrap intentionally-historical mentions in single-backticks, OR update the surface to the live v$CURRENT_VERSION numbers. Closes v1.6 / v1.7 / v1.8 / v1.9 / v1.10 PR-F-miscount pattern (v1.10 retro rules 1 + 7)." >&2
fi

# --- backtick-preservation tripwire (v1.13 PR-A1) -----------------------
#
# Reads SNAPSHOT_FILE; for each `surface:`<pin>`` line, checks the surface
# still contains the EXACT backticked byte sequence. A snapshotted pin
# whose backticks were stripped (the 2-cycle PR-G regression at v1.11 /
# v1.12) fails the gate. Snapshot is additive: new backticked pins NOT
# yet in the snapshot do NOT fire the tripwire (PR-F's --snapshot
# refresh sweeps them in at cycle close).
#
# Degraded modes (exit 0, warn on stderr):
# - SNAPSHOT_FILE missing → tripwire inactive (cold start / fresh repo).
# - SNAPSHOT_FILE empty → tripwire inactive (nothing pinned yet).
tripwire_fail=0
SNAPSHOT_PATH="$REPO_ROOT/$SNAPSHOT_FILE"
if [ ! -f "$SNAPSHOT_PATH" ]; then
    echo "trust-claim-sweep: backtick-preservation snapshot not found at $SNAPSHOT_FILE; tripwire skipped. Run with --snapshot to seed it." >&2
elif [ ! -s "$SNAPSHOT_PATH" ]; then
    echo "trust-claim-sweep: backtick-preservation snapshot at $SNAPSHOT_FILE is empty; tripwire skipped." >&2
else
    tripwire_total=0
    tripwire_missing=0
    while IFS= read -r snap_line; do
        [ -z "$snap_line" ] && continue
        # Lines look like: pykrete-tests/README.md:`261 probes`
        # Parse: SURFACE = up to first ':'; PIN = the rest (includes backticks).
        snap_surface="${snap_line%%:*}"
        snap_pin="${snap_line#*:}"
        if [ -z "$snap_surface" ] || [ -z "$snap_pin" ] || [ "$snap_surface" = "$snap_line" ]; then
            echo "trust-claim-sweep: malformed snapshot line: '$snap_line' (expected '<surface>:\`<pin>\`')" >&2
            tripwire_fail=1
            continue
        fi
        tripwire_total=$((tripwire_total + 1))
        snap_path="$REPO_ROOT/$snap_surface"
        if [ ! -f "$snap_path" ]; then
            # Skip-pykrete-tests path or surface deleted in-tree. Honor
            # the same skip semantics as the prior-leak sweep so daily
            # PR CI (no sibling checkout) doesn't trip on the
            # pykrete-tests entries.
            if [ "$SKIP_PYKRETE_TESTS" = "1" ] && [ "${snap_surface#../pykrete-tests/}" != "$snap_surface" ]; then
                continue
            fi
            echo "BACKTICK-PRESERVATION-FAIL: $snap_surface: snapshotted surface not present in tree." >&2
            tripwire_missing=$((tripwire_missing + 1))
            tripwire_fail=1
            continue
        fi
        # Fixed-string grep: the snapshot stores the EXACT byte sequence
        # we want to preserve (including the surrounding backticks).
        if ! grep -qF -- "$snap_pin" "$snap_path"; then
            echo "BACKTICK-PRESERVATION-FAIL: $snap_surface: '$snap_pin' was single-backticked at snapshot time but is not present on the current revision. Restore the backticks or refresh the snapshot with --snapshot." >&2
            tripwire_missing=$((tripwire_missing + 1))
            tripwire_fail=1
        fi
    done < "$SNAPSHOT_PATH"
    echo "trust-claim-sweep: backtick-preservation tripwire scanned $tripwire_total snapshot pin(s); $tripwire_missing missing."
    if [ "$tripwire_fail" -ne 0 ]; then
        echo "trust-claim-sweep: backtick-preservation tripwire fired. Restore the backticks, OR refresh the snapshot with: bash scripts/trust-claim-sweep-checklist.sh --snapshot. Closes v1.11 / v1.12 PR-G regression class (v1.12 retro)." >&2
    fi
fi

if [ "$fail" -ne 0 ] || [ "$tripwire_fail" -ne 0 ]; then
    exit 1
fi

exit 0
