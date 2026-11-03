#!/usr/bin/env bash
# Run Bats test cases against the Go candidate (Lane B).
# Only runs active/done blackbox cases per the ledger.
# Usage: tools/unit-agent-bats.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Build the Go binary if needed.
if [ ! -x build/agent-go ]; then
	make build-agent >/dev/null 2>&1
fi

# Stage the repo with the Go binary.
STAGED="$(mktemp -d)"
trap "rm -rf '$STAGED'" EXIT

bash tools/agent-go-bats.sh "$STAGED" >/dev/null 2>&1

# For R04, we run the full legacy suite to establish baseline.
# Later rounds will filter to active/done blackbox cases only.
BATS_JOBS="${BATS_JOBS:-$(nproc 2>/dev/null || echo 1)}"
BATS_SERIAL="${BATS_SERIAL:- }"

STAGED_ROOT="$STAGED" \
BATS_JOBS="$BATS_JOBS" \
BATS_SERIAL="$BATS_SERIAL" \
bash tools/unit-bats.sh
