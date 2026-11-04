#!/usr/bin/env bats
# Regression coverage for mandatory candidate evidence validation and lane separation.

setup() {
  REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)
  ROOT="$BATS_TEST_TMPDIR/repo"
  mkdir -p "$ROOT/tools" "$ROOT/tests/unit" "$ROOT/build" "$ROOT/bin" "$ROOT/agent"
  cp "$REPO_ROOT"/tools/{agent-bats,unit-bats,check-agent-parity,check-agent-test-map}.sh "$ROOT/tools/"
  printf '#!/bin/sh\nexit 0\n' > "$ROOT/build/agent-go"
  cp "$ROOT/build/agent-go" "$ROOT/bin/agent"
  chmod +x "$ROOT/build/agent-go" "$ROOT/bin/agent"
  printf 'bats_file\ttest_name\tclassification\timplementation_round\tgo_test_or_subtests\tstate\n' > "$ROOT/tests/agent-go-map.tsv"
  printf 'tests/unit/sample.bats\tcontract\tblackbox\tR04\tTestContract\tactive\n' >> "$ROOT/tests/agent-go-map.tsv"
  printf 'package main\nfunc TestContract() {}\n' > "$ROOT/agent/contract_test.go"
  printf '@test "contract" {\n  true\n}\n' > "$ROOT/tests/unit/sample.bats"
}

@test "candidate runner rejects skipped selected contracts and retains evidence" {
  printf '@test "contract" {\n  skip "contract not executed"\n}\n' > "$ROOT/tests/unit/sample.bats"
  run "$ROOT/tools/agent-bats.sh" --repo-root "$ROOT"
  [ "$status" -ne 0 ]
  jq -e '.selected_count == 1 and .executed_count == 1 and .skips == 1' "$ROOT/build/agent-bats-evidence.json"
}

@test "oracle execution preserves previously validated candidate evidence" {
  run "$ROOT/tools/agent-bats.sh" --repo-root "$ROOT"
  [ "$status" -eq 0 ]
  cp "$ROOT/build/agent-bats-evidence.json" "$ROOT/build/before.json"
  run env BATS_JOBS=1 "$ROOT/tools/agent-bats.sh" --repo-root "$ROOT" --lane oracle
  [ "$status" -eq 0 ]
  cmp "$ROOT/build/before.json" "$ROOT/build/agent-bats-evidence.json"
}

@test "mandatory candidate gate validates mappings after successful execution" {
  # Exercise the production recipe in a minimal fixture without rebuilding the Go executable.
  printf 'build-agent:\n\t@:\n' > "$ROOT/Makefile"
  awk '/^unit-agent-bats:/{copy=1} copy && /^$/{exit} copy{print}' "$REPO_ROOT/Makefile" >> "$ROOT/Makefile"
  run env GATE_ROOT="$ROOT" "$REPO_ROOT/tools/gate.sh" unit-agent-bats
  [ "$status" -eq 0 ]
  jq -e '.executed_count == 1 and .failures == 0' "$ROOT/build/agent-bats-evidence.json"
  printf 'package main\n' > "$ROOT/agent/contract_test.go"
  run env GATE_ROOT="$ROOT" "$REPO_ROOT/tools/gate.sh" unit-agent-bats
  [ "$status" -ne 0 ]
  [[ "$output" == *"FAIL  unit-agent-bats"* ]]
  # The runner succeeded; parity must detect the now-invalid mapped Go declaration.
  jq -e '.executed_count == 1 and .failures == 0' "$ROOT/build/agent-bats-evidence.json"
  run cat "$ROOT/build/gate/unit-agent-bats.log"
  [[ "$output" == *"Go test mapping not declared"* ]]
}

@test "oracle rejects candidate-only explicit evidence output" {
  run "$ROOT/tools/agent-bats.sh" --repo-root "$ROOT" --lane oracle --evidence "$ROOT/build/explicit.json"
  [ "$status" -ne 0 ]
  [[ "$output" == *"--evidence is valid only for candidate lane"* ]]
  [ ! -e "$ROOT/build/explicit.json" ]
}
