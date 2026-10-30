#!/usr/bin/env bats
# agent wake <task>: a paused task resumes now, not at its pause's end (a provider's limit lifted early, a bigger
# plan bought); on the lead a remote member's proxy task asks the member's node, which resumes it at once.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# paused_task — a solo task paused on a network failure (15 minutes), its runner ended; prints its ID
paused_task() {
  scenario '{"solo": {"new": "network-down", "resume": "done"}}'
  agent run solo "a task" > /dev/null
  local t pid
  t=$(task_of solo)
  wait_status "$t" paused
  wait_grep "unreachable" "$HOME/notifications.log"
  pid=$(cat "$STUB_UNITS/agent-$t-r1.service.pid")
  for _ in $(seq 1 100); do   # its runner has ended: it writes the task's fields until then
    kill -0 "$pid" 2> /dev/null || break   # "no such process" is the answer waited for
    sleep 0.1
  done
  echo "$t"
}

@test "agent wake: a paused task resumes now, not at its pause's end" {
  t=$(paused_task)
  agent wake "$t"
  wait_status "$t" 'done'
  grep -q '"event":"wake_by_hand"' "$AGENTS_HOME/logs/agent.jsonl"
}

@test "agent wake refuses a task that is not paused" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  run agent wake "$t"
  [ "$status" -ne 0 ]
  [[ $output == *"not paused"* ]]
}

@test "agent wake on a remote member's proxy task asks its node to resume it" {
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  wait_status "$m" sleeping 60
  team=$(meta "$m" TEAM)
  agentbus lead-member --dir "$AGENTS_HOME/bus" --add --member member@node2 --agent member --node node2 --team "$team" \
    --task "$m" --tier normal --job job-m
  sed -i -e '/^KIND=/d' -e '/^STATUS=/d' -e '/^AGENT=/d' "$AGENTS_HOME/tasks/$m/meta.env"
  printf 'KIND=remote-member\nSTATUS=paused\nAGENT=member@node2\n' >> "$AGENTS_HOME/tasks/$m/meta.env"   # as the lead's proxy of a paused remote member
  run agent wake "$m"
  [ "$status" -eq 0 ]
  [[ $output == *"node2"* ]]
  grep -q '"event":"wake_now_requested"' "$AGENTS_HOME/logs/agent.jsonl"
}   # nothing runs on: its coordinator sleeps, and its "remote" member is a record only

@test "a node resumes its paused member at once when the lead asks (WakesNow)" {
  t=$(paused_task)
  ( source "$REPO_ROOT/bin/agent"
    # shellcheck disable=SC2034  # read by node_handle_reply, sourced above
    NODE_STATE=$(jq -nc --arg t "$t" '{jobs: {"job-m": {task_id: $t}}}')
    node_handle_reply '{"Stops": [], "Wakes": [], "WakesNow": ["job-m"], "Job": null}' )
  wait_status "$t" 'done'
}
