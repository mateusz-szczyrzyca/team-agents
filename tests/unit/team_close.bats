#!/usr/bin/env bats
# Closing a team: agent team <id> --close, whatever its members do; a remote member's job ended on the way, a
# failure to end it logged with its reason; an open team with no local member left reported to the human once.

load helpers

# meta task key — a field of a task (as in verify_loop.bats)
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "agent team <id> --close closes a team whose members sleep" {
  scenario '{"lead": {"new": "team-lead-wait-read"}, "member": {"new": "team-sleep-read"}}'
  agent run --team lead,member "task"
  t=$(task_of lead); m=$(task_of member)
  team=$(meta "$t" TEAM)
  wait_status "$t" sleeping
  wait_status "$m" sleeping
  run agent team "$team" --close
  [ "$status" -eq 0 ]
  [[ $(agentbus team-get --dir "$AGENTS_HOME/bus" --team "$team" | jq -r .done) == true ]]
  [[ $(meta "$t" STATUS) =~ ^(done|stopped)$ ]]
  [[ $(meta "$m" STATUS) =~ ^(done|stopped)$ ]]
}

@test "closing a team ends its remote member's job; a failure to end it is logged with its reason" {
  local d="$AGENTS_HOME/tasks"
  mkdir -p "$d/20261028-100000-lead" "$d/20261028-100000-solo-at-node2"
  printf 'ID=20261028-100000-lead\nAGENT=lead\nTYPE=claude\nTEAM=T1\nSTATUS=sleeping\nREPO_NAME=repo\nRUN=1\nROUND=0\n' > "$d/20261028-100000-lead/meta.env"
  printf 'ID=20261028-100000-solo-at-node2\nAGENT=solo@node2\nKIND=remote-member\nREMOTE_NODE=node2\nJOB_ID=member-x\nTEAM=T1\nSTATUS=running\nREPO_NAME=repo\nRUN=0\nROUND=0\n' \
    > "$d/20261028-100000-solo-at-node2/meta.env"
  jq -nc --arg home "$AGENTS_HOME" '{id: "T1", repo: "repo", wake_cmd: "/bin/true", home: $home, members: {
      lead: {agent: "lead", task: "20261028-100000-lead", state: "sleeping", tier: "normal"},
      "solo@node2": {agent: "solo@node2", task: "20261028-100000-solo-at-node2", state: "active", tier: "normal", provider: "remote"}}}' \
    | agentbus team-create --dir "$AGENTS_HOME/bus"
  run agent team T1 --close
  [ "$status" -eq 0 ]
  [[ $(agentbus team-get --dir "$AGENTS_HOME/bus" --team T1 | jq -r .done) == true ]]
  # not a lead here: the job cannot be cancelled — the log says why, not only that
  grep '"event":"remote_member_cancel_failed"' "$AGENTS_HOME"/logs/agent.jsonl | grep -q '"reason":"[^"]'
}

@test "an open team with no local member left is reported to the human, once, with how to resume or close it" {
  # as team 20261028-173504: its local members failed, its remote member sleeps on its node — the team stays open
  local d="$AGENTS_HOME/tasks"
  mkdir -p "$d/20261028-110000-lead" "$d/20261028-110000-solo-at-node2"
  printf 'ID=20261028-110000-lead\nAGENT=lead\nTYPE=claude\nTEAM=T2\nSTATUS=failed\nREPO_NAME=repo\nRUN=1\nROUND=0\n' > "$d/20261028-110000-lead/meta.env"
  printf 'ID=20261028-110000-solo-at-node2\nAGENT=solo@node2\nKIND=remote-member\nREMOTE_NODE=node2\nJOB_ID=member-y\nTEAM=T2\nSTATUS=sleeping\nREPO_NAME=repo\nRUN=0\nROUND=0\n' \
    > "$d/20261028-110000-solo-at-node2/meta.env"
  jq -nc --arg home "$AGENTS_HOME" '{id: "T2", repo: "repo", wake_cmd: "/bin/true", home: $home, members: {
      lead: {agent: "lead", task: "20261028-110000-lead", state: "done", tier: "normal"},
      "solo@node2": {agent: "solo@node2", task: "20261028-110000-solo-at-node2", state: "sleeping", tier: "normal", provider: "remote"}}}' \
    | agentbus team-create --dir "$AGENTS_HOME/bus"
  AGENT_TEAM_ABANDON_MIN=0 AGENT_SUPERVISE_GRACE_SEC=0 agent _supervise
  wait_grep "agent team T2 --close" "$HOME/notifications.log"
  grep -q "agent run --resume T2" "$HOME/notifications.log"
  AGENT_TEAM_ABANDON_MIN=0 AGENT_SUPERVISE_GRACE_SEC=0 agent _supervise
  [ "$(grep -c "agent team T2 --close" "$HOME/notifications.log")" -eq 1 ]
  [[ $(agentbus team-get --dir "$AGENTS_HOME/bus" --team T2 | jq -r .done) == false ]]   # reported, not closed
}
