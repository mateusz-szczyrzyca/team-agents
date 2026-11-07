#!/usr/bin/env bats
# Regression tests for fixture environment isolation.
# Verifies that test helpers properly isolate from inherited environment variables
# that would cause test infrastructure failures when running in ordinary (non-sanitized) environments.

load helpers

@test "fixture isolation: AGENTBUS_BIN is bound to fixture, not inherited" {
  # setup() is called automatically by bats; verify AGENTBUS_BIN points to fixture
  [[ "$AGENTBUS_BIN" == "$HOME/.local/bin/agentbus" ]]
  # Verify the binary actually exists at the fixture location
  [[ -f "$AGENTBUS_BIN" ]]
}

@test "fixture isolation: AGENT_GH_BUS_DIR is bound to fixture, not inherited" {
  # setup() is called automatically; verify AGENT_GH_BUS_DIR points to fixture
  [[ "$AGENT_GH_BUS_DIR" == "$AGENTS_HOME/bus" ]]
  # Verify the directory exists at the fixture location
  [[ -d "$AGENT_GH_BUS_DIR" ]]
}

@test "fixture isolation: AGENT_GITHUB_NAME_COLORS is cleared in fixture" {
  # setup() clears this to prevent inherited color config from interfering
  [[ -z "$AGENT_GITHUB_NAME_COLORS" ]]
}

@test "fixture isolation: OPENCODE_CONFIG_CONTENT is cleared in fixture" {
  # setup() clears this to prevent inherited config from interfering
  [[ -z "$OPENCODE_CONFIG_CONTENT" ]]
}

@test "fixture isolation: setup overrides hostile inherited environment variables" {
  run bash -e -c '
    export BATS_TEST_TMPDIR="'"$BATS_TEST_TMPDIR"'/hostile"
    export AGENTBUS_BIN="/hostile/nonexistent/agentbus"
    export AGENT_GH_BUS_DIR="/hostile/nonexistent/bus"
    export AGENT_GITHUB_NAME_COLORS="hostile=red"
    export OPENCODE_CONFIG_CONTENT="hostile-content"
    export BATS_TEST_DIRNAME="'"$BATS_TEST_DIRNAME"'"
    export REPO_ROOT="'"$REPO_ROOT"'"
    source "'"$BATS_TEST_DIRNAME"'/helpers.bash"
    setup
    [[ "$AGENTBUS_BIN" == "$HOME/.local/bin/agentbus" ]] || exit 1
    [[ "$AGENT_GH_BUS_DIR" == "$AGENTS_HOME/bus" ]] || exit 2
    [[ -z ${AGENT_GITHUB_NAME_COLORS+x} ]] || exit 3
    [[ -z ${OPENCODE_CONFIG_CONTENT+x} ]] || exit 4
  '
  [ "$status" -eq 0 ]
}
