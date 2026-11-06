#!/usr/bin/env bats
# A Claude Code task's fresh session was started with the session ID of its old session: Claude Code refused it
# ("Error: Session ID … is already in use."), the run failed and the member was out of its team — every fresh
# session of a Claude agent did. A fresh session now gets a new session ID.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

answer() {
  local q="$AGENTS_HOME/tasks/$1/questions-1.md"
  sed -i 's/^\*\*Answer:\*\*$/**Answer:** ok/' "$q"
  printf '\n** ANSWERED **\n' >> "$q"
  agent _answers_saved "$1" 1
}

used() {
  local a="$AGENTS_HOME/tasks/$1/activity.jsonl" i
  for ((i = 0; i < $3; i++)); do jq -nc '{ts: "2026-10-31 10:00:01", k: "cmd", t: "ls"}' >> "$a"; done
  jq -nc --arg t "usage: in $2 (cached 0), out 10" '{ts: "2026-10-31 10:00:02", k: "info", t: $t}' >> "$a"
}

@test "a Claude Code task's fresh session gets a new session ID, not its old session's" {
  scenario '{"solo": {"new": "questions-file", "resume": "done"}}'
  agent run solo "a task about the fresh session's ID"
  t=$(task_of solo)
  wait_status "$t" waiting
  old=$(meta "$t" SESSION_ID)
  [ -n "$old" ]
  used "$t" 900000 3   # 300,000 a step: the next run starts a fresh session
  answer "$t"
  wait_grep '"event":"fresh_session"' "$AGENTS_HOME/logs/agent.jsonl"
  new=$(grep '"event":"fresh_session"' "$AGENTS_HOME/logs/agent.jsonl" | tail -n 1 | jq -r '.session // empty')
  [[ $new =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]]
  [ "$new" != "$old" ]
}
