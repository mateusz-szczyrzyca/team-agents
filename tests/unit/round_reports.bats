#!/usr/bin/env bats
# "done" is a status name passed as an argument (wait_status "$t" done), not the end of a loop.
# shellcheck disable=SC1010
# The team reports the protocol writes (.agents/reports/) are not code to review: they never count towards a
# round's size, whatever ROUND_EXCLUDE says.

load helpers

# meta id key — a value from a task's meta.env
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "a report under .agents/reports/ does not count towards the round size" {
  printf 'ROUND_BOUNCES=1\nROUND_EXCLUDE="vendor/"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "big-report", "resume": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  [ "$(meta "$t" RUN)" = 1 ]
}
