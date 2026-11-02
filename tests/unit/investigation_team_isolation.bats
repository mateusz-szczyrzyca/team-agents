#!/usr/bin/env bats
# Tests for investigation team child run MCP isolation:
# Investigators working in an investigation team have strict independence and NO agentbus MCP tools.
# Legacy investigations continue to configure agentbus MCP for all 4 CLI types.

load helpers
bats_require_minimum_version 1.5.0

# setup_isolation sets up mock tasks and stubs for CLI execution
setup_isolation() {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"

  cat > "$HOME/stub_cli.sh" << 'EOF'
#!/usr/bin/env bash
printf '%s\n' "$@" >> "$STUB_CLI_LOG"
printf '{"type":"result","is_error":false,"result":"STATUS: DONE"}\n'
EOF
  chmod +x "$HOME/stub_cli.sh"
  export CLAUDE_BIN="$HOME/stub_cli.sh"
  export CODEX_BIN="$HOME/stub_cli.sh"
  export AGY_BIN="$HOME/stub_cli.sh"
  export OPENCODE_BIN="$HOME/stub_cli.sh"
  export AGENTBUS_BIN="$HOME/.local/bin/agentbus"
  CLI_SESSION=()
  export CLI_SESSION
  export SESSION_ID="sid-test"

  watch_run() { return 0; }
}

# create_isolation_task id kind inv_team type
create_isolation_task() {
  local id=$1 kind=$2 inv_team=${3:-} type=${4:-claude}
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

@test "claude mcp config excludes agentbus for investigation team task but includes it for legacy investigator" {
  setup_isolation
  export STUB_CLI_LOG="$HOME/claude_cli.log"

  # 1. Investigation team task: must not have active agentbus in mcp.json and should enforce strict config
  create_isolation_task "inv-child-claude" "investigation" "team-parent-1" "claude"
  meta_load "inv-child-claude"
  local child_dir="$AGENTS_HOME/tasks/inv-child-claude"
  : > "$STUB_CLI_LOG"
  run_claude "inv-child-claude" new 1 "$child_dir/prompt-1.md" "$child_dir"
  [ -f "$child_dir/mcp.json" ]
  run jq -e '.mcpServers.agentbus.command // empty' "$child_dir/mcp.json"
  [ "$status" -ne 0 ]

  # 2. Legacy investigator task: must include agentbus in mcp.json
  create_isolation_task "inv-legacy-claude" "investigation" "" "claude"
  meta_load "inv-legacy-claude"
  local legacy_dir="$AGENTS_HOME/tasks/inv-legacy-claude"
  run_claude "inv-legacy-claude" new 1 "$legacy_dir/prompt-1.md" "$legacy_dir"
  [ -f "$legacy_dir/mcp.json" ]
  run jq -e '.mcpServers.agentbus.command' "$legacy_dir/mcp.json"
  [ "$status" -eq 0 ]
}

@test "codex invocation excludes agentbus args for investigation team task but includes them for legacy investigator" {
  setup_isolation
  export STUB_CLI_LOG="$HOME/codex_cli.log"

  # 1. Investigation team task: codex provides server command with enabled=false and no active args
  create_isolation_task "inv-child-codex" "investigation" "team-parent-1" "codex"
  meta_load "inv-child-codex"
  local child_dir="$AGENTS_HOME/tasks/inv-child-codex"
  : > "$STUB_CLI_LOG"
  run_codex "inv-child-codex" new 1 "$child_dir/prompt-1.md" "$child_dir"
  grep -q "mcp_servers\.agentbus\.command" "$STUB_CLI_LOG"
  grep -q "mcp_servers\.agentbus\.enabled=false" "$STUB_CLI_LOG"
  run grep -q "mcp_servers\.agentbus\.args" "$STUB_CLI_LOG"
  [ "$status" -ne 0 ]
  run grep -q "mcp_servers\.agentbus\.tool_timeout_sec" "$STUB_CLI_LOG"
  [ "$status" -ne 0 ]

  # 2. Legacy investigator task: codex args must contain active mcp_servers.agentbus command and args
  create_isolation_task "inv-legacy-codex" "investigation" "" "codex"
  meta_load "inv-legacy-codex"
  local legacy_dir="$AGENTS_HOME/tasks/inv-legacy-codex"
  : > "$STUB_CLI_LOG"
  run_codex "inv-legacy-codex" new 1 "$legacy_dir/prompt-1.md" "$legacy_dir"
  grep -q "mcp_servers\.agentbus\.command" "$STUB_CLI_LOG"
  grep -q "mcp_servers\.agentbus\.args" "$STUB_CLI_LOG"
  run grep -q "mcp_servers\.agentbus\.enabled=false" "$STUB_CLI_LOG"
  [ "$status" -ne 0 ]
}

@test "gemini mcp config excludes agentbus for investigation team task but includes it for legacy investigator" {
  setup_isolation
  export STUB_CLI_LOG="$HOME/gemini_cli.log"

  # 1. Investigation team task: workspace config must not contain active agentbus
  create_isolation_task "inv-child-gemini" "investigation" "team-parent-1" "gemini"
  meta_load "inv-child-gemini"
  local child_dir="$AGENTS_HOME/tasks/inv-child-gemini"
  rm -f "$HOME/repo/.agents/mcp_config.json"
  run_gemini "inv-child-gemini" new 1 "$child_dir/prompt-1.md" "$child_dir"
  if [[ -f "$HOME/repo/.agents/mcp_config.json" ]]; then
    run jq -e '.mcpServers.agentbus.command // empty' "$HOME/repo/.agents/mcp_config.json"
    [ "$status" -ne 0 ]
  fi

  # 2. Legacy investigator task: workspace config must contain agentbus command
  create_isolation_task "inv-legacy-gemini" "investigation" "" "gemini"
  meta_load "inv-legacy-gemini"
  local legacy_dir="$AGENTS_HOME/tasks/inv-legacy-gemini"
  rm -f "$HOME/repo/.agents/mcp_config.json"
  run_gemini "inv-legacy-gemini" new 1 "$legacy_dir/prompt-1.md" "$legacy_dir"
  [ -f "$HOME/repo/.agents/mcp_config.json" ]
  run jq -e '.mcpServers.agentbus.command' "$HOME/repo/.agents/mcp_config.json"
  [ "$status" -eq 0 ]
}

@test "opencode config excludes agentbus for investigation team task but includes it for legacy investigator" {
  setup_isolation

  # 1. Investigation team task: opencode_config must not configure active agentbus command
  create_isolation_task "inv-child-opencode" "investigation" "team-parent-1" "opencode"
  meta_load "inv-child-opencode"
  run opencode_config "inv-child-opencode"
  [ "$status" -eq 0 ]
  run jq -e 'if .mcp.agentbus then .mcp.agentbus.enabled == false else true end' <<< "$output"
  [ "$status" -eq 0 ]
  run jq -e '.mcp.agentbus.command // empty' <<< "$output"
  [ "$status" -ne 0 ]

  # 2. Legacy investigator task: opencode_config must configure active agentbus
  create_isolation_task "inv-legacy-opencode" "investigation" "" "opencode"
  meta_load "inv-legacy-opencode"
  run opencode_config "inv-legacy-opencode"
  [ "$status" -eq 0 ]
  run jq -e '.mcp.agentbus.command' <<< "$output"
  [ "$status" -eq 0 ]
}
