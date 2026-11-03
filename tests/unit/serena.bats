#!/usr/bin/env bats
# Serena (an MCP server that navigates code through language servers) for every CLI when AGENT_SERENA names its
# command: claude, codex, gemini (Antigravity) and opencode get a `serena` server on the run's own worktree, with the
# context Serena has for that client; the prompt says to use it. Off (the default): nothing changes.

load helpers

# setup_runs — the script sourced, the CLIs a stub that records its arguments, a run's task of that type
setup_runs() {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  cat > "$HOME/stub_cli.sh" << 'EOF'
#!/usr/bin/env bash
printf '%s\n' "$@" >> "$STUB_CLI_LOG"
printf '{"type":"result","is_error":false,"result":"STATUS: DONE"}\n'
EOF
  chmod +x "$HOME/stub_cli.sh"
  export CLAUDE_BIN="$HOME/stub_cli.sh" CODEX_BIN="$HOME/stub_cli.sh" AGY_BIN="$HOME/stub_cli.sh" OPENCODE_BIN="$HOME/stub_cli.sh"
  export AGENTBUS_BIN="$HOME/.local/bin/agentbus" STUB_CLI_LOG="$HOME/cli.log" SESSION_ID=sid-test
  CLI_SESSION=()
  export CLI_SESSION
  watch_run() { return 0; }
  : > "$STUB_CLI_LOG"
}

# a_task id type [investigation-team] — a task of that CLI working in $HOME/repo, loaded
a_task() {
  local dir="$AGENTS_HOME/tasks/$1"
  mkdir -p "$dir"
  printf '%s\n' "ID=$1" "KIND=${3:+investigation}" "TYPE=$2" "INVESTIGATION_TEAM=${3:-}" "REPO_NAME=repo" \
    "REPO_ROOT=$HOME/repo" "WORKTREE=$HOME/repo" "BRANCH=b" "BASE=main" "SESSION_ID=sid-test" "RUN=1" "STATUS=running" \
    > "$dir/meta.env"
  touch "$dir/prompt-1.md"
  meta_load "$1"
}

@test "Serena off by default: no serena server for any CLI" {
  setup_runs
  [ -z "$AGENT_SERENA" ]
  a_task T-claude claude
  run_claude T-claude new 1 "$AGENTS_HOME/tasks/T-claude/prompt-1.md" "$AGENTS_HOME/tasks/T-claude"
  [ "$(jq '.mcpServers | has("serena")' "$AGENTS_HOME/tasks/T-claude/mcp.json")" = false ]
  [ "$(grep -c serena "$STUB_CLI_LOG")" -eq 0 ]
  [ "$(opencode_config T-claude | jq '.mcp | has("serena")')" = false ]
}

@test "claude: a serena server on the run's worktree, context claude-code, next to agentbus" {
  setup_runs
  export AGENT_SERENA=/opt/serena/bin/serena
  a_task T-claude claude
  run_claude T-claude new 1 "$AGENTS_HOME/tasks/T-claude/prompt-1.md" "$AGENTS_HOME/tasks/T-claude"
  local m="$AGENTS_HOME/tasks/T-claude/mcp.json"
  [ "$(jq -r .mcpServers.serena.command "$m")" = /opt/serena/bin/serena ]
  [ "$(jq -c .mcpServers.serena.args "$m")" = \
    "$(jq -nc --arg w "$HOME/repo" '["start-mcp-server","--context","claude-code","--project",$w,"--open-web-dashboard","false"]')" ]
  jq -e .mcpServers.agentbus.command "$m"
}

@test "claude: an independent investigator gets serena, still without agentbus" {
  setup_runs
  export AGENT_SERENA=serena
  a_task T-inv claude team-1
  run_claude T-inv new 1 "$AGENTS_HOME/tasks/T-inv/prompt-1.md" "$AGENTS_HOME/tasks/T-inv"
  local m="$AGENTS_HOME/tasks/T-inv/mcp.json"
  [ "$(jq -r '.mcpServers | keys | join(",")' "$m")" = serena ]
  grep -qx -- --strict-mcp-config "$STUB_CLI_LOG"
}

