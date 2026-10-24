#!/usr/bin/env bats
# "done" is a status name passed as an argument (wait_status "$t" done), not the end of a loop.
# shellcheck disable=SC1010
# Provider limits (README "Waking, pausing and quotas"): an exhausted quota or credits pauses a task until the
# limit resets; a rate limit (HTTP 429) is not an exhausted quota — a short pause; a 429 the CLI already retried,
# long before the run ended, pauses nothing.

load helpers

# meta id key — a value from a task's meta.env
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "a rate limit (HTTP 429) is a short pause, not an exhausted quota, and the notification shows its line" {
  printf 'AGENT_RATE_LIMIT_PAUSE_MIN=4\nAGENT_QUOTA_PAUSE_MIN=30\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "rate-limit"}}'
  before=$(date +%s)
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" paused
  [ "$(meta "$t" PAUSE_REASON)" = rate_limit ]
  wake=$(meta "$t" WAKE_AT)
  (( wake >= before + 4 * 60 + 60 && wake <= before + 4 * 60 + 60 + 30 ))
  wait_grep "rate-limiting" "$HOME/notifications.log"
  grep -q "429 Too Many Requests" "$HOME/notifications.log"
  run grep -q "quota or usage limit is exhausted" "$HOME/notifications.log"
  [ "$status" -ne 0 ]
}

@test "an exhausted quota still pauses until its reset and names the line that said so" {
  scenario '{"solo": {"new": "quota"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" paused
  [ "$(meta "$t" PAUSE_REASON)" = quota ]
  wait_grep "Individual quota reached" "$HOME/notifications.log"
}

@test "a 429 the CLI retried long before the run ended does not pause the task" {
  scenario '{"solo": {"new": "stale-rate-limit"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" 'waiting|done|failed|paused'
  [ "$(meta "$t" STATUS)" != paused ]
}
