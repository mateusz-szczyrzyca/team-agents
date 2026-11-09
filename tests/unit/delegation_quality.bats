#!/usr/bin/env bats
# Worker requests came in a coordinator's shorthand — "all8 status exact", "requester base831e883", "the newest easy
# board item" — and workers, who see only the request, refused them as unclear or did the wrong thing. Every agent's
# prompt now says what a worker sees and has, and how to write a request it can do.

load helpers

@test "the prompt says a worker sees only the request, has no board, and how to write a request for it" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task" > /dev/null
  t=$(task_of solo)
  wait_status "$t" 'done'
  p="$AGENTS_HOME/tasks/$t/prompt-1.md"
  grep -q "The worker sees only your request" "$p"
  grep -q "no board tools" "$p"
  grep -q "absolute paths" "$p"
  grep -qi "a full gate" "$p"
}

@test "an easy item that cannot go to a worker is the coordinator's: it hears why" {
  export AGENT_EASY_TO_WORKER_MIN=0
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); team=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$t/meta.env")
  wait_status "$t" sleeping
  wait_status "$(task_of member)" sleeping
  "$BUILD_DIR/agentbus" board-add --dir "$AGENTS_HOME/bus" --team "$team" --as lead --title "count files" \
    --details "count them, then board_done with the number" --level easy > /dev/null
  agent _supervise
  grep -q '"event":"easy_item_to_worker_failed"' "$AGENTS_HOME/logs/agent.jsonl"
  wait_grep 'Easy board item B1 (count files) is yours now' "$AGENTS_HOME/bus/messages.jsonl"
  agent stop --running > /dev/null
}
