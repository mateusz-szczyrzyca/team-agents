#!/usr/bin/env bats
# Tests for tools/check-agent-test-map.sh ledger validation.

setup() {
  REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)
  TOOL="$REPO_ROOT/tools/check-agent-test-map.sh"
}

@test "check-agent-test-map validates repository ledger successfully" {
  run "$TOOL" "$REPO_ROOT/tests/agent-go-map.tsv"
  [ "$status" -eq 0 ]
  [[ "$output" =~ "check-agent-test-map: OK" ]]
  [[ "$output" =~ "total tests:    480" ]]
  [[ "$output" =~ "blackbox cases: 359" ]]
  [[ "$output" =~ "source cases:   121" ]]
  [[ "$output" =~ "planned:        "[0-9]+ ]]
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

@test "check-agent-test-map quiet flag suppresses output on success" {
  run "$TOOL" --quiet "$REPO_ROOT/tests/agent-go-map.tsv"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}
