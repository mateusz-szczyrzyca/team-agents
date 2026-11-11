#!/usr/bin/env bats
# A worktree got the repository's .serena/project.yml only when it had none: a language added to the repository's
# file (rust, for new code in that language) never reached the worktrees already made, and their investigators,
# finding no language server for the files they worked on, reported Serena unavailable. Now each run copies the
# repository's file over a worktree's that differs; a repository without one leaves the worktree's as it is.

load helpers

setup_runs() {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  cat > "$HOME/stub_cli.sh" << 'EOF'
#!/usr/bin/env bash
printf '{"type":"result","is_error":false,"result":"STATUS: DONE"}\n'
EOF
  chmod +x "$HOME/stub_cli.sh"
  export CLAUDE_BIN="$HOME/stub_cli.sh" AGENTBUS_BIN="$HOME/.local/bin/agentbus" SESSION_ID=sid-test AGENT_SERENA=serena
  CLI_SESSION=()
  export CLI_SESSION
  watch_run() { return 0; }
  git -C "$HOME/repo" worktree add -q -b wt-branch "$HOME/wt" main
  local dir="$AGENTS_HOME/tasks/T-wt"
  mkdir -p "$dir"
  printf '%s\n' "ID=T-wt" "AGENT=lead" "KIND=" "TYPE=claude" "INVESTIGATION_TEAM=" "REPO_NAME=repo" "REPO_ROOT=$HOME/repo" \
    "WORKTREE=$HOME/wt" "BRANCH=wt-branch" "BASE=main" "SESSION_ID=sid-test" "RUN=1" "STATUS=running" > "$dir/meta.env"
  touch "$dir/prompt-1.md"
  meta_load T-wt
}

a_run() { run_claude T-wt new 1 "$AGENTS_HOME/tasks/T-wt/prompt-1.md" "$AGENTS_HOME/tasks/T-wt"; }

@test "a run copies the repository's Serena project file over a worktree's that differs" {
  setup_runs
  mkdir -p "$HOME/repo/.serena" "$HOME/wt/.serena"
  printf 'project_name: "repo"\nlanguage_servers:\n- go\n- rust\n' > "$HOME/repo/.serena/project.yml"
  printf 'project_name: "repo"\nlanguage_servers:\n- go\n' > "$HOME/wt/.serena/project.yml"
  a_run
  cmp -s "$HOME/repo/.serena/project.yml" "$HOME/wt/.serena/project.yml"
  grep -q '"event":"serena_project_synced"' "$AGENTS_HOME/logs/agent.jsonl"
}

@test "a repository without a Serena project file leaves the worktree's as it is" {
  setup_runs
  mkdir -p "$HOME/wt/.serena"
  printf 'project_name: "wt"\n' > "$HOME/wt/.serena/project.yml"
  a_run
  [ "$(cat "$HOME/wt/.serena/project.yml")" = 'project_name: "wt"' ]
}
