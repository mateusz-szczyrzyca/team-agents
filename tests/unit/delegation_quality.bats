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
