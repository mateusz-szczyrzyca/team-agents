#!/usr/bin/env bats
# Tests for tools/agent-bats.sh: regex escaping, oracle scheduling, and process cleanup.

setup() {
  REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)
  TOOL="$REPO_ROOT/tools/agent-bats.sh"
}

@test "agent-bats escapes extended regex metacharacters including pipe and braces" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_ere"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  # Active test has pipe: "foo|bar" (which fails). Planned test is "foo" (which passes).
  # If | is unescaped, ^(foo|bar)$ matches "foo", succeeding falsely.
  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/sample.bats	foo|bar	blackbox	R04	TestFooBar	active
tests/unit/sample.bats	foo	blackbox	R04	TestFoo	planned
EOF

  cat > "$mock_repo/tests/unit/sample.bats" <<'EOF'
@test "foo" {
  true
}

@test "foo|bar" {
  false
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  run env CHECK_ROOT="$mock_repo" "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  # When pipe is properly escaped, "foo|bar" is executed and fails.
  [ "$status" -ne 0 ]
}

@test "agent-bats matches test titles with literal braces" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_braces"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/sample.bats	title with {2} braces	blackbox	R04	TestBraces	active
EOF

  cat > "$mock_repo/tests/unit/sample.bats" <<'EOF'
@test "title with {2} braces" {
  true
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  run env CHECK_ROOT="$mock_repo" "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
}

@test "agent-bats preserves relative test identities for oracle serial scheduling" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_serial"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/bin" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"

  cat > "$mock_repo/bin/agent" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/bin/agent"

  touch "$mock_repo/tests/unit/serial.bats"
  touch "$mock_repo/tests/unit/parallel.bats"

  # Stub unit-bats.sh that asserts BATS_FILES contains root-relative paths matching BATS_SERIAL
  cat > "$mock_repo/tools/unit-bats.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
matched=0
for f in $BATS_FILES; do
  for s in $BATS_SERIAL; do
    if [[ "$f" == "$s" ]]; then
      matched=1
    fi
  done
done
if (( matched == 0 )); then
  echo "serial file not matched against BATS_SERIAL: BATS_FILES='$BATS_FILES' BATS_SERIAL='$BATS_SERIAL'" >&2
  exit 1
fi
exit 0
EOF
  chmod +x "$mock_repo/tools/unit-bats.sh"

  run env BATS_SERIAL="tests/unit/serial.bats" "$mock_repo/tools/agent-bats.sh" --lane oracle --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
}

@test "agent-bats cleanup handles pkill return status explicitly" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_pkill"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools" "$mock_repo/bin_fake"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/sample.bats	dummy	blackbox	R04	TestDummy	active
EOF

  cat > "$mock_repo/tests/unit/sample.bats" <<'EOF'
@test "dummy" {
  true
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  # Stub pkill that exits 2
  cat > "$mock_repo/bin_fake/pkill" <<'EOF'
#!/bin/sh
echo "pkill: simulated error" >&2
exit 2
EOF
  chmod +x "$mock_repo/bin_fake/pkill"

  run env PATH="$mock_repo/bin_fake:$PATH" CHECK_ROOT="$mock_repo" "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [[ "$output" == *"pkill exited with status 2"* || "$output" == *"pkill: simulated error"* ]]
}

@test "make unit-bats executes through staged oracle lane" {
  run make -n unit-bats
  [ "$status" -eq 0 ]
  [[ "$output" == *"tools/agent-bats.sh --lane oracle"* ]]
}

@test "agent-bats Lane A supports sourcing and function extraction from staged oracle" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_src_extract"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/bin" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/bin/agent" <<'EOF'
#!/bin/sh
node_transition() {
  echo "oracle_transition_ok"
}
EOF
  chmod +x "$mock_repo/bin/agent"

  cat > "$mock_repo/tests/unit/source_extract.bats" <<'EOF'
@test "extract and source" {
  extracted=$(sed -n '/^node_transition() {/,/^}/p' "$BATS_TEST_DIRNAME/../../bin/agent")
  eval "$extracted"
  [ "$(node_transition)" = "oracle_transition_ok" ]

  REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)
  source "$REPO_ROOT/bin/agent"
  [ "$(node_transition)" = "oracle_transition_ok" ]
}
EOF

  run "$mock_repo/tools/agent-bats.sh" --lane oracle --repo-root "$mock_repo" tests/unit/source_extract.bats
  [ "$status" -eq 0 ]
}

@test "agent-bats fails when executed count differs from selected count" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_count_mismatch"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  # Map file selects a test that does not exist in sample.bats
  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/sample.bats	nonexistent test title	blackbox	R04	TestNonexistent	active
EOF

  cat > "$mock_repo/tests/unit/sample.bats" <<'EOF'
@test "existing other test" {
  true
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  run env CHECK_ROOT="$mock_repo" "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -ne 0 ]
  [[ "$output" == *"executed count (0) does not match selected count (1)"* ]]
}

@test "agent-bats exact filter anchoring isolates active test from prefix and suffix matches" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_anchoring"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/sample.bats	foo	blackbox	R04	TestFoo	active
tests/unit/sample.bats	prefix foo	blackbox	R04	TestPrefixFoo	planned
tests/unit/sample.bats	foo suffix	blackbox	R04	TestFooSuffix	planned
EOF

  cat > "$mock_repo/tests/unit/sample.bats" <<'EOF'
@test "foo" {
  true
}

@test "prefix foo" {
  false
}

@test "foo suffix" {
  false
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  run env CHECK_ROOT="$mock_repo" "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"ok 1 foo"* ]]
  [[ "$output" != *"prefix foo"* ]]
  [[ "$output" != *"foo suffix"* ]]
}

