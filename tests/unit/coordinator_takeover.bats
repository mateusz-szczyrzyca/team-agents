#!/usr/bin/env bats
# A coordinator failed and its team stood still: its members were told to take the role over (claim_lead), and the
# bus refused them — it gave the role up for a hung coordinator only. A coordinator out of its team is now replaced
# by any normal member that claims the role; and the script itself makes one of them the coordinator — the first of
# AGENT_COORDINATOR_PREFER among them, else the first of them — and wakes it.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
lead_of() { "$BUILD_DIR/agentbus" team-get --dir "$AGENTS_HOME/bus" --team "$1" | jq -r '.lead // ""'; }

# failed_coordinator — a team of lead, member and solo whose coordinator lead fails; its team in $team
failed_coordinator() {
  scenario '{"lead": {"new": "team-lead-crash"}, "member": {"new": "team-sleep", "resume": "team-sleep"}, "solo": {"new": "team-sleep", "resume": "team-sleep"}}'
  agent run --team lead,member,solo "team task" > /dev/null
  t=$(task_of lead)
  team=$(meta "$t" TEAM)
  wait_status "$t" failed
}

@test "a failed coordinator's role goes to the first of AGENT_COORDINATOR_PREFER among its members, woken at once" {
  printf 'AGENT_COORDINATOR_PREFER="lead solo member"\n' >> "$AGENTS_HOME/agents.conf"
  failed_coordinator
  for _ in $(seq 1 100); do [ "$(lead_of "$team")" = solo ] && break; sleep 0.2; done
  [ "$(lead_of "$team")" = solo ]
  wait_grep '"event":"coordinator_takeover"' "$AGENTS_HOME/logs/agent.jsonl"
  wait_grep 'solo coordinates the team now' "$AGENTS_HOME/bus/messages.jsonl"   # the members hear who coordinates
  s=$(task_of solo)
  for _ in $(seq 1 150); do [ "$(meta "$s" RUN)" -ge 2 ] && break; sleep 0.2; done
  [ "$(meta "$s" RUN)" -ge 2 ]   # woken to coordinate
  agent stop --running > /dev/null
}

@test "with no preference a failed coordinator's role goes to the first member at work" {
  failed_coordinator
  for _ in $(seq 1 100); do [ -n "$(lead_of "$team")" ] && [ "$(lead_of "$team")" != lead ] && break; sleep 0.2; done
  [[ "$(lead_of "$team")" == member || "$(lead_of "$team")" == solo ]]
  wait_grep '"event":"coordinator_takeover"' "$AGENTS_HOME/logs/agent.jsonl"
  agent stop --running > /dev/null
}

@test "coordinator_takeover picks a preferred member that is waking: the news of the failure woke it" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # read by preferred_coordinator
  AGENT_COORDINATOR_PREFER="lead solo member"
  team_json() { echo '{"lead":"lead","members":{"lead":{"state":"done","tier":"normal"},"solo":{"state":"waking","tier":"normal"},"member":{"state":"sleeping","tier":"normal"}}}'; }
  bus() { [[ $1 == claim-lead ]] && echo "You are the coordinator"; return 0; }
  team_lead() { echo solo; }
  run coordinator_takeover T lead "its run failed"
  [ "$status" -eq 0 ]
  [ "$output" = solo ]
}
