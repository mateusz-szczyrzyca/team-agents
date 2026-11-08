#!/usr/bin/env bats
# The human applies a patch and runs the tests while a team works: its agents' own test runs raced them. agent pause
# <team> stops the team's runs at once — its members', its coordinator's, its workers' and investigators' — without
# failing anyone: their tasks are held, still in the team. While the team is paused nothing of it starts (a wake, a
# message, an answer wait), and the supervisor leaves it alone. agent unpause <team> resumes the interrupted runs where
# they stopped — the same session, told it was paused — and starts what waited.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# paused_team — a team: a coordinator asleep and a member at work (it works until stopped; resumed, it ends DONE);
# sets t (the coordinator's task), m (the member's), team
paused_team() {
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-wait", "resume": "team-lead-wait"}, "member": {"new": "hang", "resume": "done"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member); team=$(meta "$t" TEAM)
  wait_status "$t" sleeping
  wait_status "$m" running
}

member_state() { "$BUILD_DIR/agentbus" team-get --dir "$AGENTS_HOME/bus" --team "$team" | jq -r ".members[\"$1\"].state"; }

@test "agent pause stops the team's runs without failing anyone: held, still members of the team" {
  paused_team
  run agent pause "$team"
  [ "$status" -eq 0 ]
  [ "$(meta "$m" STATUS)" = held ]
  [ "$(member_state member)" != stopped ]
  [ "$(member_state member)" != "done" ]
  [ "$(meta "$t" STATUS)" = sleeping ]
  ! grep -q '"event":"member_out"' "$AGENTS_HOME/logs/agent.jsonl" || false
  grep -q '"event":"team_paused"' "$AGENTS_HOME/logs/agent.jsonl"
  agent stop --running > /dev/null
}

@test "a paused team starts nothing: a wake waits for agent unpause" {
  paused_team
  agent pause "$team" > /dev/null
  run=$(meta "$t" RUN)
  agent _wake "$t"
  sleep 1
  [ "$(meta "$t" RUN)" = "$run" ]
  [ "$(meta "$t" HOLD_START)" = resume ]
  grep -q '"event":"start_held"' "$AGENTS_HOME/logs/agent.jsonl"
  agent stop --running > /dev/null
}

@test "agent unpause resumes the interrupted runs where they stopped, told what happened, and starts what waited" {
  paused_team
  agent pause "$team" > /dev/null
  agent _wake "$t"
  lrun=$(meta "$t" RUN)
  run agent unpause "$team"
  [ "$status" -eq 0 ]
  wait_status "$m" 'done'
  [ "$(meta "$m" RUN)" = 2 ]
  grep -q '▶ run 2 (resume)' "$AGENTS_HOME/tasks/$m/activity.jsonl"
  wait_grep "paused your team" "$AGENTS_HOME/tasks/$m/prompt-2.md"
  for _ in $(seq 1 100); do [ "$(meta "$t" RUN)" -gt "$lrun" ] && break; sleep 0.2; done
  [ "$(meta "$t" RUN)" -gt "$lrun" ]
  [ -z "$(meta "$t" HOLD_START)" ]
  grep -q '"event":"team_unpaused"' "$AGENTS_HOME/logs/agent.jsonl"
  agent stop --running > /dev/null
}

@test "the supervisor leaves a paused team alone: no stall alarm on its board" {
  export AGENT_BOARD_STALL_MIN=0 AGENT_EASY_TO_WORKER_MIN=999
  paused_team
  "$BUILD_DIR/agentbus" board-add --dir "$AGENTS_HOME/bus" --team "$team" --as lead --title "waits" --details "d" --level normal > /dev/null
  agent pause "$team" > /dev/null
  agent _supervise
  ! grep -q '"event":"board_stall"' "$AGENTS_HOME/logs/agent.jsonl" || false
  [ "$(meta "$m" STATUS)" = held ]
  agent stop --running > /dev/null
}

@test "pause and unpause: an unknown team is refused; a second pause, or unpausing a team not paused, says so" {
  run agent pause no-such-team
  [ "$status" -ne 0 ]
  paused_team
  agent pause "$team" > /dev/null
  run agent pause "$team"
  [ "$status" -eq 0 ]
  [[ $output == *"paused already"* ]]
  agent unpause "$team" > /dev/null
  run agent unpause "$team"
  [ "$status" -eq 0 ]
  [[ $output == *"not paused"* ]]
  agent stop --running > /dev/null
}
