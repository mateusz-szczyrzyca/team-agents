#!/usr/bin/env bats
# A headless run ends with its turn. inv-opus ran make gate in the background and ended its turn to wait for it:
# Claude Code ended the run, killed the gate, and the run had no STATUS — the investigation's only phase-one run
# failed. Every run's prompt now says to run commands in the foreground and wait for them, and Claude Code's runs
# have its background tasks off, with a Bash timeout long enough for the gate.

load helpers

@test "a Claude Code run has background tasks off and a Bash timeout of CLAUDE_BASH_TIMEOUT_MIN" {
  scenario '{"solo": {"new": "done"}}'
  CLAUDE_BASH_TIMEOUT_MIN=25 agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  s=$AGENTS_HOME/tasks/$t/claude-settings.json
  [ "$(jq -r '.env.CLAUDE_CODE_DISABLE_BACKGROUND_TASKS' "$s")" = 1 ]
  [ "$(jq -r '.env.BASH_DEFAULT_TIMEOUT_MS' "$s")" = 1500000 ]
  [ "$(jq -r '.env.BASH_MAX_TIMEOUT_MS' "$s")" = 1500000 ]
}

@test "every run's prompt says to run commands in the foreground, never ending a turn to wait for one" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  grep -q "Run commands in the foreground" "$AGENTS_HOME/tasks/$t/prompt-1.md"
  grep -q "never end your turn to wait for" "$AGENTS_HOME/tasks/$t/prompt-1.md"
}
