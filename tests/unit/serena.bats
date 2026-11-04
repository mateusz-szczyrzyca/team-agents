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

@test "config/agents.conf enables AGENT_SERENA=serena and worktrees inherit .serena/project.yml from REPO_ROOT" {
  grep -qx 'AGENT_SERENA="serena"' "$REPO_ROOT/config/agents.conf"
  setup_runs
  export AGENT_SERENA=serena
  mkdir -p "$HOME/repo/.serena"
  printf 'project_name: "repo"\nlanguage_servers:\n- go\n- bash\n' > "$HOME/repo/.serena/project.yml"
  git -C "$HOME/repo" worktree add -q -b wt-branch "$HOME/wt" main
  a_task T-wt claude
  meta_set T-wt WORKTREE "$HOME/wt"
  meta_load T-wt
  run_claude T-wt new 1 "$AGENTS_HOME/tasks/T-wt/prompt-1.md" "$AGENTS_HOME/tasks/T-wt"
  [ -f "$HOME/wt/.serena/project.yml" ]
  grep -qx -- '- go' "$HOME/wt/.serena/project.yml"
  grep -qx -- '- bash' "$HOME/wt/.serena/project.yml"
}

@test "every model CLI (claude, codex, gemini, opencode) receives the Serena prompt and invokes the serena MCP server when needed" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  setup_runs
  watch_run() { wait "$1"; }
  cat > "$HOME/serena_mcp.sh" << 'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$HOME/serena-invocations.log"
EOF
  chmod +x "$HOME/serena_mcp.sh"
  export AGENT_SERENA="$HOME/serena_mcp.sh"

  cat > "$HOME/stub_cli.sh" << 'EOF'
#!/usr/bin/env bash
if [[ -n ${OPENCODE_CONFIG_CONTENT:-} ]]; then
  mapfile -t cmd < <(jq -r '.mcp.serena.command[]' <<< "$OPENCODE_CONFIG_CONTENT")
  "${cmd[@]}" <<< '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"serena_repl"}}'
  printf '{"type":"step_finish","part":{"reason":"stop","tokens":{"input":10,"output":5}}}\n'
  printf '{"type":"text","part":{"text":"STATUS: DONE"}}\n'
  exit 0
fi
for ((i=1; i<=$#; i++)); do
  arg="${!i}"
  if [[ $arg == --mcp-config ]]; then
    j=$((i+1)); cfg="${!j}"
    cmd=$(jq -r '.mcpServers.serena.command' "$cfg")
    mapfile -t sargs < <(jq -r '.mcpServers.serena.args[]' "$cfg")
    "$cmd" "${sargs[@]}" <<< '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"serena_repl"}}'
  elif [[ $arg == mcp_servers.serena.args=* ]]; then
    raw="${arg#mcp_servers.serena.args=}"
    mapfile -t sargs < <(jq -r '.[]' <<< "$raw")
    "$AGENT_SERENA" "${sargs[@]}" <<< '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"serena_repl"}}'
  fi
done
if [[ -f "$PWD/.agents/mcp_config.json" ]] && grep -q -- '--print-timeout' <<< "$*"; then
  cmd=$(jq -r '.mcpServers.serena.command' "$PWD/.agents/mcp_config.json")
  mapfile -t sargs < <(jq -r '.mcpServers.serena.args[]' "$PWD/.agents/mcp_config.json")
  "$cmd" "${sargs[@]}" <<< '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"serena_repl"}}'
  printf '{"event":"result","result":{"status":"SUCCESS","response":"STATUS: DONE","conversation_id":"cid-1"}}\n'
  exit 0
fi
printf '{"type":"thread.started","thread_id":"tid-1"}\n'
printf '{"type":"result","is_error":false,"result":"STATUS: DONE"}\n'
EOF
  chmod +x "$HOME/stub_cli.sh"

  for cli in claude codex gemini opencode; do
    a_task "T-$cli" "$cli"
    meta_set "T-$cli" AGENT "solo"
    meta_load "T-$cli"
    printf 'Task for %s\n' "$cli" > "$AGENTS_HOME/tasks/T-$cli/task.md"
    build_prompt "T-$cli" new "$AGENTS_HOME/tasks/T-$cli/questions-1.md" 1 > "$AGENTS_HOME/tasks/T-$cli/prompt-1.md"
    grep -q 'initial_instructions' "$AGENTS_HOME/tasks/T-$cli/prompt-1.md"
    grep -q 'serena_repl' "$AGENTS_HOME/tasks/T-$cli/prompt-1.md"
    "run_$cli" "T-$cli" new 1 "$AGENTS_HOME/tasks/T-$cli/prompt-1.md" "$AGENTS_HOME/tasks/T-$cli"
  done

  grep -qxF "start-mcp-server --context claude-code --project $HOME/repo --open-web-dashboard false" "$HOME/serena-invocations.log"
  grep -qxF "start-mcp-server --context codex --project $HOME/repo --open-web-dashboard false" "$HOME/serena-invocations.log"
  grep -qxF "start-mcp-server --context antigravity --project $HOME/repo --open-web-dashboard false" "$HOME/serena-invocations.log"
  grep -qxF "start-mcp-server --context ide --project $HOME/repo --open-web-dashboard false" "$HOME/serena-invocations.log"
}