@test "codex: serena with context codex and a startup time for its language servers" {
  setup_runs
  export AGENT_SERENA=serena
  a_task T-codex codex
  run_codex T-codex new 1 "$AGENTS_HOME/tasks/T-codex/prompt-1.md" "$AGENTS_HOME/tasks/T-codex"
  grep -qx -- 'mcp_servers.serena.command="serena"' "$STUB_CLI_LOG"
  grep -qxF -- "mcp_servers.serena.args=[\"start-mcp-server\",\"--context\",\"codex\",\"--project\",\"$HOME/repo\",\"--open-web-dashboard\",\"false\"]" "$STUB_CLI_LOG"
  grep -qx -- 'mcp_servers.serena.startup_timeout_sec=120' "$STUB_CLI_LOG"
  grep -q -- 'mcp_servers.agentbus.command=' "$STUB_CLI_LOG"
}

@test "gemini (Antigravity): serena in the workspace's MCP file, context antigravity" {
  setup_runs
  export AGENT_SERENA=serena
  a_task T-agy gemini
  run_gemini T-agy new 1 "$AGENTS_HOME/tasks/T-agy/prompt-1.md" "$AGENTS_HOME/tasks/T-agy"
  local m="$HOME/repo/.agents/mcp_config.json"
  [ "$(jq -r .mcpServers.serena.command "$m")" = serena ]
  [ "$(jq -r '.mcpServers.serena.args[2]' "$m")" = antigravity ]
  jq -e .mcpServers.agentbus.command "$m"
}

@test "opencode: serena as a local server, context ide, on" {
  setup_runs
  export AGENT_SERENA=serena
  a_task T-oc opencode
  cfg=$(opencode_config T-oc)
  [ "$(jq -r .mcp.serena.type <<< "$cfg")" = local ]
  [ "$(jq -r .mcp.serena.enabled <<< "$cfg")" = true ]
  [ "$(jq -c .mcp.serena.command <<< "$cfg")" = \
    "$(jq -nc --arg w "$HOME/repo" '["serena","start-mcp-server","--context","ide","--project",$w,"--open-web-dashboard","false"]')" ]
}

@test "Serena's project folder in a worktree is never committed: .serena/ is excluded" {
  setup_runs
  export AGENT_SERENA=serena
  a_task T-claude claude
  run_claude T-claude new 1 "$AGENTS_HOME/tasks/T-claude/prompt-1.md" "$AGENTS_HOME/tasks/T-claude"
  grep -qx '.serena/' "$HOME/repo/.git/info/exclude"
  mkdir -p "$HOME/repo/.serena" && touch "$HOME/repo/.serena/project.yml"
  [ -z "$(git -C "$HOME/repo" status --porcelain)" ]
  run_claude T-claude new 1 "$AGENTS_HOME/tasks/T-claude/prompt-1.md" "$AGENTS_HOME/tasks/T-claude"
  [ "$(grep -cx '.serena/' "$HOME/repo/.git/info/exclude")" -eq 1 ]
}

@test "the prompt tells the agent to navigate with Serena only when it is on" {
  cd "$HOME/repo"
  agent run solo "Fix the cache." > /dev/null
  t=$(task_of solo)
  wait_status "$t" "done"
  [ "$(grep -ci serena "$AGENTS_HOME/tasks/$t/prompt-1.md")" -eq 0 ]
  echo 'AGENT_SERENA=serena' >> "$AGENTS_HOME/agents.conf"
  agent run solo "Fix the cache again." > /dev/null
  t=$(task_of solo)
  wait_status "$t" "done"
  grep -q 'MCP server `serena`' "$AGENTS_HOME/tasks/$t/prompt-1.md"
  [ "$(grep -c '{{' "$AGENTS_HOME/tasks/$t/prompt-1.md")" -eq 0 ]
}
