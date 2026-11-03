#!/usr/bin/env bats
# Tests for tools/agent-bats.sh: Lane B candidate execution, staging, and harness safety.

setup() {
  REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)
  TOOL="$REPO_ROOT/tools/agent-bats.sh"
}

@test "agent-bats rejects deliberately broken candidate" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_broken"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  # Create a mock map with one active blackbox test
  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/sample.bats	always passes	blackbox	R04	TestAlwaysPasses	active
EOF

  cat > "$mock_repo/tests/unit/sample.bats" <<'EOF'
@test "always passes" {
  run "$REPO_ROOT/bin/agent" --help
  [ "$status" -eq 0 ]
}
EOF

  # Broken candidate exits 1 on everything
  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 1
EOF
  chmod +x "$mock_repo/build/agent-go"

  run env CHECK_ROOT="$mock_repo" "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -ne 0 ]
}

@test "agent-bats stages candidate copy as real binary without symlinks" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_symlink"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/sample.bats	check not symlink	blackbox	R04	TestNotSymlink	active
EOF

  cat > "$mock_repo/tests/unit/sample.bats" <<'EOF'
@test "check not symlink" {
  local agent_bin="$BATS_TEST_DIRNAME/../../bin/agent"
  [ -f "$agent_bin" ]
  [ ! -L "$agent_bin" ]
  [ -x "$agent_bin" ]
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  run "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
}

@test "agent-bats applies exact filter for mixed files and ignores planned tests" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_filter"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/mixed.bats	active blackbox test	blackbox	R04	TestActive	active
tests/unit/mixed.bats	planned blackbox test	blackbox	R04	-	planned
tests/unit/mixed.bats	active source test	source	R04	TestActiveSource	active
EOF

  cat > "$mock_repo/tests/unit/mixed.bats" <<'EOF'
@test "active blackbox test" {
  true
}
@test "planned blackbox test" {
  # Should not be executed
  echo "PLANNED_EXECUTED" >&2
  false
}
@test "active source test" {
  # Should not be executed in Lane B
  echo "SOURCE_EXECUTED" >&2
  false
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  run "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
  [[ "$output" != *"PLANNED_EXECUTED"* ]]
  [[ "$output" != *"SOURCE_EXECUTED"* ]]
}

@test "agent-bats passes when no blackbox tests are active or done" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_empty"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/sample.bats	planned test	blackbox	R05	-	planned
EOF

  cat > "$mock_repo/tests/unit/sample.bats" <<'EOF'
@test "planned test" {
  false
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  run "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
  [[ "$output" =~ "0 active/done blackbox tests" ]]
}
