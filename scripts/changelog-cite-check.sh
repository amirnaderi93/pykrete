#!/usr/bin/env bash
# v1.11 PR-A2 — CHANGELOG file:line cite-check.
#
# Closes v1.10 retro rule 3 (v1.10 CHANGELOG cited --snapshot at
# `alias_report.rs` instead of `main.rs:972`; cited the v1.9 author-
# boundary carve-out at `:282-284` instead of `:446-448`). The drift
# class is "CHANGELOG mentions a file path + line number that doesn't
# resolve to the claimed code anymore." This script catches it
# mechanically.
#
# WHAT IT DOES
# ------------
# 1. Parses CHANGELOG.md for tokens matching
#    `[A-Za-z0-9_/.-]+\.(rs|py|sh|yml|yaml|md|json|toml):\d+`
#    (e.g. `crates/pykrete/src/main.rs:972`, `scripts/probes.py:215`).
# 2. For each cite:
#    - File must exist at that path. We try (a) the literal path
#      relative to repo root first, then (b) if the cite is a bare
#      basename, look for a unique match in the repo tree under
#      `crates/`, `scripts/`, `editors/`, `.github/`. Ambiguous bare
#      basenames fail.
#    - Line number must be within range (≤ `wc -l`).
#    - Heuristic anchor check: extract a keyword from the CHANGELOG
#      sentence within ~80 chars of the cite (function / struct /
#      identifier matching `[A-Za-z_][A-Za-z0-9_]{2,}`); if the keyword
#      appears in the source range (5 lines before / after the cited
#      line), the cite is "anchored." If not, WARN to stderr but exit
#      0 — anchor is heuristic, not authoritative.
# 3. Skip cites inside historical CHANGELOG sections (sections older
#    than the latest released version). Historical sections are
#    preserved-as-artifact: a v1.9 cite that drifted at v1.10 release
#    time stays drifted in the history.
#
# EXIT CODES
# ----------
# 0  — every non-historical cite resolves to existing file + in-range
#      line. Anchor warnings are NOT failures.
# 1  — at least one cite fails (file missing or line out of range).
# 2  — script invocation error (CHANGELOG not found, etc.).

set -u

CHANGELOG="${CHANGELOG:-CHANGELOG.md}"
REPO_ROOT="${REPO_ROOT:-.}"

if [ ! -f "$CHANGELOG" ]; then
    echo "changelog-cite-check: CHANGELOG file not found at '$CHANGELOG'" >&2
    exit 2
fi

if [ ! -d "$REPO_ROOT" ]; then
    echo "changelog-cite-check: REPO_ROOT not a directory: '$REPO_ROOT'" >&2
    exit 2
fi

# Python does the parse + verify in one shot — bash regex + arithmetic
# over markdown sections is fragile and we already require python3
# elsewhere (changelog-grep.sh uses it too).
python3 - "$CHANGELOG" "$REPO_ROOT" <<'PY'
import os
import re
import sys

changelog_path = sys.argv[1]
repo_root = sys.argv[2]

with open(changelog_path, encoding='utf-8') as f:
    lines = f.read().splitlines()

# Section header pattern: `## [version]` or `## [Unreleased]`.
section_re = re.compile(r'^## \[(?P<name>[^\]]+)\]')
# A "current" section is `[Unreleased]` or the first released version
# we encounter. Everything from the SECOND released version onward is
# historical and skipped.
header_indices = []
for i, line in enumerate(lines):
    m = section_re.match(line)
    if m:
        header_indices.append((i, m.group('name')))

# Identify the cutoff line: first historical section header. Anything
# at or after this index is skipped. If there's only one or zero
# released sections, nothing is historical.
released_seen = 0
cutoff_line = len(lines)
for idx, name in header_indices:
    if name == 'Unreleased':
        continue
    released_seen += 1
    if released_seen == 2:
        cutoff_line = idx
        break

# Cite pattern: <path-chars>.<ext>:<line>. Path chars exclude
# whitespace, colons (so we don't span citations), and a few markdown-
# noise punctuators.
cite_re = re.compile(
    r'([A-Za-z0-9_./\-]+\.(?:rs|py|sh|yml|yaml|md|json|toml)):(\d+)'
)

