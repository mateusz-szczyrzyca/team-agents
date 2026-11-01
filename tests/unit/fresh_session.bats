#!/usr/bin/env bats
# A run that read a large context a step is not resumed: the next run starts a fresh session — the full prompt, the
# last report, the files changed, then why it was woken; a small one is resumed as before. A team member's run that
# grows long (AGENT_LONG_RUN_STEPS) is asked to end, and its next run starts a fresh session at once.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# answer task — the task's first questions answered "ok" and marked ANSWERED
answer() {
  local q="$AGENTS_HOME/tasks/$1/questions-1.md"
  sed -i 's/^\*\*Answer:\*\*$/**Answer:** ok/' "$q"
  printf '\n** ANSWERED **\n' >> "$q"
  agent _answers_saved "$1" 1
}

# used task in steps — the task's last run made that many tool calls and read that many tokens of input
used() {
  local a="$AGENTS_HOME/tasks/$1/activity.jsonl" i
  for ((i = 0; i < $3; i++)); do jq -nc '{ts: "2026-10-31 10:00:01", k: "cmd", t: "ls"}' >> "$a"; done
  jq -nc --arg t "usage: in $2 (cached 0), out 10" '{ts: "2026-10-31 10:00:02", k: "info", t: $t}' >> "$a"
}

@test "a run that read a large context a step is followed by a fresh session, not a resume" {
  scenario '{"solo": {"new": "questions-file", "resume": "done"}}'
  agent run solo "a task about the fresh session"
  t=$(task_of solo)
  wait_status "$t" waiting
  used "$t" 900000 3   # 300,000 a step: beyond AGENT_FRESH_SESSION_TOKENS
  answer "$t"
  wait_grep "fresh session" "$AGENTS_HOME/tasks/$t/activity.jsonl"
  wait_status "$t" waiting   # its CLI started anew (no --resume): the fake asks again, as on a new run
  p=$AGENTS_HOME/tasks/$t/prompt-2.md
  grep -q "a task about the fresh session" "$p"
  grep -q "A fresh session" "$p"
  grep -q '"event":"fresh_session"' "$AGENTS_HOME/logs/agent.jsonl"
}

@test "a run with a small context a step is resumed as before" {
  scenario '{"solo": {"new": "questions-file", "resume": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" waiting
  used "$t" 3000 3
  answer "$t"
  wait_status "$t" 'done'
  ! grep -q "A fresh session" "$AGENTS_HOME/tasks/$t/prompt-2.md"
}

@test "a team member's long run ends it: its next run starts a fresh session, woken at once" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  pid=$(cat "$STUB_UNITS/agent-$t-r1.service.pid")
  for _ in $(seq 1 100); do   # its runner has ended: its own end of run would take the mark away
    kill -0 "$pid" 2> /dev/null || break   # "no such process" is the answer waited for
    sleep 0.1
  done
  touch "$AGENTS_HOME/tasks/$t/long-run"
  ( source "$REPO_ROOT/bin/agent"; long_run_end "$t" sleeping )
  [ "$(meta "$t" LONG_RUN)" = 1 ]
  [ ! -e "$AGENTS_HOME/tasks/$t/long-run" ]
  grep -rqs -- "_wake $t" "$STUB_UNITS"   # the wake is scheduled
  ( source "$REPO_ROOT/bin/agent"; fresh_session_due "$t" )
}
