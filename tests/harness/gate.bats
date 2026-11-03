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
	@for i in $$(seq 1 30); do echo "noise line $$i"; done
	@exit 1
MK
  run env GATE_ROOT="$ROOT" "$REPO_ROOT/tools/gate.sh" harness
  [ "$status" -ne 0 ]
  [[ "$output" == *"FAIL  harness"* ]]
  [[ "$output" == *"not ok 2 ledger validation failed"* ]]
  [[ "$output" == *"# (in test file tests/harness/check_agent_test_map.bats, line 10)"* ]]
}

@test "gate executes unit-agent-bats step when defined in Makefile" {
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
unit-agent-bats:
	@echo "unit-agent-bats ok"
MK
  run env GATE_ROOT="$ROOT" "$REPO_ROOT/tools/gate.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"gate: 8 steps"* ]]
  [[ "$output" == *"ok    unit-agent-bats"* ]]
}

@test "a failing unit-agent-bats step shows its name and diagnosis" {
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
	@exit 0
unit-agent-bats:
	@echo "1..2"
	@echo "ok 1 candidate setup"
	@echo "not ok 2 candidate test failed"
	@echo "# (in test file tests/unit/agent.bats, line 42)"
	@echo "#   \`agent run' failed"
	@for i in $$(seq 1 30); do echo "noise line $$i"; done
	@exit 1
MK
  run env GATE_ROOT="$ROOT" "$REPO_ROOT/tools/gate.sh" unit-agent-bats
  [ "$status" -ne 0 ]
  [[ "$output" == *"FAIL  unit-agent-bats"* ]]
  [[ "$output" == *"not ok 2 candidate test failed"* ]]
  [[ "$output" == *"# (in test file tests/unit/agent.bats, line 42)"* ]]
}
