#!/usr/bin/env bash
# Stage a copy of the repository with the Go binary at STAGED_ROOT/bin/agent.
# Lane B: runs unchanged blackbox Bats suite against Go executable.
# Usage: tools/agent-go-bats.sh STAGED_ROOT [BATS_ARGS...]

set -euo pipefail

if [ $# -lt 1 ]; then
	echo "usage: $0 STAGED_ROOT [BATS_ARGS...]" >&2
	exit 1
fi

STAGED_ROOT="$1"
shift

# Find the repository root.
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Build the Go agent if not already built.
if [ ! -x "$REPO_ROOT/build/agent-go" ]; then
	cd "$REPO_ROOT"
	make build-agent >/dev/null 2>&1
fi

# Copy the working tree to the staged root, excluding the oracle.
mkdir -p "$STAGED_ROOT"
if [ -d "$STAGED_ROOT/.git" ]; then
	# Already staged; just ensure it's clean.
	:
else
	# Copy the entire working tree.
	cd "$REPO_ROOT"
	rsync -a \
		--exclude='.git' \
		--exclude='build/' \
		--exclude='agent/.git' \
		. "$STAGED_ROOT/"
fi

# Replace the oracle with the Go binary (not a symlink, a real copy).
mkdir -p "$STAGED_ROOT/bin"
rm -f "$STAGED_ROOT/bin/agent"
cp "$REPO_ROOT/build/agent-go" "$STAGED_ROOT/bin/agent"
chmod +x "$STAGED_ROOT/bin/agent"

# Verify it's an ELF binary.
file "$STAGED_ROOT/bin/agent" | grep -q "ELF" || exit 1
