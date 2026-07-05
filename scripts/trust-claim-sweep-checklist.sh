#!/usr/bin/env bash
# v1.11 PR-A1 — trust-claim sweep checklist.
# v1.13 PR-A1 — backtick-preservation tripwire (--snapshot + tripwire).
# v1.14 PR-A1 — backticked-claim-stale scanner (default-mode 3rd check).
# v1.15 PR-A1 — unbackticked-marketing-table scanner (default-mode 4th check)
#               + DRY-up of the v1.14 dual assemble_*_surfaces pair into a
#               single collect_surfaces helper.
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
EXPECTED_FAILURES_FILE=""
# Args to forward to the inner (allowlist-free) re-exec. --expected-failures
# is stripped so the child runs the raw gate whose fires we then reconcile.
PASSTHROUGH_ARGS=()

while [ "$#" -gt 0 ]; do
    case "$1" in
        --current-version=*)
            CURRENT_VERSION="${1#--current-version=}"
            PASSTHROUGH_ARGS+=("$1")
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
            PASSTHROUGH_ARGS+=("--current-version" "$next")
            ;;
        --expected-failures=*)
            EXPECTED_FAILURES_FILE="${1#--expected-failures=}"
            ;;
        --expected-failures)
            shift
            next="${1:-}"
            if [ -z "$next" ] || [ "${next#--}" != "$next" ]; then
                echo "trust-claim-sweep: --expected-failures requires a file path; got '$next'" >&2
                exit 2
            fi
            EXPECTED_FAILURES_FILE="$next"
            ;;
        --skip-pykrete-tests)
            SKIP_PYKRETE_TESTS=1
            PASSTHROUGH_ARGS+=("$1")
            ;;
        --snapshot)
            SNAPSHOT_MODE=1
            PASSTHROUGH_ARGS+=("$1")
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

Also runs the backticked-claim-stale scanner (v1.14 PR-A1): every
non-`--snapshot` invocation greps tracked surfaces for backticked
`<num> <key>` patterns and fails if the pin matches NEITHER a current
text-numeric pin NOR a text-numeric-historical fenced block. Closes
the v1.13 docs-sync audit 8-blocker pattern.

Also runs the unbackticked-marketing-table scanner (v1.15 PR-A1):
every non-`--snapshot` invocation scans tracked surfaces for BARE
`<num> <key>` patterns inside markdown-table rows (lines starting with
`|` and containing more than one `|`). Same validity rule as the
backticked-claim-stale scanner; closes the v1.14 architecture-auditor
finding that bare numbers in trajectory-table cells escaped both prior
scanners.

