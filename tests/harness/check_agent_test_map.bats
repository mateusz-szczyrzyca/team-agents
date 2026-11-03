#!/usr/bin/env bats
# Tests for tools/check-agent-test-map.sh ledger validation.

setup() {
  REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)
  TOOL="$REPO_ROOT/tools/check-agent-test-map.sh"
}

@test "check-agent-test-map validates repository ledger successfully" {
  local expected_tests expected_planned
  expected_tests=$(grep -c '^[[:space:]]*@test[[:space:]]' "$REPO_ROOT"/tests/unit/*.bats | awk -F: '{s+=$2} END {print s}')
  expected_planned=$(awk -F '\t' '$6 == "planned" {n++} END {print n+0}' "$REPO_ROOT/tests/agent-go-map.tsv")
  run "$TOOL" "$REPO_ROOT/tests/agent-go-map.tsv"
  [ "$status" -eq 0 ]
  [[ "$output" =~ "check-agent-test-map: OK" ]]
  [[ "$output" =~ "total tests:    "$expected_tests ]]
  [[ "$output" =~ "blackbox cases: "[0-9]+ ]]
  [[ "$output" =~ "source cases:   "[0-9]+ ]]
  [[ "$output" =~ "planned:        "$expected_planned ]]
}

@test "check-agent-test-map supports planned, active, and done status transitions" {
  local mock_root="$BATS_TEST_TMPDIR/mock_repo"
  mkdir -p "$mock_root/tests/unit"
  printf '@test "%s" { true; }\n' "case 1" "case 2" "case 3" > "$mock_root/tests/unit/sample.bats"
  local map="$mock_root/map.tsv"
  printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\n' > "$map"
  printf 'tests/unit/sample.bats\tcase 1\tblackbox\tR01\tTestOne\tplanned\n' >> "$map"
  printf 'tests/unit/sample.bats\tcase 2\tblackbox\tR02\tTestTwo\tactive\n' >> "$map"
  printf 'tests/unit/sample.bats\tcase 3\tsource\tR03\tTestThree\tdone\n' >> "$map"

  run env CHECK_ROOT="$mock_root" "$TOOL" "$map"
  [ "$status" -eq 0 ]
  [[ "$output" =~ "planned:        1" ]]
  [[ "$output" =~ "active:         1" ]]
  [[ "$output" =~ "done:           1" ]]
}

@test "check-agent-test-map fails on non-existent map file" {
  run "$TOOL" "$BATS_TEST_TMPDIR/non_existent.tsv"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "map file not found" ]]
}

@test "check-agent-test-map rejects invalid round format" {
  local bad_map="$BATS_TEST_TMPDIR/bad_round.tsv"
  printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\n' > "$bad_map"
  printf 'tests/unit/meta.bats\ttest name\tblackbox\tROUND1\tTestGo\tplanned\n' >> "$bad_map"
  run "$TOOL" "$bad_map"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "invalid round format" ]]
}

@test "check-agent-test-map rejects rounds outside R01-R56 range" {
  for r in R00 R57 R99; do
    local bad_map="$BATS_TEST_TMPDIR/bad_round_${r}.tsv"
    printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\n' > "$bad_map"
    printf 'tests/unit/meta.bats\ttest name\tblackbox\t%s\tTestGo\tplanned\n' "$r" >> "$bad_map"
    run "$TOOL" "$bad_map"
    [ "$status" -ne 0 ]
    local expected="invalid round format '$r'"
    [[ "$output" =~ $expected ]]
  done
}

@test "check-agent-test-map fails when unit tests directory is missing" {
  local empty_root="$BATS_TEST_TMPDIR/no_unit_dir"
  mkdir -p "$empty_root"
  local map="$empty_root/map.tsv"
  printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\n' > "$map"
  run env CHECK_ROOT="$empty_root" "$TOOL" "$map"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "unit tests directory not found" ]]
}

@test "check-agent-test-map fails when unit tests directory contains no Bats files" {
  local empty_root="$BATS_TEST_TMPDIR/empty_unit_dir"
  mkdir -p "$empty_root/tests/unit"
  local map="$empty_root/map.tsv"
  printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\n' > "$map"
  run env CHECK_ROOT="$empty_root" "$TOOL" "$map"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "no Bats test files found" ]]
}

@test "check-agent-test-map rejects duplicate entry" {
  local mock_root="$BATS_TEST_TMPDIR/mock_repo_dup"
  mkdir -p "$mock_root/tests/unit"
  printf '@test "%s" { true; }\n' "case 1" > "$mock_root/tests/unit/sample.bats"
  local map="$mock_root/map.tsv"
  printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\n' > "$map"
  printf 'tests/unit/sample.bats\tcase 1\tblackbox\tR01\tTestOne\tplanned\n' >> "$map"
  printf 'tests/unit/sample.bats\tcase 1\tblackbox\tR01\tTestOne\tplanned\n' >> "$map"
  run env CHECK_ROOT="$mock_root" "$TOOL" "$map"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "duplicate entry" ]]
}

@test "check-agent-test-map rejects map when a unit test is missing" {
  local mock_root="$BATS_TEST_TMPDIR/mock_repo_missing"
  mkdir -p "$mock_root/tests/unit"
  printf '@test "%s" { true; }\n' "case 1" "case 2" > "$mock_root/tests/unit/sample.bats"
  local map="$mock_root/map.tsv"
  printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\n' > "$map"
  printf 'tests/unit/sample.bats\tcase 1\tblackbox\tR01\tTestOne\tplanned\n' >> "$map"
  run env CHECK_ROOT="$mock_root" "$TOOL" "$map"
  [ "$status" -ne 0 ]
  [[ "$output" == *"test missing from map: tests/unit/sample.bats :: case 2"* ]]
}

@test "check-agent-test-map rejects map with extra entry not in unit tests" {
  local mock_root="$BATS_TEST_TMPDIR/mock_repo_extra"
  mkdir -p "$mock_root/tests/unit"
  printf '@test "%s" { true; }\n' "case 1" > "$mock_root/tests/unit/sample.bats"
  local map="$mock_root/map.tsv"
  printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\n' > "$map"
  printf 'tests/unit/sample.bats\tcase 1\tblackbox\tR01\tTestOne\tplanned\n' >> "$map"
  printf 'tests/unit/sample.bats\tcase 2\tblackbox\tR01\tTestTwo\tplanned\n' >> "$map"
  run env CHECK_ROOT="$mock_root" "$TOOL" "$map"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "does not match Bats tests count" ]]
}

@test "check-agent-test-map rejects row with missing required column" {
  local mock_root="$BATS_TEST_TMPDIR/mock_repo_miscol"
  mkdir -p "$mock_root/tests/unit"
  printf '@test "%s" { true; }\n' "case 1" > "$mock_root/tests/unit/sample.bats"
  local map="$mock_root/map.tsv"
  printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\n' > "$map"
  printf 'tests/unit/sample.bats\tcase 1\tblackbox\tR01\tTestOne\n' >> "$map"
  run env CHECK_ROOT="$mock_root" "$TOOL" "$map"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "missing required column" ]]
}

@test "check-agent-test-map rejects row with unexpected extra columns" {
  local mock_root="$BATS_TEST_TMPDIR/mock_repo_extracol"
  mkdir -p "$mock_root/tests/unit"
  printf '@test "%s" { true; }\n' "case 1" > "$mock_root/tests/unit/sample.bats"
  local map="$mock_root/map.tsv"
  printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\textra\n' > "$map"
  printf 'tests/unit/sample.bats\tcase 1\tblackbox\tR01\tTestOne\tplanned\tspurious\n' >> "$map"
  run env CHECK_ROOT="$mock_root" "$TOOL" "$map"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "unexpected extra columns" ]]
}

@test "check-agent-test-map rejects invalid classification" {
  local mock_root="$BATS_TEST_TMPDIR/mock_repo_badclass"
  mkdir -p "$mock_root/tests/unit"
  printf '@test "%s" { true; }\n' "case 1" > "$mock_root/tests/unit/sample.bats"
  local map="$mock_root/map.tsv"
  printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\n' > "$map"
  printf 'tests/unit/sample.bats\tcase 1\tunknown\tR01\tTestOne\tplanned\n' >> "$map"
  run env CHECK_ROOT="$mock_root" "$TOOL" "$map"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "invalid classification" ]]
}

@test "check-agent-test-map rejects invalid status" {
  local mock_root="$BATS_TEST_TMPDIR/mock_repo_badstat"
  mkdir -p "$mock_root/tests/unit"
  printf '@test "%s" { true; }\n' "case 1" > "$mock_root/tests/unit/sample.bats"
  local map="$mock_root/map.tsv"
  printf 'bats_file\ttest_name\tclassification\tround\tgo_test\tstatus\n' > "$map"
  printf 'tests/unit/sample.bats\tcase 1\tblackbox\tR01\tTestOne\tinvalid\n' >> "$map"
  run env CHECK_ROOT="$mock_root" "$TOOL" "$map"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "invalid status" ]]
}

@test "check-agent-test-map rejects unknown option" {
  run "$TOOL" --invalid-flag "$REPO_ROOT/tests/agent-go-map.tsv"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "unknown option" ]]
}

@test "check-agent-test-map rejects unexpected argument" {
  run "$TOOL" "$REPO_ROOT/tests/agent-go-map.tsv" "extra_arg"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "unexpected argument" ]]
}

@test "check-agent-test-map quiet flag suppresses output on success" {
  run "$TOOL" --quiet "$REPO_ROOT/tests/agent-go-map.tsv"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "repository ledger validation accepts activation and completion" {
  local staged_root="$BATS_TEST_TMPDIR/repository"
  mkdir -p "$staged_root/tests/unit" "$staged_root/tests/harness" "$staged_root/tools"
  cp "$REPO_ROOT"/tests/unit/*.bats "$staged_root/tests/unit/"
  cp "$TOOL" "$staged_root/tools/"
  cp "$BATS_TEST_FILENAME" "$staged_root/tests/harness/"

  local lifecycle_status
  for lifecycle_status in active 'done'; do
    # Change one real row while preserving every exact repository test key.
    awk -F '\t' -v OFS='\t' -v state="$lifecycle_status" '
      !changed && $6 ~ /^(planned|active|done)$/ { $5 = "TestMigrationLifecycle"; $6 = state; changed = 1 }
      { print }
      END { if (!changed) exit 1 }
    ' "$REPO_ROOT/tests/agent-go-map.tsv" > "$staged_root/tests/agent-go-map.tsv"

    run env CHECK_ROOT="$staged_root" "$staged_root/tools/check-agent-test-map.sh"
    [ "$status" -eq 0 ]
    [[ "$output" =~ "$lifecycle_status:"[[:space:]]+[1-9][0-9]* ]]

    run bats --filter '^check-agent-test-map validates repository ledger successfully$' \
      "$staged_root/tests/harness/check_agent_test_map.bats"
    printf '%s\n' "$output"
    [ "$status" -eq 0 ]
    [[ "$output" == *"ok 1 check-agent-test-map validates repository ledger successfully"* ]]
  done
}
