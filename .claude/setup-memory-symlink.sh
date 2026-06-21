#!/usr/bin/env bash
# Memory portability — run this once on a fresh machine after cloning/copying
# this project directory. Recreates the symlink Claude Code's auto-memory
# system expects so it reads/writes memories from .claude/memory/ inside the
# project tree (which travels with `cp -R` of the project, unlike
# ~/.claude/projects/.../memory).
#
# The symlink target is the absolute path to .claude/memory/ in the project,
# resolved from this script's own location — so the script works wherever
# you cloned the project to.
#
# Safe to re-run: removes any existing symlink/dir at the target before
# creating the new symlink. If a REAL directory (not a symlink) is at the
# target with files in it, the script refuses to delete it — manually
# merge/move first.

set -euo pipefail

# Resolve project tree memory dir (absolute, follows symlinks for portability).
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_MEMORY="$PROJECT_DIR/.claude/memory"

# Claude Code expects this exact path per its system prompt.
# The path encodes the project's working directory with `/` replaced by `-`.
SLUG="$(echo "$PROJECT_DIR" | sed 's|/|-|g')"
GLOBAL_LINK="$HOME/.claude/projects/${SLUG}/memory"
GLOBAL_PARENT="$HOME/.claude/projects/${SLUG}"

echo "Project memory:  $PROJECT_MEMORY"
echo "Global symlink:  $GLOBAL_LINK"

# Sanity: the project memory dir must exist with content (or at least a MEMORY.md).
if [ ! -d "$PROJECT_MEMORY" ]; then
  echo "Error: $PROJECT_MEMORY does not exist."
  echo "       This project's memory hasn't been initialized. If you just cloned the"
  echo "       project and expected memories to be present, the .claude/memory/ dir"
  echo "       was not copied along with the project. Restore from backup or start"
  echo "       fresh — Claude will populate it on first use."
  exit 1
fi

# Make sure the parent dir exists (Claude Code's session-projects dir).
mkdir -p "$GLOBAL_PARENT"

# If a symlink is already there, remove it.
if [ -L "$GLOBAL_LINK" ]; then
  echo "Removing existing symlink at $GLOBAL_LINK"
  rm "$GLOBAL_LINK"
elif [ -e "$GLOBAL_LINK" ]; then
  # Something real (file or dir) is at the path.
  if [ -d "$GLOBAL_LINK" ] && [ -z "$(ls -A "$GLOBAL_LINK" 2>/dev/null)" ]; then
    # Empty dir — safe to remove.
    echo "Removing empty directory at $GLOBAL_LINK"
    rmdir "$GLOBAL_LINK"
  else
    echo "Error: $GLOBAL_LINK exists and is not an empty directory or symlink."
    echo "       Refusing to overwrite. Inspect manually and either:"
    echo "         - merge its content into $PROJECT_MEMORY (if it has newer memories)"
    echo "         - move/delete it"
    echo "       Then re-run this script."
    exit 1
  fi
fi

# Create the symlink.
ln -s "$PROJECT_MEMORY" "$GLOBAL_LINK"
echo "Symlink created: $GLOBAL_LINK -> $PROJECT_MEMORY"

# Verify Claude can see MEMORY.md through the symlink.
if [ -f "$GLOBAL_LINK/MEMORY.md" ]; then
  echo "Verified: MEMORY.md readable through symlink."
else
  echo "Warning: MEMORY.md not found at $GLOBAL_LINK/MEMORY.md."
  echo "         Symlink is in place but the project memory dir has no index file."
fi

echo "Done."
