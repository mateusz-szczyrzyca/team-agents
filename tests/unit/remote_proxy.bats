#!/usr/bin/env bats
# A remote team member's proxy task on the lead has no local unit: the member runs on its node. agent status lists
# it; agent _supervise does not take it for a dead runner (which would put the member out of its team).

load helpers

# proxy id status — a remote member's proxy task as agent run --team made them before they had ROUND and RUN
proxy() {
  mkdir -p "$AGENTS_HOME/tasks/$1"
  printf 'ID=%s\nAGENT=gemini-2@rpi5-x\nKIND=remote-member\nREMOTE_NODE=rpi5-x\nTEAM=T\nSTATUS=%s\nREPO_NAME=repo\n' "$1" "$2" > "$AGENTS_HOME/tasks/$1/meta.env"
}

@test "agent status lists a remote member's proxy task" {
  proxy 20261027-120000-gemini-2-at-rpi5-x running
  run agent status
  [ "$status" -eq 0 ]
  [[ $output == *"gemini-2@rpi5-x"*"running"* ]]
}

@test "agent _supervise leaves a remote member's proxy task alone" {
  proxy 20261027-120000-gemini-2-at-rpi5-x running
  AGENT_SUPERVISE_GRACE_SEC=0 agent _supervise
  grep -qx 'STATUS=running' "$AGENTS_HOME/tasks/20261027-120000-gemini-2-at-rpi5-x/meta.env"
}