# Keyword extractor: a sentence-shaped window around the cite. We
# slice ~80 chars before and after the cite within the same line and
# pull identifier-shaped tokens.
KEYWORD_RE = re.compile(r'[A-Za-z_][A-Za-z0-9_]{2,}')
# Stop words: extension names, common english, markdown noise — these
# don't carry signal even if they live in source too.
STOP = {
    'the', 'and', 'for', 'with', 'from', 'this', 'that', 'into', 'when',
    'where', 'than', 'then', 'over', 'under', 'between', 'across',
    'crates', 'pykrete', 'scripts', 'editors', 'src', 'tests', 'main',
    'lib', 'mod', 'use', 'fn', 'pub', 'let', 'rust', 'python', 'bash',
    'yaml', 'json', 'toml', 'check', 'file', 'line', 'lines', 'spec',
    'docs', 'design', 'rule', 'arm', 'shape', 'kind', 'name', 'side',
    'gate', 'flag', 'block', 'blocks', 'mode', 'path', 'paths', 'self',
}

errors = []
warnings = []
ok_count = 0
historical_skipped = 0

for i, line in enumerate(lines):
    for m in cite_re.finditer(line):
        path, line_str = m.group(1), m.group(2)
        line_no = int(line_str)

        if i >= cutoff_line:
            historical_skipped += 1
            continue

        full_path = os.path.join(repo_root, path)
        if not os.path.isfile(full_path):
            # Bare-basename fallback: if the cite has no path separator,
            # search a small set of source roots for a unique match.
            # Ambiguous matches (or no matches) fail.
            if os.sep not in path and '/' not in path:
                matches = []
                for src_root in ('crates', 'scripts', 'editors', '.github'):
                    walk_root = os.path.join(repo_root, src_root)
                    if not os.path.isdir(walk_root):
                        continue
                    for dirpath, _, filenames in os.walk(walk_root):
                        if path in filenames:
                            matches.append(os.path.join(dirpath, path))
                if len(matches) == 1:
                    full_path = matches[0]
                elif len(matches) > 1:
                    errors.append(
                        f"CHANGELOG.md:{i+1}: cite '{path}:{line_no}' — bare basename "
                        f"matches {len(matches)} files; use the qualified path "
                        f"(e.g. {matches[0].replace(repo_root + os.sep, '')})"
                    )
                    continue
                else:
                    errors.append(
                        f"CHANGELOG.md:{i+1}: cite '{path}:{line_no}' — file not "
                        f"found at '{full_path}' (and no basename match in repo)"
                    )
                    continue
            else:
                errors.append(
                    f"CHANGELOG.md:{i+1}: cite '{path}:{line_no}' — file not found at "
                    f"'{full_path}'"
                )
                continue

        with open(full_path, encoding='utf-8', errors='replace') as fh:
            src_lines = fh.read().splitlines()
        if line_no < 1 or line_no > len(src_lines):
            errors.append(
                f"CHANGELOG.md:{i+1}: cite '{path}:{line_no}' — line {line_no} out of "
                f"range (file has {len(src_lines)} lines)"
            )
            continue

        ok_count += 1

        # Anchor heuristic: pull tokens from the CHANGELOG sentence
        # window (~80 chars each side of the cite, within the same
        # line), then check whether any token also appears in the 5-
        # line source window. If yes, the cite is "anchored." If no,
        # warn (don't fail).
        start_window = max(0, m.start() - 80)
        end_window = min(len(line), m.end() + 80)
        window = line[start_window:m.start()] + ' ' + line[m.end():end_window]
        candidates = [
            tok for tok in KEYWORD_RE.findall(window)
            if tok.lower() not in STOP and len(tok) >= 3
        ]
        if not candidates:
            continue

        src_window_lo = max(0, line_no - 1 - 5)
        src_window_hi = min(len(src_lines), line_no - 1 + 6)
        src_blob = '\n'.join(src_lines[src_window_lo:src_window_hi])

        anchored = any(tok in src_blob for tok in candidates)
        if not anchored:
            warnings.append(
                f"CHANGELOG.md:{i+1}: cite '{path}:{line_no}' — anchor heuristic "
                f"did not match any of {candidates[:5]} in source lines "
                f"{src_window_lo+1}-{src_window_hi}"
            )

for e in errors:
    print(f"FAIL: {e}", file=sys.stderr)
for w in warnings:
    print(f"WARN: {w}", file=sys.stderr)

summary = (
    f"changelog-cite-check: {ok_count} cite(s) verified, "
    f"{len(warnings)} anchor warning(s), {len(errors)} failure(s), "
    f"{historical_skipped} historical cite(s) skipped"
)
print(summary)

sys.exit(1 if errors else 0)
PY
