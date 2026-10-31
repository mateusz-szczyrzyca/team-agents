#!/usr/bin/env bats
# Tests that opencode disabled MCP configuration matches official schema:
# When disabled, McpConfig requires { enabled: false } with no additional properties.

load helpers

# setup_isolation sets up mock tasks
setup_isolation() {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  export AGENTBUS_BIN="$HOME/.local/bin/agentbus"
}

create_isolation_task() {
  local id=$1 kind=$2 inv_team=${3:-} type=${4:-opencode}
  local dir="$AGENTS_HOME/tasks/$id"
  mkdir -p "$dir"
  cat > "$dir/meta.env" << EOF
ID=$id
AGENT=inv
KIND=$kind
TYPE=$type
INVESTIGATION_TEAM=$inv_team
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
BRANCH=agents/inv/test
BASE=main
SESSION_ID=sid-test
RUN=1
ROUND=0
STATUS=running
EOF
  touch "$dir/prompt-1.md"
}

@test "opencode config for independent investigation emits strictly valid disabled mcp object with enabled false only" {
  setup_isolation
  create_isolation_task "inv-child-opencode-schema" "investigation" "team-parent-1" "opencode"
  meta_load "inv-child-opencode-schema"
  run opencode_config "inv-child-opencode-schema"
  [ "$status" -eq 0 ]
  run jq -e '.mcp.agentbus | keys == ["enabled"] and .enabled == false' <<< "$output"
  [ "$status" -eq 0 ]
}
