#!/usr/bin/env bats
# A round's coordinator ended it while a member's run still went on; --autopilot's fix pass then could not go back to
# the team ("task … is still active") and the human was called — the member ended 8 minutes later. A hand-back
# refused for a run that still goes on now waits: _supervise hands the work back once the run ends, and after
# AGENT_HANDBACK_WAIT_MIN it stops that one run. Any other refusal reaches the human at once, with the hand-back's text.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
set_meta() { (source "$REPO_ROOT/bin/agent"; meta_set "$1" "$2" "$3"); }

# round_ended_member_running — a team whose coordinator ended its run (the team closed) while its member still runs
round_ended_member_running() {
  scenario '{"lead": {"new": "done"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member)
  wait_status "$m" running
  wait_status "$t" 'done'
  team=$(meta "$t" TEAM)
}

# fix_pass — the fix pass's hand-back, as --autopilot starts it
fix_pass() {
  (source "$REPO_ROOT/bin/agent"; handback "$t" "the fix pass" "AGENT_FIX_PASS=1" run --resume "$team" --same-round "Fix the findings.")
}

@test "a hand-back refused while a member still runs waits; _supervise hands the work back once the run ends" {
  round_ended_member_running
  fix_pass
  [ "$(meta "$t" HANDBACK_PENDING)" = 1 ]
  [ -z "$(grep 'could not go back' "$HOME/notifications.log" 2> /dev/null)" ]   # nobody is called
  agent _supervise                       # the member still runs: the hand-back still waits
  [ "$(meta "$t" HANDBACK_PENDING)" = 1 ]
  [ "$(task_of lead)" = "$t" ]
  agent stop "$m" > /dev/null            # its run ends
  agent _supervise
  [ "$(meta "$t" HANDBACK_PENDING)" != 1 ]
  [ "$(task_of lead)" != "$t" ]          # the team took its work back: its coordinator's new task
  grep -q 'Fix the findings.' "$AGENTS_HOME/tasks/$(task_of lead)/task.md"
  grep -q '"event":"handback_done"' "$AGENTS_HOME/logs/agent.jsonl"
  agent stop --running > /dev/null
}

@test "a run that goes on past AGENT_HANDBACK_WAIT_MIN is stopped — that one alone — and the work goes back to the team" {
  round_ended_member_running
  fix_pass
  set_meta "$t" HANDBACK_SINCE "$(( $(date +%s) - 31 * 60 ))"
  AGENT_HANDBACK_WAIT_MIN=30 agent _supervise
  [ "$(meta "$m" STATUS)" = stopped ]
  [ "$(meta "$t" HANDBACK_PENDING)" != 1 ]
  [ "$(task_of lead)" != "$t" ]
  grep -q '"event":"handback_stopped_run"' "$AGENTS_HOME/logs/agent.jsonl"
  agent stop --running > /dev/null
}

@test "a hand-back refused for another reason reaches the human at once, with where its text is" {
  round_ended_member_running
  agent stop "$m" > /dev/null
  run bash -c "source '$REPO_ROOT/bin/agent'; handback '$t' 'the fix pass' 'AGENT_FIX_PASS=1' run --resume no-such-team --same-round 'Fix the findings.' || { handover_failed '$t' 'the fix pass' \"\$HANDBACK_OUT\"; exit 1; }"
  [ "$status" -ne 0 ]
  [ "$(meta "$t" HANDBACK_PENDING)" != 1 ]
  wait_grep "could not go back" "$HOME/notifications.log"
  grep -q "handback.md" "$HOME/notifications.log"
  grep -q 'Fix the findings.' "$AGENTS_HOME/tasks/$t/handback.md"
}

@test "resuming a team with a task still running names that task to stop, not every task" {
  round_ended_member_running
  run agent run --resume "$team" --same-round "go on"
  [ "$status" -ne 0 ]
  [[ $output == *"is still active"* && $output == *"agent stop $m"* ]]
  [[ $output != *"agent stop --running"* ]]
  agent stop --running > /dev/null
}
