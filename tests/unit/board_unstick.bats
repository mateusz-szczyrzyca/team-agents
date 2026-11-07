#!/usr/bin/env bats
# A team stood still 8 hours: its coordinator put easy items on the board for workers, with no worker in the team and
# its members told to take only their own items; two items waited for a member who had finished; everybody slept.
# Now easy items nobody owns go to a worker, as the coordinator's request; a finished member's items go back to the
# board; and a team whose members all sleep while items wait past AGENT_BOARD_STALL_MIN tells the human ⛔, once.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
board() { "$BUILD_DIR/agentbus" board-dump --dir "$AGENTS_HOME/bus" --team "$1" | jq -c --arg id "$2" '.board[$id]'; }

# open_team — a team whose coordinator (lead) and member both sleep, the team open
open_team() {
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "idle"}, "worker": {"new": "sleep:300"}, "worker2": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead)
  team=$(meta "$t" TEAM)
  wait_status "$t" sleeping
  wait_status "$(task_of member)" sleeping
}

@test "an easy item nobody owns goes to a worker as the coordinator's request; the coordinator holds it" {
  export AGENT_EASY_TO_WORKER_MIN=0
  open_team
  "$BUILD_DIR/agentbus" board-add --dir "$AGENTS_HOME/bus" --team "$team" --as lead --title "count the files" --details "how many" --level easy > /dev/null
  agent _supervise
  [ "$(board "$team" B1 | jq -r '.status + " " + .assignee')" = "claimed lead" ]
  "$BUILD_DIR/agentbus" helper-list --dir "$AGENTS_HOME/bus" | grep -q "count the files"
  grep -q '"event":"easy_item_to_worker"' "$AGENTS_HOME/logs/agent.jsonl"
  agent stop --running > /dev/null
}

@test "a member that finishes gives back the items designated for it" {
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "sleep:4"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member)
  team=$(meta "$t" TEAM)
  wait_status "$m" running
  "$BUILD_DIR/agentbus" board-add --dir "$AGENTS_HOME/bus" --team "$team" --as lead --title "for the member" --details "d" --level normal --for member > /dev/null
  wait_status "$m" 'done'
  for _ in $(seq 1 100); do [ "$(board "$team" B1 | jq -r '.for // ""')" = "" ] && break; sleep 0.2; done
  [ "$(board "$team" B1 | jq -r '(.for // "") + "|" + .previous')" = "|member" ]
  agent stop --running > /dev/null
}

@test "a team whose members all sleep while items wait past AGENT_BOARD_STALL_MIN tells the human ⛔, once" {
  export AGENT_BOARD_STALL_MIN=0 AGENT_EASY_TO_WORKER_MIN=999
  open_team
  # an easy item: with no worker in the team it wakes nobody (a normal one would wake a member, at work for a while)
  "$BUILD_DIR/agentbus" board-add --dir "$AGENTS_HOME/bus" --team "$team" --as lead --title "nobody's" --details "d" --level easy > /dev/null
  agent _supervise
  agent _supervise
  [ "$(grep -l "standstill" "$AGENTS_HOME"/notifications/*.md | wc -l)" -eq 1 ]
  f=$(grep -l "standstill" "$AGENTS_HOME"/notifications/*.md)
  [ "$(head -n 1 "$f")" = "⛔ repo" ]
  grep -q "B1" "$f"
  agent stop --running > /dev/null
}

@test "a stopped team is neither reported at a standstill nor has its easy items given to workers" {
  export AGENT_BOARD_STALL_MIN=0 AGENT_EASY_TO_WORKER_MIN=0
  open_team
  "$BUILD_DIR/agentbus" board-add --dir "$AGENTS_HOME/bus" --team "$team" --as lead --title "nobody's" --details "d" --level normal > /dev/null
  "$BUILD_DIR/agentbus" board-add --dir "$AGENTS_HOME/bus" --team "$team" --as lead --title "an easy one" --details "d" --level easy > /dev/null
  agent stop --running > /dev/null
  agent _supervise
  ! grep -l "standstill" "$AGENTS_HOME"/notifications/*.md 2> /dev/null || false
  [ "$(board "$team" B2 | jq -r .status)" = todo ]
  ! grep -q '"event":"board_stall"' "$AGENTS_HOME/logs/agent.jsonl" || false
}

@test "a team none of whose members was at work in AGENT_BOARD_RECENT_H is left alone: an old one, never closed" {
  export AGENT_BOARD_STALL_MIN=0 AGENT_EASY_TO_WORKER_MIN=0 AGENT_BOARD_RECENT_H=6
  open_team
  "$BUILD_DIR/agentbus" board-add --dir "$AGENTS_HOME/bus" --team "$team" --as lead --title "an old one" --details "d" --level easy > /dev/null
  local t pid
  for t in $(grep -lx "TEAM=$team" "$AGENTS_HOME"/tasks/*/meta.env | xargs -n1 dirname); do
    for _ in $(seq 1 150); do   # its runner done writing — a last line would make the activity new again
      pid=$(cat "$t/runner.pid" 2> /dev/null) || break
      kill -0 "$pid" 2> /dev/null || break
      sleep 0.2
    done
    touch -d '-7 hours' "$t/activity.jsonl"
  done
  agent _supervise
  ! grep -l "standstill" "$AGENTS_HOME"/notifications/*.md 2> /dev/null || false
  [ "$(board "$team" B1 | jq -r .status)" = todo ]
  agent stop --running > /dev/null
}
