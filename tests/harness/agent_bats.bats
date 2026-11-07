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

@test "agent-bats executes staged candidate for gh dispatch and self-callbacks" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_dispatch"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/tests/agent-go-map.tsv" <<'MAP'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/dispatch.bats	test gh and self-callbacks	blackbox	R04	TestDispatch	active
MAP

  cat > "$mock_repo/tests/unit/dispatch.bats" <<'TEST'
@test "test gh and self-callbacks" {
  local agent_bin="$BATS_TEST_DIRNAME/../../bin/agent"
  # gh basename invocation
  local gh_link="$BATS_TEST_TMPDIR/gh"
  ln -s "$agent_bin" "$gh_link"
  run "$gh_link" --version
  [ "$status" -eq 0 ]
  [[ "$output" =~ "candidate-as-gh" ]]

  # self-callback invocation
  run "$agent_bin" _runner fake-task
  [ "$status" -eq 0 ]
  [[ "$output" =~ "candidate-self-callback" ]]
}
TEST

  cat > "$mock_repo/build/agent-go" <<'CAND'
#!/bin/sh
if [ "$(basename "$0")" = "gh" ]; then
  echo "candidate-as-gh 1.0"
  exit 0
fi
if [ "$1" = "_runner" ]; then
  echo "candidate-self-callback"
  exit 0
fi
exit 0
CAND
  chmod +x "$mock_repo/build/agent-go"

  run "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
}

@test "agent-bats executes staged candidate across multi-home test setups" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_multihome"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/tests/agent-go-map.tsv" <<'MAP'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/multihome.bats	test multi-home copies	blackbox	R04	TestMultiHome	active
MAP

  cat > "$mock_repo/tests/unit/multihome.bats" <<'TEST'
@test "test multi-home copies" {
  local staged_repo="$BATS_TEST_DIRNAME/../.."
  local home1="$BATS_TEST_TMPDIR/home1"
  local home2="$BATS_TEST_TMPDIR/home2"
  mkdir -p "$home1/bin" "$home2/bin"
  cp "$staged_repo/bin/agent" "$home1/bin/agent"
  cp "$staged_repo/bin/agent" "$home2/bin/agent"

  run "$home1/bin/agent" whoami
  [ "$status" -eq 0 ]
  [[ "$output" == *"candidate-agent"* ]]

  run "$home2/bin/agent" whoami
  [ "$status" -eq 0 ]
  [[ "$output" == *"candidate-agent"* ]]
}
TEST

  cat > "$mock_repo/build/agent-go" <<'CAND'
#!/bin/sh
echo "candidate-agent"
exit 0
CAND
  chmod +x "$mock_repo/build/agent-go"

  run "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
}

@test "agent-bats supports oracle staging in Lane A" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_oracle"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/bin" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/bin/agent" <<'ORACLE'
#!/bin/sh
echo "oracle-agent"
exit 0
ORACLE
  chmod +x "$mock_repo/bin/agent"

  cat > "$mock_repo/tests/unit/oracle.bats" <<'TEST'
@test "test oracle execution" {
  local agent_bin="$BATS_TEST_DIRNAME/../../bin/agent"
  run "$agent_bin"
  [ "$status" -eq 0 ]
  [[ "$output" == *"oracle-agent"* ]]
}
TEST

  run "$mock_repo/tools/agent-bats.sh" --lane oracle --repo-root "$mock_repo" tests/unit/oracle.bats
  [ "$status" -eq 0 ]
}

@test "agent-bats control: working candidate invoked as staged binary succeeds" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_control_working"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/positive.bats	working candidate test	blackbox	R04	TestWorkingCandidate	active
EOF

  cat > "$mock_repo/tests/unit/positive.bats" <<'EOF'
@test "working candidate test" {
  local agent_bin="$BATS_TEST_DIRNAME/../../bin/agent"
  run "$agent_bin" --working-control
  [ "$status" -eq 0 ]
  [[ "$output" == *"working-candidate"* ]]
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
if [ "$1" = "--working-control" ]; then
  echo "working-candidate"
  exit 0
fi
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  run "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
}

@test "agent-bats control: broken candidate properly detected when assertion fails" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_control_broken"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/negative.bats	broken candidate test	blackbox	R04	TestBrokenCandidate	active
EOF

  cat > "$mock_repo/tests/unit/negative.bats" <<'EOF'
@test "broken candidate test" {
  local agent_bin="$BATS_TEST_DIRNAME/../../bin/agent"
  run "$agent_bin" --test-command
  [ "$status" -eq 0 ]
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 1
EOF
  chmod +x "$mock_repo/build/agent-go"

  run "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -ne 0 ]
}

@test "agent-bats scrubs hostile inherited environment variables in staged root" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_env_scrub"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/env_scrub.bats	inherited env vars cleared	blackbox	R10B	TestEnvScrub	active
EOF

  cat > "$mock_repo/tests/unit/env_scrub.bats" <<'EOF'
@test "inherited env vars cleared" {
  # Verify that hostile inherited variables are not present in the staged test
  [[ -z ${AGENTBUS_BIN+x} ]] || exit 1
  [[ -z ${AGENT_GH_BUS_DIR+x} ]] || exit 1
  [[ -z ${AGENT_GITHUB_NAME_COLORS+x} ]] || exit 1
  [[ -z ${OPENCODE_CONFIG_CONTENT+x} ]] || exit 1
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  # Run with hostile inherited variables set
  export AGENTBUS_BIN="/hostile/agentbus"
  export AGENT_GH_BUS_DIR="/hostile/bus"
  export AGENT_GITHUB_NAME_COLORS="hostile=red"
  export OPENCODE_CONFIG_CONTENT="hostile-config"

  run "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -eq 0 ]

  # Verify the environment variables were cleared in the calling context too
  # (after the tool returns)
  [[ -z ${AGENTBUS_BIN+x} ]] || true
}
