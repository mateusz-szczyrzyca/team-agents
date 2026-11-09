#!/usr/bin/env bats
# A reviewer ran make gate in the background and ended its turn "waiting" for it, without a STATUS line; the human
# got a question about it, and answered "continue as planned", again and again. Under --autopilot a turn without
# STATUS now goes back to the agent with just that — continue as planned, run commands in the foreground, end with a
# STATUS line —; the human hears of it only when the agent does it again on the next turn.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

autopilot_team() {   # $1: the coordinator's actions
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario "{\"lead\": $1, \"member\": {\"new\": \"team-sleep\"}}"
  agent run --autopilot --team lead,member "team task" > /dev/null
  t=$(task_of lead)
}

@test "--autopilot: a turn without STATUS goes back to the agent, continue as planned — no question for the human" {
  autopilot_team '{"new": "no-status", "resume": "team-lead-wait"}'
  for _ in $(seq 1 150); do [ "$(meta "$t" RUN)" -ge 2 ] && break; sleep 0.2; done
  [ "$(meta "$t" RUN)" -ge 2 ]
  wait_status "$t" sleeping
  ! ls "$AGENTS_HOME/tasks/$t"/questions-*.md 2> /dev/null || false
  grep -q '"event":"nostatus_autopilot_bounced"' "$AGENTS_HOME/logs/agent.jsonl"
  grep -rqi "continue as planned" "$AGENTS_HOME/tasks/$t"/prompt-2*
  agent stop --running > /dev/null
}

@test "--autopilot: a second turn in a row without STATUS asks the human" {
  autopilot_team '{"new": "no-status", "resume": "no-status"}'
  wait_status "$t" waiting
  ls "$AGENTS_HOME/tasks/$t"/questions-*.md > /dev/null
  [ "$(grep -c '"event":"nostatus_autopilot_bounced"' "$AGENTS_HOME/logs/agent.jsonl")" -eq 1 ]
  agent stop --running > /dev/null
}

@test "nostatus_autopilot: an investigator's or reviewer's task follows the --autopilot of the task that asked for it" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/C" "$TASKS/R" "$TASKS/S" "$TASKS/O"
  printf 'ID=C\nAUTOPILOT=1\n' > "$TASKS/C/meta.env"
  printf 'ID=R\nKIND=investigation\nPARENT_TASK=C\n' > "$TASKS/R/meta.env"
  printf 'ID=S\nKIND=worker\n' > "$TASKS/S/meta.env"
  printf 'ID=O\nKIND=investigation\nPARENT_TASK=S\n' > "$TASKS/O/meta.env"
  nostatus_autopilot R          # its requester runs --autopilot
  ! nostatus_autopilot O || false   # its requester does not
  ! nostatus_autopilot S || false
  meta_set C AUTOPILOT_OFF 1
  ! nostatus_autopilot R || false   # the human turned it off
}