Options:
  --current-version=X.Y.Z   Override version detection (PR-F passes this).
                            Also accepts the space form: --current-version X.Y.Z.
  --skip-pykrete-tests      Skip the sibling pykrete-tests/README.md surface
                            (used by daily PR CI; release-gate CI doesn't skip).
  --snapshot                Re-derive the backtick-preservation snapshot from
                            the current surface set and write it to
                            SNAPSHOT_FILE; skip both the prior-release-number
                            sweep AND the tripwire check.
  --expected-failures=FILE  Read an allowlist of expected-failure entries
                            (JSON). Each active entry SUPPRESSES a matching
                            gate fire (logged as EXPECTED-FAILURE-SUPPRESSED)
                            so a deferred surface can pass while flagged. Each
                            entry MUST carry an `expiresAfter` version; once
                            CURRENT_VERSION exceeds it the entry is STALE and
                            the gate fails LOUD (EXPECTED-FAILURE-EXPIRED). An
                            active entry matching no fire warns (dead entry).
                            Also accepts the space form: --expected-failures FILE.
  -h, --help                Show this message.

Environment overrides:
  REPO_ROOT                 Repo root to scan (default: .).
  CHANGELOG                 CHANGELOG path relative to REPO_ROOT (default: CHANGELOG.md).
  SNAPSHOT_FILE             Snapshot path relative to REPO_ROOT (default:
                            scripts/trust-claim-sweep-checklist.snapshot.txt).

Expected-failures JSON shape:
  { "entries": [ { "surface": "<path>", "pattern": "<substring of the fire
    line, optional>", "reason": "<why deferred>", "expiresAfter": "<X.Y.Z>" } ] }

Exit codes:
  0   sweep clean (or no prior pins to compare against; or --snapshot succeeded;
      or every gate fire was suppressed by an active allowlist entry).
  1   one or more PRIOR-RELEASE-NUMBER-LEAKED, BACKTICK-PRESERVATION-FAIL,
      BACKTICKED-CLAIM-STALE, MARKETING-TABLE-CLAIM-STALE, or ROADMAP-HEADER-DRIFT
      lines survived (unsuppressed), OR an allowlist entry is EXPIRED.
  2   misuse (bad flag, malformed version, malformed/missing allowlist, missing
      CHANGELOG, etc.).
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
# Single source of truth for the trust-claim surface inventory. Used by the
# prior-leak sweep, the --snapshot / tripwire path, the stale-claim scanner
# (v1.14 PR-A1), and the unbackticked marketing-table scanner (v1.15 PR-A1).
# v1.15 PR-A1 consolidated the v1.14 `assemble_surfaces` / `assemble_stale_
# surfaces` pair into this single helper (v1.14 PR-A1 R2 follow-up). Each
# caller passes the name of the array variable to populate.
collect_surfaces() {
    local _out_var="$1"

    _add() {
        if [ -f "$REPO_ROOT/$1" ]; then
            eval "${_out_var}+=(\"\$1\")"
        fi
    }

    _add "README.md"
    _add "CHANGELOG.md"
    _add "editors/vscode/CHANGELOG.md"
    _add "editors/vscode/README.md"
    _add "docs/roadmap.md"

    if [ -d "$REPO_ROOT/docs-site/src/content" ]; then
        while IFS= read -r f; do
            local rel="${f#$REPO_ROOT/}"
            eval "${_out_var}+=(\"\$rel\")"
        done < <(find "$REPO_ROOT/docs-site/src/content" -type f \( -name '*.md' -o -name '*.mdx' \) | sort)
    fi

    # Sibling pykrete-tests checkout. The README isn't always present (release-
    # gate CI checks it out; daily PR CI doesn't).
    local _pyk_tests_readme="$REPO_ROOT/../pykrete-tests/README.md"
    if [ "$SKIP_PYKRETE_TESTS" != "1" ] && [ -f "$_pyk_tests_readme" ]; then
        eval "${_out_var}+=(\"../pykrete-tests/README.md\")"
    fi
}

SURFACES=()

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

# Auto-discover a committed repo-level allowlist when no --expected-failures
# flag is passed, so `trust-claim-sweep-checklist.sh` (default, as CI runs it)
# honors the declared expected-failures without every caller passing the flag.
# _TC_SWEEP_INNER guards the wrapper's re-exec below from re-discovering the
# file and recursing. Explicit --expected-failures always wins.
DEFAULT_EXPECTED_FAILURES_FILE="scripts/trust-claim-sweep-expected-failures.json"
if [ -z "$EXPECTED_FAILURES_FILE" ] && [ -z "${_TC_SWEEP_INNER:-}" ] \
    && [ -f "$REPO_ROOT/$DEFAULT_EXPECTED_FAILURES_FILE" ]; then
    EXPECTED_FAILURES_FILE="$DEFAULT_EXPECTED_FAILURES_FILE"
fi

# --- expected-failures allowlist (v1.16 PR-A1) --------------------------
#
# When --expected-failures=FILE is given (or the default file is
# auto-discovered above), re-run the gate WITHOUT the flag (the inner raw
# scan), capture its stderr, and reconcile each fire line against the
# allowlist. An ACTIVE entry (CURRENT_VERSION <= expiresAfter) suppresses a
# matching fire and logs EXPECTED-FAILURE-SUPPRESSED so a surface PR-G will
# resolve can pass while flagged. An EXPIRED entry (CURRENT_VERSION >
# expiresAfter) fails the gate LOUD — the allowlist is a countdown, not a
# dumping-ground (v1.15 retro rule 2). An active entry that matches no fire
# warns (dead entry) without failing. The inner gate is unmodified, so every
# existing self-test exercises the raw path untouched.
if [ -n "$EXPECTED_FAILURES_FILE" ]; then
    _EF_PATH="$EXPECTED_FAILURES_FILE"
    if [ ! -f "$_EF_PATH" ] && [ -f "$REPO_ROOT/$_EF_PATH" ]; then
        _EF_PATH="$REPO_ROOT/$_EF_PATH"
    fi
    if [ ! -f "$_EF_PATH" ]; then
        echo "trust-claim-sweep: --expected-failures file not found: '$EXPECTED_FAILURES_FILE'" >&2
        exit 2
    fi

    _EF_ERR=$(mktemp 2>/dev/null || mktemp -t trust-claim-sweep-ef)
    trap 'rm -f "$_EF_ERR"' EXIT
    if [ "${#PASSTHROUGH_ARGS[@]}" -gt 0 ]; then
        _TC_SWEEP_INNER=1 bash "$0" "${PASSTHROUGH_ARGS[@]}" 2> "$_EF_ERR"
    else
        _TC_SWEEP_INNER=1 bash "$0" 2> "$_EF_ERR"
    fi
    _EF_INNER_RC=$?

    python3 - "$_EF_ERR" "$_EF_PATH" "$CURRENT_VERSION" "$_EF_INNER_RC" <<'PY'
import json
import sys

err_path, ef_path, current, inner_rc = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])

FIRE_PREFIXES = (
    "PRIOR-RELEASE-NUMBER-LEAKED:",
    "BACKTICK-PRESERVATION-FAIL:",
    "BACKTICKED-CLAIM-STALE:",
    "MARKETING-TABLE-CLAIM-STALE:",
    "ROADMAP-HEADER-DRIFT:",
)


def is_fire(line):
    stripped = line.lstrip()
    return any(stripped.startswith(p) for p in FIRE_PREFIXES)


def parse_ver(v):
    parts = str(v).strip().lstrip("v").split(".")
    out = []
    for p in parts:
        if not p.isdigit():
            return None
        out.append(int(p))
    while len(out) < 3:
        out.append(0)
    return tuple(out[:3])


try:
    with open(ef_path, "r", encoding="utf-8") as f:
        data = json.load(f)
except (OSError, ValueError) as exc:
    print(f"trust-claim-sweep: could not parse --expected-failures {ef_path}: {exc}", file=sys.stderr)
    sys.exit(2)

if not isinstance(data, dict) or not isinstance(data.get("entries"), list):
    print(f"trust-claim-sweep: --expected-failures {ef_path}: top-level object must have an 'entries' array", file=sys.stderr)
    sys.exit(2)

cur_tuple = parse_ver(current)
norm = []
for i, e in enumerate(data["entries"]):
    if not isinstance(e, dict):
        print(f"trust-claim-sweep: --expected-failures entry #{i} is not an object", file=sys.stderr)
        sys.exit(2)
    surface, reason, expires = e.get("surface"), e.get("reason"), e.get("expiresAfter")
    pattern = e.get("pattern", "") or ""
    if not surface or not reason or not expires:
        print(f"trust-claim-sweep: --expected-failures entry #{i} requires surface, reason, expiresAfter", file=sys.stderr)
        sys.exit(2)
    exp_tuple = parse_ver(expires)
    if exp_tuple is None:
        print(f"trust-claim-sweep: --expected-failures entry #{i} has malformed expiresAfter '{expires}' (need X.Y.Z)", file=sys.stderr)
        sys.exit(2)
    norm.append({"surface": surface, "pattern": pattern, "expires": expires, "exp_tuple": exp_tuple, "matched": 0})

try:
    with open(err_path, "r", encoding="utf-8") as f:
        err_lines = f.read().splitlines()
except OSError:
    err_lines = []

fire_lines = [ln for ln in err_lines if is_fire(ln)]
other_lines = [ln for ln in err_lines if not is_fire(ln)]

expired = [e for e in norm if cur_tuple is not None and cur_tuple > e["exp_tuple"]]
active = [e for e in norm if not (cur_tuple is not None and cur_tuple > e["exp_tuple"])]


def entry_matches(e, line):
    if (" " + e["surface"] + ":") not in line:
        return False
    if e["pattern"] and e["pattern"] not in line:
        return False
    return True


surviving = []
for ln in fire_lines:
    hit = next((e for e in active if entry_matches(e, ln)), None)
    if hit is not None:
        hit["matched"] += 1
    else:
        surviving.append(ln)

for e in expired:
    print(f"EXPECTED-FAILURE-EXPIRED: {e['surface']} entry expiresAfter {e['expires']} but current is {current}", file=sys.stderr)
for e in active:
    if e["matched"] > 0:
        print(f"EXPECTED-FAILURE-SUPPRESSED: {e['surface']}: {e['pattern']} (expiresAfter {e['expires']})", file=sys.stderr)
    else:
        print(f"EXPECTED-FAILURE-DEAD: {e['surface']}: {e['pattern']} matched no actual fire (expiresAfter {e['expires']})", file=sys.stderr)

for ln in other_lines:
    print(ln, file=sys.stderr)
for ln in surviving:
    print(ln, file=sys.stderr)

dead = sum(1 for e in active if e["matched"] == 0)
print(
    f"trust-claim-sweep: expected-failures — {len(fire_lines)} fire(s); "
    f"{len(fire_lines) - len(surviving)} suppressed; {len(surviving)} surviving; "
    f"{len(expired)} expired; {dead} dead."
)

if inner_rc == 2:
    sys.exit(2)

structural = any("malformed snapshot line" in ln for ln in other_lines)
fail = bool(expired) or bool(surviving) or structural
if inner_rc == 1 and not fire_lines and not expired:
    # Inner failed for a reason we did not recognize as a suppressible fire.
    fail = True
sys.exit(1 if fail else 0)
PY
    exit $?
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
collect_surfaces SURFACES
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
# (no Unreleased; just versioned sections — v1.13 PR-V1 audited + locked
# this behavior in via dedicated vscode-CHANGELOG self-test cases, closing
# the 3-cycle manual-backtick workaround at PR-G v1.10 / v1.11 / v1.12).
#
# NOTE: docs-site tables with version-row layouts (e.g. a "Releases" page
# listing `1.9.0 | 255 probes` rows) must use backtick-wrap or a
# `text-numeric-historical` fenced block to flag the row content as
# intentionally-historical. The header-second-onward mask only fires on
# files named `CHANGELOG.md`; docs-site `*.md` / `*.mdx` get no implicit
# historical carve-out. Level-3 (`### 0.X.Y`) section markers are NOT
# recognized — vscode CHANGELOG sections must use `## ` per format
# convention (locked in by the level-3 negative-space self-test).
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

# --- backticked-claim-stale scanner (v1.14 PR-A1) ----------------------
#
# Closes the v1.13 docs-sync audit 8-blocker pattern (per project-v23
# retrospective rule 9): the existing prior-leak sweep's backtick
# carve-out (carve-out 2 in the per-surface scan above) lets stale-but-
# still-backticked numbers escape. A surface that backticks a prior
# release's `271 probes` to "preserve" it across the sweep STILL leaks
# trust if that number isn't current and isn't recorded as historical
# in a CHANGELOG block — backticks are a typography signal, not a
# legitimacy claim. The probe-density audit independently flagged the
# same blind-spot.
#
# Validity rules per the v1.14 PR-A1 brief:
#   A backticked `<num> <key>` in any tracked surface is valid IFF
#   either (a) `<num> <key>` matches a current `text-numeric` pin in
#   CHANGELOG, OR (b) the backticked occurrence appears inside a
#   `text-numeric-historical` fenced block. CHANGELOG.md sections from
#   the 2nd `## ` header onward are also masked (per the same convention
#   as the prior-leak sweep) — historical CHANGELOG sections are
#   immutable by design and cite their own pinned numbers verbatim.
stale_fail=0
STALE_TOTAL=0
STALE_HITS=0
if [ -f "$REPO_ROOT/$CHANGELOG" ]; then
    SURFACES_FOR_STALE=()
    collect_surfaces SURFACES_FOR_STALE

    # Pre-extract current + historical pin sets ONCE; reuse for every
    # surface scan. A parser crash here is fail-loud (tmpfile + exit-code
    # capture, matching the prior-leak scanner's convention).
    _TC_STALE_TMP=$(mktemp 2>/dev/null || mktemp -t trust-claim-sweep-stale)
    trap 'rm -f "$_TC_PINS_TMP" "$_TC_STALE_TMP"' EXIT
    python3 - "$REPO_ROOT/$CHANGELOG" "$CURRENT_VERSION" > "$_TC_STALE_TMP" <<'PY'
import re
import sys

path = sys.argv[1]
current = sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    text = f.read()

current_section = re.compile(
    r"^## \[" + re.escape(current) + r"\][^\n]*\n([\s\S]*?)(?=^## \[|\Z)",
    re.MULTILINE,
)
allowed = ("probes", "positive", "negative", "fixtures", "tests", "donors")

current_pins = set()
m = current_section.search(text)
if m:
    block_pat = re.compile(r"^```text-numeric\n([\s\S]*?)^```", re.MULTILINE)
    for bm in block_pat.finditer(m.group(1)):
        for line in bm.group(1).splitlines():
            parts = line.strip().split(None, 1)
            if len(parts) < 2:
                continue
            num, key = parts[0], parts[1].split()[0]
            if num.isdigit() and key in allowed:
                current_pins.add(f"{num} {key}")

historical_pins = set()
hist_block_pat = re.compile(
    r"^```text-numeric-historical\n([\s\S]*?)^```", re.MULTILINE
)
for bm in hist_block_pat.finditer(text):
    for line in bm.group(1).splitlines():
        parts = line.strip().split(None, 1)
        if len(parts) < 2:
            continue
        num, key = parts[0], parts[1].split()[0]
        if num.isdigit() and key in allowed:
            historical_pins.add(f"{num} {key}")

print("CURRENT")
for p in sorted(current_pins):
    print(p)
print("HISTORICAL")
for p in sorted(historical_pins):
    print(p)
PY
    _TC_STALE_RC=$?
    if [ "$_TC_STALE_RC" -ne 0 ]; then
        echo "trust-claim-sweep: CHANGELOG parser failed (exit $_TC_STALE_RC) for stale-scanner pin extraction" >&2
        exit 2
    fi
    STALE_PIN_SETS=$(cat "$_TC_STALE_TMP")
    # Degraded mode: if the current CHANGELOG section has no text-numeric
    # block, there's no "current pin truth source" for the stale scanner
    # to validate against, so any backticked `<num> <key>` would over-fire
    # unless it happens to live inside a historical block. Skip with a
    # stderr note (cold-start / pre-v1.9 / test-fixture mode); the existing
    # prior-leak sweep + tripwire still run.
    if ! printf '%s\n' "$STALE_PIN_SETS" | awk '/^CURRENT$/{flag=1; next} /^HISTORICAL$/{exit} flag && NF' | grep -q '^.'; then
        echo "trust-claim-sweep: no current text-numeric pins for v$CURRENT_VERSION in $CHANGELOG; backticked-claim-stale scanner skipped." >&2
        SURFACES_FOR_STALE=()
    fi

    while IFS= read -r surface; do
        [ -z "$surface" ] && continue
        STALE_PIN_SETS_ENV="$STALE_PIN_SETS" \
        SURFACE_PATH="$REPO_ROOT/$surface" \
        SURFACE_DISPLAY="$surface" \
        python3 - <<'PY'
import os
import re
import sys

surface_path = os.environ["SURFACE_PATH"]
surface_display = os.environ["SURFACE_DISPLAY"]

current_pins = set()
historical_pins = set()
bucket = None
for ln in os.environ["STALE_PIN_SETS_ENV"].splitlines():
    ln = ln.strip()
    if ln == "CURRENT":
        bucket = current_pins
        continue
    if ln == "HISTORICAL":
        bucket = historical_pins
        continue
    if not ln or bucket is None:
        continue
    bucket.add(ln)

try:
    with open(surface_path, "r", encoding="utf-8") as f:
        text = f.read()
except OSError as exc:
    print(f"trust-claim-sweep: could not read {surface_display}: {exc}", file=sys.stderr)
    sys.exit(2)

def blank_keep_lines(match):
    return "\n" * match.group(0).count("\n")

hist_fence = re.compile(
    r"^```text-numeric-historical\n[\s\S]*?^```",
    re.MULTILINE,
)
text = hist_fence.sub(blank_keep_lines, text)

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

_BT = chr(96)
allowed = "probes|positive|negative|fixtures|tests|donors"
pat = re.compile(
    _BT + r"(\d+)\s+(" + allowed + r")" + _BT
)

hits = 0
for m in pat.finditer(text):
    num, key = m.group(1), m.group(2)
    pin = f"{num} {key}"
    if pin in current_pins:
        continue
    if pin in historical_pins:
        continue
    line_no = text.count("\n", 0, m.start()) + 1
    print(
        f"BACKTICKED-CLAIM-STALE: {surface_display}:{line_no}: "
        f"{_BT}{pin}{_BT} (non-current, non-historical)",
        file=sys.stderr,
    )
    hits += 1

sys.exit(1 if hits > 0 else 0)
PY
        rc=$?
        STALE_TOTAL=$((STALE_TOTAL + 1))
        if [ "$rc" -eq 1 ]; then
            stale_fail=1
            STALE_HITS=$((STALE_HITS + 1))
        elif [ "$rc" -ne 0 ]; then
            echo "trust-claim-sweep: stale-scanner aborted on $surface (exit $rc)" >&2
            stale_fail=1
        fi
    done <<EOF
$(printf '%s\n' "${SURFACES_FOR_STALE[@]:-}")
EOF
    echo "trust-claim-sweep: backticked-claim-stale scanned $STALE_TOTAL surface(s); $STALE_HITS surface(s) had stale backticked claims."
    if [ "$stale_fail" -ne 0 ]; then
        echo "trust-claim-sweep: backticked-claim-stale gate fired. A backticked '<num> <key>' must EITHER match a current text-numeric pin OR live inside a text-numeric-historical fenced block. Closes v1.13 docs-sync audit 8-blocker pattern (project-v23-retrospective rule 9)." >&2
    fi
fi

# --- unbackticked-marketing-table scanner (v1.15 PR-A1) ----------------
#
# Closes the v1.14 architecture-auditor finding (project-v24-retrospective)
# that backticked-claim-stale only sees `<num> <key>` pairs wrapped in
# single-backticks. Marketing-table surfaces (e.g. trajectory tables in
# docs-site/about/pandas-roadmap.md) carry UNBACKTICKED `<num> <key>` in
# table cells per the rendered-typography convention — and those bare
# numbers escaped both the prior-leak sweep (only fires on the PRIOR
# release's specific number) and the backticked-claim-stale scanner
# (requires backticks). A surface that writes `223 probes` bare in a
# table row still claims trust; the validity rule should apply identically.
#
# Validity rule (same as backticked-claim-stale):
#   A bare `<num> <key>` IN A MARKDOWN-TABLE ROW is valid IFF either
#   (a) `<num> <key>` matches a current text-numeric pin in CHANGELOG,
#   OR (b) the bare occurrence appears inside a text-numeric-historical
#   fenced block on the surface. CHANGELOG.md sections from the 2nd `## `
#   header onward are masked (per the existing convention).
#
# Scope is INTENTIONALLY narrowed to table-row contexts. Bare numbers in
# prose paragraphs are already governed by the prior-leak sweep (catches
# the specific prior-release number) plus the CHANGELOG grep gate v3
# (catches prose-paragraph numerics in CHANGELOG.md). Generalizing to all
# bare numbers in all surfaces would be a much larger change with high
# false-positive risk on incidental numeric prose ("the 10 donors", etc.);
# the table-row context is the narrow surface the v1.14 audit flagged.
#
# Heuristic for "markdown-table row": a line that BOTH starts with `|`
# AND contains more than one `|` total. Separator rows (`|---|`) are
# included by the heuristic but contain no `<num> <key>` patterns and
# are naturally skipped by the inner regex.
table_fail=0
TABLE_TOTAL=0
TABLE_HITS=0
if [ -f "$REPO_ROOT/$CHANGELOG" ] && [ -n "${STALE_PIN_SETS:-}" ]; then
    SURFACES_FOR_TABLE=()
    collect_surfaces SURFACES_FOR_TABLE

    # Degraded mode: if the current CHANGELOG section had no text-numeric
    # block (stale-scanner short-circuited above), we have no truth source
    # for the table scanner either; skip with a stderr note.
    if ! printf '%s\n' "$STALE_PIN_SETS" | awk '/^CURRENT$/{flag=1; next} /^HISTORICAL$/{exit} flag && NF' | grep -q '^.'; then
        echo "trust-claim-sweep: no current text-numeric pins for v$CURRENT_VERSION in $CHANGELOG; unbackticked-marketing-table scanner skipped." >&2
        SURFACES_FOR_TABLE=()
    fi

    while IFS= read -r surface; do
        [ -z "$surface" ] && continue
        STALE_PIN_SETS_ENV="$STALE_PIN_SETS" \
        SURFACE_PATH="$REPO_ROOT/$surface" \
        SURFACE_DISPLAY="$surface" \
        python3 - <<'PY'
import os
import re
import sys

surface_path = os.environ["SURFACE_PATH"]
surface_display = os.environ["SURFACE_DISPLAY"]

current_pins = set()
historical_pins = set()
bucket = None
for ln in os.environ["STALE_PIN_SETS_ENV"].splitlines():
    ln = ln.strip()
    if ln == "CURRENT":
        bucket = current_pins
        continue
    if ln == "HISTORICAL":
        bucket = historical_pins
        continue
    if not ln or bucket is None:
        continue
    bucket.add(ln)

try:
    with open(surface_path, "r", encoding="utf-8") as f:
        text = f.read()
except OSError as exc:
    print(f"trust-claim-sweep: could not read {surface_display}: {exc}", file=sys.stderr)
    sys.exit(2)

def blank_keep_lines(match):
    return "\n" * match.group(0).count("\n")

# Carve-out: text-numeric-historical fenced blocks. A surface that
# explicitly opts into a historical block on a table-formatted release
# matrix (rare but possible) gets the same immunity as the backticked
# scanner above.
hist_fence = re.compile(
    r"^```text-numeric-historical\n[\s\S]*?^```",
    re.MULTILINE,
)
text = hist_fence.sub(blank_keep_lines, text)

# Carve-out: CHANGELOG.md sections from the 2nd `## ` header onward —
# historical CHANGELOG sections are immutable by design (same convention
# as the prior-leak sweep + stale-claim scanner).
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

# Number-key regex shared with the backticked-claim-stale scanner.
allowed = "probes|positive|negative|fixtures|tests|donors"
# Anchor: NOT preceded by a backtick (so the backticked-claim-stale
# scanner above owns those occurrences and this one only sees BARE
# numbers). Also NOT preceded by alnum/underscore so D-codes / identifier-
# prefixed digits don't false-match (same convention as the prior-leak
# sweep's `(?<![A-Za-z0-9_])` anchor). Followed by `\b` so `1234 probesx`
# does not match.
_BT = chr(96)
pat = re.compile(
    r"(?<![A-Za-z0-9_" + _BT + r"])(\d+)\s+(" + allowed + r")\b(?!" + _BT + r")"
)

hits = 0
# Iterate line-by-line so we can apply the table-row heuristic per line.
# A markdown-table row is a line that BOTH starts with `|` (after any
# leading whitespace) AND contains more than one `|` total. The cell
# content between the `|` characters is then scanned for bare `<num>
# <key>` pairs.
lines = text.splitlines(keepends=True)
char_offset = 0
for line in lines:
    stripped = line.lstrip()
    if stripped.startswith("|") and line.count("|") > 1:
        for m in pat.finditer(line):
            num, key = m.group(1), m.group(2)
            pin = f"{num} {key}"
            if pin in current_pins:
                continue
            if pin in historical_pins:
                continue
            # `line_no` reported is the line of the table-row match (1-based).
            line_no = text.count("\n", 0, char_offset + m.start()) + 1
            print(
                f"MARKETING-TABLE-CLAIM-STALE: {surface_display}:{line_no}: "
                f"'{pin}' bare in table row (non-current, non-historical)",
                file=sys.stderr,
            )
            hits += 1
    char_offset += len(line)

sys.exit(1 if hits > 0 else 0)
PY
        rc=$?
        TABLE_TOTAL=$((TABLE_TOTAL + 1))
        if [ "$rc" -eq 1 ]; then
            table_fail=1
            TABLE_HITS=$((TABLE_HITS + 1))
        elif [ "$rc" -ne 0 ]; then
            echo "trust-claim-sweep: marketing-table-scanner aborted on $surface (exit $rc)" >&2
            table_fail=1
        fi
    done <<EOF
$(printf '%s\n' "${SURFACES_FOR_TABLE[@]:-}")
EOF
    echo "trust-claim-sweep: unbackticked-marketing-table scanned $TABLE_TOTAL surface(s); $TABLE_HITS surface(s) had stale table-row claims."
    if [ "$table_fail" -ne 0 ]; then
        echo "trust-claim-sweep: unbackticked-marketing-table gate fired. A bare '<num> <key>' in a markdown-table row must EITHER match a current text-numeric pin OR live inside a text-numeric-historical fenced block. Wrap in backticks (then it'll be governed by the stale-claim scanner) OR add a text-numeric-historical block for the cycle. Closes v1.14 architecture-auditor finding (project-v24-retrospective)." >&2
    fi
fi

# --- roadmap-header guard (v1.16 PR-A1) ---------------------------------
#
# Per v1.16-spec §1.iv.1. Asserts BOTH:
#   (a) the highest "## Where we are (vX.Y.Z)" header in pandas-roadmap.md
#       matches CURRENT_VERSION, AND
#   (b) the highest "## Shipped in vX.Y" section across the roadmap docs
#       (pandas-roadmap.md + about/roadmap.md + docs/roadmap.md) matches
#       CURRENT_VERSION's major.minor.
# Same shell shape as the marketing-table scan. Fires ROADMAP-HEADER-DRIFT
# to stderr — suppressible via the --expected-failures allowlist so PR-A1
# can land the guard while PR-G resolves the standing drift (the live
# pandas-roadmap.md "Where we are (v1.14.0)" header is two cycles stale).
# Skips cleanly when pandas-roadmap.md is absent (test-fixture repos).
roadmap_fail=0
ROADMAP_PANDAS="docs-site/src/content/docs/about/pandas-roadmap.md"
if [ -f "$REPO_ROOT/$ROADMAP_PANDAS" ]; then
    CURRENT_VERSION_ENV="$CURRENT_VERSION" \
    REPO_ROOT_ENV="$REPO_ROOT" \
    ROADMAP_PANDAS_ENV="$ROADMAP_PANDAS" \
    ROADMAP_FILES_ENV="$ROADMAP_PANDAS docs-site/src/content/docs/about/roadmap.md docs/roadmap.md" \
    python3 - <<'PY'
import os
import re
import sys

current = os.environ["CURRENT_VERSION_ENV"]
repo_root = os.environ["REPO_ROOT_ENV"]
pandas_rel = os.environ["ROADMAP_PANDAS_ENV"]
files_rel = os.environ["ROADMAP_FILES_ENV"].split()


def parse_ver(s):
    parts = s.strip().lstrip("v").split(".")
    out = []
    for p in parts:
        if not p.isdigit():
            return None
        out.append(int(p))
    while len(out) < 3:
        out.append(0)
    return tuple(out[:3])


cur = parse_ver(current)
fires = 0

# (a) highest "## Where we are (vX.Y[.Z])" header in pandas-roadmap.md.
try:
    with open(os.path.join(repo_root, pandas_rel), encoding="utf-8") as f:
        text = f.read()
except OSError:
    text = ""
where_pat = re.compile(r"^#{2,3}\s+Where we are \(v(\d+\.\d+(?:\.\d+)?)\)", re.MULTILINE)
best = None
for m in where_pat.finditer(text):
    v = m.group(1)
    t = parse_ver(v)
    line_no = text.count("\n", 0, m.start()) + 1
    if best is None or t > best[0]:
        best = (t, v, line_no)
if best is not None and best[0] != cur:
    print(
        f"ROADMAP-HEADER-DRIFT: {pandas_rel}:{best[2]}: "
        f"'Where we are (v{best[1]})' header is v{best[1]} but CURRENT_VERSION is v{current}",
        file=sys.stderr,
    )
    fires += 1

# (b) highest "## Shipped in vX.Y" section across the roadmap docs.
shipped_pat = re.compile(r"^#{2,3}\s+Shipped in v(\d+\.\d+)", re.MULTILINE)
best_shipped = None
for rel in files_rel:
    try:
        with open(os.path.join(repo_root, rel), encoding="utf-8") as f:
            ftext = f.read()
    except OSError:
        continue
    for m in shipped_pat.finditer(ftext):
        v = m.group(1)
        t = parse_ver(v)[:2]
        line_no = ftext.count("\n", 0, m.start()) + 1
        if best_shipped is None or t > best_shipped[0]:
            best_shipped = (t, v, rel, line_no)
if best_shipped is not None and best_shipped[0] != cur[:2]:
    print(
        f"ROADMAP-HEADER-DRIFT: {best_shipped[2]}:{best_shipped[3]}: "
        f"highest 'Shipped in v{best_shipped[1]}' section is v{best_shipped[1]} "
        f"but CURRENT_VERSION is v{current}",
        file=sys.stderr,
    )
    fires += 1

sys.exit(1 if fires else 0)
PY
    roadmap_rc=$?
    if [ "$roadmap_rc" -eq 1 ]; then
        roadmap_fail=1
    elif [ "$roadmap_rc" -ne 0 ]; then
        echo "trust-claim-sweep: roadmap-header guard aborted (exit $roadmap_rc)" >&2
        roadmap_fail=1
    fi
    echo "trust-claim-sweep: roadmap-header guard scanned pandas-roadmap.md + about/roadmap.md + docs/roadmap.md against v$CURRENT_VERSION."
    if [ "$roadmap_fail" -ne 0 ]; then
        echo "trust-claim-sweep: roadmap-header guard fired. Update the highest 'Where we are (vX.Y.Z)' header and 'Shipped in vX.Y' section to v$CURRENT_VERSION, OR hold the drift with an --expected-failures entry (expiresAfter set). Closes project-v26-backlog item 6 (v1.14 architecture-audit)." >&2
    fi
fi

if [ "$fail" -ne 0 ] || [ "$tripwire_fail" -ne 0 ] || [ "$stale_fail" -ne 0 ] || [ "$table_fail" -ne 0 ] || [ "$roadmap_fail" -ne 0 ]; then
    exit 1
fi

exit 0
