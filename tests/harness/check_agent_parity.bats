#!/usr/bin/env bats
# Parity validation uses private ledgers and execution evidence, never real agents.

# Create a private test tree and matching candidate execution evidence.
setup() {
  REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)
  TOOL="$REPO_ROOT/tools/check-agent-parity.sh"
  FIXTURE="$BATS_TEST_TMPDIR/repo"
  EVIDENCE="$FIXTURE/build/agent-bats-evidence.json"
  mkdir -p "$FIXTURE/tests/unit" "$FIXTURE/build" "$FIXTURE/agent"
  printf 'package main\nimport "testing"\nfunc TestActive(t *testing.T) {}\nfunc TestDone(t *testing.T) {}\n' > "$FIXTURE/agent/sample_test.go"
  printf '@test "%s" { true; }\n' 'planned case' 'active case' 'done case' > "$FIXTURE/tests/unit/sample.bats"
  printf 'bats_file\ttest_name\tclassification\timplementation_round\tgo_test_or_subtests\tstate\n' > "$FIXTURE/tests/agent-go-map.tsv"
  printf 'tests/unit/sample.bats\tplanned case\tblackbox\tR56\t-\tplanned\n' >> "$FIXTURE/tests/agent-go-map.tsv"
  printf 'tests/unit/sample.bats\tactive case\tblackbox\tR04\tTestActive\tactive\n' >> "$FIXTURE/tests/agent-go-map.tsv"
  printf 'tests/unit/sample.bats\tdone case\tsource\tR01\tTestDone\tdone\n' >> "$FIXTURE/tests/agent-go-map.tsv"
  write_evidence
}

# Record evidence for the exact synthetic ledger and unchanged test tree.
write_evidence() {
  local map_hash tests_hash
  map_hash=$(sha256sum "$FIXTURE/tests/agent-go-map.tsv")
  tests_hash=$(cd "$FIXTURE" && find tests/unit -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum)
  jq -n --arg map "${map_hash%% *}" --arg tests "${tests_hash%% *}" \
    '{lane:"candidate", selected_count:1, executed_count:1, failures:0, skips:0,
      map_sha256:$map, tests_sha256:$tests}' > "$EVIDENCE"
}

# Change a single evidence field while preserving a valid JSON document.
alter_evidence() {
  jq "$1" "$EVIDENCE" > "$EVIDENCE.tmp"
  mv "$EVIDENCE.tmp" "$EVIDENCE"
}

@test "parity accepts consistent classifications states counts and quiet mode" {
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -eq 0 ]
  [[ "$output" == *"blackbox=2 source=1 planned=1 active=1 done=1"* ]]
  run env CHECK_ROOT="$FIXTURE" "$TOOL" --quiet --evidence "$EVIDENCE"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "parity rejects missing and duplicated ledger rows" {
  sed -i '/planned case/d' "$FIXTURE/tests/agent-go-map.tsv"
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -ne 0 ]
  [[ "$output" == *"test missing from map"* ]]
  setup
  local last_row
  last_row=$(tail -n 1 "$FIXTURE/tests/agent-go-map.tsv")
  printf '%s\n' "$last_row" >> "$FIXTURE/tests/agent-go-map.tsv"
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -ne 0 ]
  [[ "$output" == *"duplicate entry"* ]]
}

@test "parity rejects invalid rounds classifications and states" {
  local replacement
  for replacement in 's/R56/R00/' 's/R56/R57/' 's/blackbox/unknown/' 's/planned$/unknown/'; do
    setup
    sed -i "$replacement" "$FIXTURE/tests/agent-go-map.tsv"
    run env CHECK_ROOT="$FIXTURE" "$TOOL"
    [ "$status" -ne 0 ]
    [[ "$output" == *"invalid "* ]]
  done
}

@test "parity rejects an extra ledger identity even when total counts match" {
  sed -i 's/planned case/nonexistent case/' "$FIXTURE/tests/agent-go-map.tsv"
  # Restore forward coverage with a duplicate Bats title; the reverse lookup must fail.
  printf '@test "%s" { true; }\n' 'active case' 'active case' 'done case' > "$FIXTURE/tests/unit/sample.bats"
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -ne 0 ]
  [[ "$output" == *"not found in Bats"* ]]
}

@test "parity requires mappings for active and done rows but permits planned placeholders" {
  sed -i 's/TestActive/-/' "$FIXTURE/tests/agent-go-map.tsv"
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -ne 0 ]
  [[ "$output" == *"Go test mapping"* ]]
  setup
  sed -i 's/TestDone/-/' "$FIXTURE/tests/agent-go-map.tsv"
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -ne 0 ]
  [[ "$output" == *"Go test mapping"* ]]
}

@test "parity validates real top-level Go tests and comma-separated subtest mappings" {
  sed -i 's/TestActive/TestActive\/subtest,TestDone/' "$FIXTURE/tests/agent-go-map.tsv"
  write_evidence
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -eq 0 ]
  local mapping
  for mapping in TestMissing TestActiveMissing 'TestActive|TestDone' 'TestActive,'; do
    setup
    sed -i "s/TestActive/$mapping/" "$FIXTURE/tests/agent-go-map.tsv"
    run env CHECK_ROOT="$FIXTURE" "$TOOL"
    [ "$status" -ne 0 ]
    [[ "$output" == *"Go test mapping"* ]]
  done
}

@test "parity rejects incorrect selected executed failed and skipped counts" {
  local expression
  for expression in '.selected_count=2' '.executed_count=0' '.executed_count=2' '.failures=1' '.skips=1' '.selected_count="1"' '.executed_count=-1'; do
    setup
    alter_evidence "$expression"
    run env CHECK_ROOT="$FIXTURE" "$TOOL"
    [ "$status" -ne 0 ]
    [[ "$output" == *"evidence"* ]]
  done
}

@test "parity rejects missing malformed wrong-lane or stale evidence" {
  rm "$EVIDENCE"
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -ne 0 ]
  [[ "$output" == *"evidence"* ]]
  printf '{' > "$EVIDENCE"
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -ne 0 ]
  [[ "$output" == *"evidence"* ]]
  write_evidence
  alter_evidence '.lane="oracle"'
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -ne 0 ]
  [[ "$output" == *"evidence"* ]]
  write_evidence
  printf '# changed helper\n' > "$FIXTURE/tests/unit/helper.bash"
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -ne 0 ]
  [[ "$output" == *"hash"* ]]
  write_evidence
  sed -i 's/R56/R55/' "$FIXTURE/tests/agent-go-map.tsv"
  run env CHECK_ROOT="$FIXTURE" "$TOOL"
  [ "$status" -ne 0 ]
  [[ "$output" == *"hash"* ]]
}

@test "parity permits absent evidence only when every blackbox row is planned" {
  sed -i 's/TestActive\tactive/-\tplanned/' "$FIXTURE/tests/agent-go-map.tsv"
  rm "$EVIDENCE"
  run env CHECK_ROOT="$FIXTURE" "$TOOL" --quiet
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  run env CHECK_ROOT="$FIXTURE" "$TOOL" --evidence "$EVIDENCE"
  [ "$status" -ne 0 ]
  [[ "$output" == *"evidence"* ]]
}

@test "parity rejects unknown flags" {
  run env CHECK_ROOT="$FIXTURE" "$TOOL" --unknown
  [ "$status" -ne 0 ]
  [[ "$output" == *"unknown option"* ]]
}
