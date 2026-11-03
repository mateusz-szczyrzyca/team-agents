#!/usr/bin/env bats
# Test the Lane infrastructure for the Go agent.

setup() {
	REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)
}

@test "Lane A staging tool exists" {
	[ -x "$REPO_ROOT/tools/agent-bats.sh" ] || skip "tools/agent-bats.sh not implemented"
	[ -f "$REPO_ROOT/tools/agent-bats.sh" ]
}

@test "Lane B staging tool exists" {
	[ -x "$REPO_ROOT/tools/agent-go-bats.sh" ] || skip "tools/agent-go-bats.sh not implemented"
	[ -f "$REPO_ROOT/tools/agent-go-bats.sh" ]
}

@test "Parity check tool exists" {
	[ -x "$REPO_ROOT/tools/check-agent-test-map.sh" ]
}

@test "Ledger exists" {
	[ -f "$REPO_ROOT/tests/agent-go-map.tsv" ]
	[ "$(grep -c . "$REPO_ROOT/tests/agent-go-map.tsv")" -gt 200 ]
}