@test "exact-filter anchoring prevents partial test name matches" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_anchor"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  # Active test is "foo"; planned tests are "foo suffix" and "prefix foo".
  # Without anchoring (^...$), the filter would match partial strings.
  # This test verifies that only the exact "foo" is executed, not the partial matches.
  cat > "$mock_repo/tests/agent-go-map.tsv" <<'EOF'
bats_file	test_name	classification	implementation_round	go_test_or_subtests	state
tests/unit/sample.bats	foo	blackbox	R04	TestFoo	active
tests/unit/sample.bats	foo suffix	blackbox	R04	TestFooSuffix	planned
tests/unit/sample.bats	prefix foo	blackbox	R04	TestPrefixFoo	planned
EOF

  cat > "$mock_repo/tests/unit/sample.bats" <<'EOF'
@test "foo" {
  true
}

@test "foo suffix" {
  echo "should not execute"
  false
}

@test "prefix foo" {
  echo "should not execute"
  false
}
EOF

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  run env CHECK_ROOT="$mock_repo" "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"1..1"* ]]
  [[ "$output" == *"ok 1 foo"* ]]
}

@test "agent-bats rejects in-repository TMPDIR to prevent recursive staging" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_tmpdir"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools" "$mock_repo/tmp"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  run env TMPDIR="$mock_repo/tmp" "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -ne 0 ]
  [[ "$output" == *"TMPDIR cannot be inside repository root"* ]]
}

@test "agent-bats fails cleanly when external TMPDIR cannot be resolved" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_unresolvable_tmpdir"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  local bad_tmpdir="$BATS_TEST_TMPDIR/nonexistent_external_tmpdir/subdir"
  run bash -c "cd '$mock_repo' && env TMPDIR='$bad_tmpdir' '$mock_repo/tools/agent-bats.sh' --repo-root '$mock_repo'"
  [ "$status" -ne 0 ]
  [[ "$output" == *"failed to resolve TMPDIR"* ]]
  [[ "$output" != *"TMPDIR cannot be inside repository root"* ]]
}

@test "agent-bats rejects nonexistent external TMPDIR with appropriate error, not in-repo error" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_external_tmpdir"
  local nonexistent_tmpdir="/tmp/opencode/nonexistent-tmpdir-$$"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/build" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/build/agent-go" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$mock_repo/build/agent-go"

  # This TMPDIR does not exist and is external to the repo.
  # The error should be about TMPDIR not existing, not "inside repository root".
  run env TMPDIR="$nonexistent_tmpdir" "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -ne 0 ]
  # Should NOT report as in-repository (which was the bug)
  [[ "$output" != *"TMPDIR cannot be inside repository root"* ]]
  # Should report TMPDIR error clearly
  [[ "$output" == *"TMPDIR"* ]]
}

@test "agent-bats scrubs hostile inherited environment variables in staged root (candidate lane)" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_env_scrub_candidate"
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
  [[ -z ${AGENT_GH_COORDINATOR+x} ]] || exit 1
  [[ -z ${AGENT_GH_OWN_PR+x} ]] || exit 1
  [[ -z ${AGENT_GH_SELF+x} ]] || exit 1
  [[ -z ${AGENT_GH_TEAM+x} ]] || exit 1
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
  export AGENT_GH_COORDINATOR="hostile-lead"
  export AGENT_GH_OWN_PR="999"
  export AGENT_GH_SELF="hostile-self"
  export AGENT_GH_TEAM="hostile-team"
  export AGENT_GITHUB_NAME_COLORS="hostile=red"
  export OPENCODE_CONFIG_CONTENT="hostile-config"

  run "$mock_repo/tools/agent-bats.sh" --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
}

@test "agent-bats scrubs hostile inherited environment variables in staged root (oracle lane)" {
  local mock_repo="$BATS_TEST_TMPDIR/mock_repo_env_scrub_oracle"
  mkdir -p "$mock_repo/tests/unit" "$mock_repo/bin" "$mock_repo/tools"
  cp "$TOOL" "$mock_repo/tools/"
  cp "$REPO_ROOT/tools/unit-bats.sh" "$mock_repo/tools/"

  cat > "$mock_repo/bin/agent" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
  chmod +x "$mock_repo/bin/agent"

  cat > "$mock_repo/tests/unit/env_scrub.bats" <<'EOF'
@test "oracle inherited env vars cleared" {
  # Verify that hostile inherited variables are not present in the staged oracle test
  [[ -z ${AGENTBUS_BIN+x} ]] || exit 1
  [[ -z ${AGENT_GH_BUS_DIR+x} ]] || exit 1
  [[ -z ${AGENT_GITHUB_NAME_COLORS+x} ]] || exit 1
  [[ -z ${OPENCODE_CONFIG_CONTENT+x} ]] || exit 1
  [[ -z ${AGENT_GH_TEAM+x} ]] || exit 2
  [[ -z ${AGENT_GH_SELF+x} ]] || exit 3
  [[ -z ${AGENT_GH_COORDINATOR+x} ]] || exit 4
  [[ -z ${AGENT_GH_OWN_PR+x} ]] || exit 5
}
EOF

  # Run with hostile inherited variables set
  export AGENTBUS_BIN="/hostile/agentbus"
  export AGENT_GH_BUS_DIR="/hostile/bus"
  export AGENT_GITHUB_NAME_COLORS="hostile=red"
  export OPENCODE_CONFIG_CONTENT="hostile-config"
  export AGENT_GH_TEAM="fake-team"
  export AGENT_GH_SELF="fake-self"
  export AGENT_GH_COORDINATOR="fake-coordinator"
  export AGENT_GH_OWN_PR="fake-pr"

  run "$mock_repo/tools/agent-bats.sh" --lane oracle --repo-root "$mock_repo"
  [ "$status" -eq 0 ]
}
