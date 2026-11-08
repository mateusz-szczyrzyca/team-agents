#!/usr/bin/env bats
# claude-haiku left its team: its Claude Code session, resumed run after run, reached some 130,000-176,000 tokens of
# context a step of its 200,000 window; it read the warnings as a hard limit, claimed an item and ended STATUS: DONE,
# which takes a member out of the team for good. The fresh-session rule did not see it: its first run had been heavy,
# and twice that was past the window. Now a resumed run of a CLI with a known window (AGENT_FRESH_SESSION_MAX, by
# default claude=120000) past that many tokens a step is followed by a fresh session; and a member's DONE while it
# holds a claimed board item resumes it once, in a fresh session, about those items — a second DONE finishes it.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# run_lines task n mode steps in — the activity of one run: its start, its tool calls, its usage
run_lines() {
  local a="$AGENTS_HOME/tasks/$1/activity.jsonl" i
  jq -nc --arg t "▶ run $2 ($3)" '{ts: "2026-11-01 10:00:00", k: "info", t: $t}' >> "$a"
  for ((i = 0; i < $4; i++)); do jq -nc '{ts: "2026-11-01 10:00:01", k: "cmd", t: "ls"}' >> "$a"; done
  jq -nc --arg t "usage: in $5 (cached 0), out 10" '{ts: "2026-11-01 10:00:02", k: "info", t: $t}' >> "$a"
}

# fake_task id type — a task's metadata, its CLI of that type
fake_task() { mkdir -p "$AGENTS_HOME/tasks/$1"; printf 'ID=%s\nAGENT=a\nTYPE=%s\nKIND=worker\n' "$1" "$2" > "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "a resumed claude run past AGENT_FRESH_SESSION_MAX a step is followed by a fresh session, under twice its start" {
  fake_task F claude
  run_lines F 1 new 3 300000      # 100,000 a step: a heavy start
  run_lines F 2 resume 3 390000   # 130,000: under twice the start, past claude's 120,000
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  fresh_session_due F
  [[ $FRESH_WHY == *"130000 tokens of context a step"* ]]
}

@test "a CLI without AGENT_FRESH_SESSION_MAX keeps the rule of twice its session starts" {
  fake_task G gemini
  run_lines G 1 new 3 300000
  run_lines G 2 resume 3 390000
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  ! fresh_session_due G || false
}

@test "a fresh run past AGENT_FRESH_SESSION_MAX is resumed next: no fresh session after fresh session" {
  fake_task H claude
  run_lines H 1 new 3 300000
  run_lines H 2 fresh 3 390000    # itself a fresh session, 130,000 a step
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  ! fresh_session_due H || false
}

# member_claims_then_done — a team whose member works 3 s and ends STATUS: DONE; the test claims B1 for the member
# meanwhile; sets t, m, team
member_claims_then_done() {
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "sleep:3"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member); team=$(meta "$t" TEAM)
  wait_status "$m" running
  "$BUILD_DIR/agentbus" board-add --dir "$AGENTS_HOME/bus" --team "$team" --as lead --title "for the member" --details "d" --level normal > /dev/null
  "$BUILD_DIR/agentbus" board-claim --dir "$AGENTS_HOME/bus" --team "$team" --as member --id B1 > /dev/null
}

@test "a member's DONE while it holds a claimed item resumes it once, in a fresh session, about the item" {
  member_claims_then_done
  # waits of up to 60 s: under the gate's parallel load the whole sequence takes some 20 s
  for _ in $(seq 1 300); do [ "$(meta "$m" RUN)" = 2 ] && break; sleep 0.2; done
  [ "$(meta "$m" RUN)" = 2 ]
  wait_grep '"event":"member_done_bounced"' "$AGENTS_HOME/logs/agent.jsonl" 60
  # the prompt is written just after RUN=2, piece by piece: wait for its whole line
  wait_grep "STATUS: DONE while you hold board items: B1" "$AGENTS_HOME/tasks/$m/prompt-2.md" 60
  grep -q "fresh session" "$AGENTS_HOME/tasks/$m/activity.jsonl"
  ! grep -q '"event":"member_items_back"' "$AGENTS_HOME/logs/agent.jsonl" || false
  agent stop --running > /dev/null
}

@test "a member's second DONE in a row finishes it as before: its items go back to the board" {
  member_claims_then_done
  wait_grep '"event":"member_items_back"' "$AGENTS_HOME/logs/agent.jsonl" 60   # some 20 s under the gate's load
  wait_status "$m" 'done'
  [ "$(meta "$m" RUN)" = 2 ]
  [ "$(grep -c '"event":"member_done_bounced"' "$AGENTS_HOME/logs/agent.jsonl")" -eq 1 ]
  agent stop --running > /dev/null
}

@test "a member's DONE without a claimed item finishes it at once" {
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "sleep:1"}}'
  agent run --team lead,member "team task" > /dev/null
  m=$(task_of member)
  wait_status "$m" 'done'
  [ "$(meta "$m" RUN)" = 1 ]
  ! grep -q '"event":"member_done_bounced"' "$AGENTS_HOME/logs/agent.jsonl" || false
  agent stop --running > /dev/null
}

@test "a member's DONE in a finished team finishes it at once, its claimed item or not" {
  member_claims_then_done
  "$BUILD_DIR/agentbus" team-done --dir "$AGENTS_HOME/bus" --team "$team" > /dev/null
  wait_status "$m" 'done'
  [ "$(meta "$m" RUN)" = 1 ]
  ! grep -q '"event":"member_done_bounced"' "$AGENTS_HOME/logs/agent.jsonl" || false
  agent stop --running > /dev/null
}

@test "a remote team member's DONE finishes it as before: its board lives on the lead, not on this bus" {
  # the runner died (exit 3) when it read the board of a team this bus does not have. The fields are written between
  # the runs: a run reads them when it starts
  scenario '{"solo": {"new": "questions-file", "resume": "done"}}'
  agent run solo "a task" > /dev/null
  t=$(task_of solo)
  wait_status "$t" waiting
  printf 'TEAM=T\nMEMBER=solo@node2\nREMOTE_TEAM=1\nJOB_LEAD=http://127.0.0.1:9\n' >> "$AGENTS_HOME/tasks/$t/meta.env"
  sed -i 's/^\*\*Answer:\*\*$/**Answer:** ok/' "$AGENTS_HOME/tasks/$t/questions-1.md"
  printf '\n** ANSWERED **\n' >> "$AGENTS_HOME/tasks/$t/questions-1.md"
  agent _answers_saved "$t" 1
  wait_status "$t" 'done'
  [ "$(meta "$t" RUN)" = 2 ]
  ! grep -q '"event":"member_done_bounced"' "$AGENTS_HOME/logs/agent.jsonl" || false
}
