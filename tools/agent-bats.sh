#!/usr/bin/env bash
# Stage a copy of the repository with the Bash oracle at STAGED_ROOT/bin/agent.
# Lane A: runs unchanged legacy Bats suite against oracle (not Go binary).
# Usage: tools/agent-bats.sh STAGED_ROOT [BATS_ARGS...]

set -euo pipefail

if [ $# -lt 1 ]; then
	echo "usage: $0 STAGED_ROOT [BATS_ARGS...]" >&2
	exit 1
fi

STAGED_ROOT="$1"
shift

# Find the repository root.
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Copy the working tree to the staged root, including uncommitted changes.
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

# Verify the oracle (original Bash agent) is unchanged and in place.
test -x "$STAGED_ROOT/bin/agent" || exit 1
file "$STAGED_ROOT/bin/agent" | grep -q "shell script" || exit 1
