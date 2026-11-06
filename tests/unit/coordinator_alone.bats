#!/usr/bin/env bats
# A coordinator worked alone 200 steps, its board empty, while its members slept: the split was left to it, and it did
# not split. The runner now ends a coordinator's run once it has worked AGENT_SOLO_COORD_STEPS steps alone — its other
# members not at work, its board with nothing open for anyone else —, and its next run, a fresh session, starts with
# the work to split. And AGENT_COORDINATOR_PREFER names who coordinates a new team.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "coordinator_alone: the coordinator, its members not at work, nothing open on the board for anyone else" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/L" "$TASKS/M"
  printf 'ID=L\nTEAM=T\nAGENT=lead\nSTATUS=running\n' > "$TASKS/L/meta.env"
  printf 'ID=M\nTEAM=T\nAGENT=member\nSTATUS=sleeping\n' > "$TASKS/M/meta.env"
  team_lead() { echo lead; }
  bus() { echo '{"board": {}}'; }
  coordinator_alone L
  meta_set M STATUS running
  run coordinator_alone L
  [ "$status" -ne 0 ]
  meta_set M STATUS sleeping
  bus() { echo '{"board": {"B1": {"status": "todo", "for": "member"}}}'; }
  run coordinator_alone L
  [ "$status" -ne 0 ]
  bus() { echo '{"board": {"B1": {"status": "done", "for": "member"}, "B2": {"status": "todo", "for": "lead"}}}'; }
  coordinator_alone L
  team_lead() { echo member; }
  run coordinator_alone L
  [ "$status" -ne 0 ]
  team_lead() { echo lead; }
  rm -r "$TASKS/M"
  run coordinator_alone L
  [ "$status" -ne 0 ]
}

@test "a coordinator working alone past AGENT_SOLO_COORD_STEPS is ended; its next run starts with the work to split" {
  export AGENT_SOLO_COORD_STEPS=3 AGENT_HANG_POLL=1
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "done"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member)
  wait_status "$m" 'done'
  wait_status "$t" running
  team=$(meta "$t" TEAM)
  f=$(grep -l '"teams"' "$AGENTS_HOME"/bus/*.json | head -n 1)
  jq --arg t "$team" '.teams[$t].lead = "lead"' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
  for i in 1 2 3 4; do jq -nc --arg i "$i" '{ts: "2026-11-06 10:00:00", k: "cmd", t: ("ls " + $i)}' >> "$AGENTS_HOME/tasks/$t/activity.jsonl"; done
  wait_grep "worked alone" "$AGENTS_HOME/tasks/$t/activity.jsonl"
  grep -q '"event":"solo_coordinator_cut"' "$AGENTS_HOME/logs/agent.jsonl"
  wait_grep "put the work on the board" "$AGENTS_HOME/tasks/$t/prompt-2.md"   # the next run's prompt, once written
  agent stop --running > /dev/null
}

@test "AGENT_COORDINATOR_PREFER names who coordinates a new team" {
  printf 'AGENT_COORDINATOR_PREFER="solo member"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  team=$(meta "$(task_of lead)" TEAM)
  [ "$(source "$REPO_ROOT/bin/agent"; team_lead "$team")" = member ]
  agent stop --running > /dev/null
}
