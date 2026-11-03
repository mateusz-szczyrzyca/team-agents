#!/usr/bin/env bats
# Tests for tools/gate.sh harness execution and failure excerpt.

setup() {
  REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)
  ROOT=$(mktemp -d)
  cat > "$ROOT/Makefile" <<'MK'
check-generated:
	@echo "check-generated ok"
lint-shell:
	@echo "lint-shell ok"
lint-go:
	@echo "lint-go ok"
lint-docs:
	@echo "lint-docs ok"
unit-go:
	@echo "unit-go ok"
unit-bats:
	@echo "unit-bats ok"
harness:
	@echo "harness ok"
MK
}

teardown() { rm -rf "$ROOT"; }

@test "gate executes harness step by default" {
  run env GATE_ROOT="$ROOT" "$REPO_ROOT/tools/gate.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"gate: 7 steps"* ]]
  [[ "$output" == *"ok    harness"* ]]
}

@test "a failing harness step shows its name and diagnosis" {
  cat > "$ROOT/Makefile" <<'MK'
check-generated:
	@exit 0
lint-shell:
	@exit 0
lint-go:
	@exit 0
lint-docs:
	@exit 0
unit-go:
	@exit 0
unit-bats:
	@exit 0
harness:
	@echo "1..2"
	@echo "ok 1 passing test"
	@echo "not ok 2 ledger validation failed"
	@echo "# (in test file tests/harness/check_agent_test_map.bats, line 10)"
	@echo "#   \`check-agent-test-map' failed"
	@exit 1
MK
  run env GATE_ROOT="$ROOT" "$REPO_ROOT/tools/gate.sh" harness
  [ "$status" -ne 0 ]
  [[ "$output" == *"FAIL  harness"* ]]
  [[ "$output" == *"not ok 2 ledger validation failed"* ]]
  [[ "$output" == *"# (in test file tests/harness/check_agent_test_map.bats, line 10)"* ]]
}
