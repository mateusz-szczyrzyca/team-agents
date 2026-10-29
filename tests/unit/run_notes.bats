#!/usr/bin/env bats
# What a run ended on reaches whoever looks: a spent workspace budget is a quota pause until its reset (an ISO
# time); a failed run keeps its note — the CLI's exit code and its last error line — for the lead to show; the lead
# hands a remote member's pause, with its reason, to the proxy task.

load helpers

@test "a spent workspace budget is a quota pause until its reset, read from an ISO time" {
  source "$REPO_ROOT/bin/agent"
  d=$BATS_TEST_TMPDIR/t
  mkdir -p "$d"
  now=$(date +%s)
  reset=$((now + 3 * 86400))
  printf 'Workspace monthly budget of $200.00 exceeded. Contact your org admin. $200.18 of $200.00 used; resets %s.\n' \
    "$(date -u -d "@$reset" +%Y-%m-%dT%H:%M:%S.000Z)" > "$d/err-1.log"
  [ "$(limit_kind "$d" 1 "")" = quota ]
  secs=$(quota_reset_seconds "$d" 1 "")
  (( secs >= reset - now + 60 - 5 && secs <= reset - now + 60 + 5 ))
}

@test "a failed run keeps its note: the CLI's exit code and its last error line, for the lead to show" {
  scenario '{"solo": {"new": "cli-error"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" failed
  for _ in $(seq 1 100); do grep -q '^NOTE=' "$AGENTS_HOME/tasks/$t/meta.env" && break; sleep 0.1; done   # written at the run's end
  # node_job_result is what a node reports to its lead
  out=$(source "$REPO_ROOT/bin/agent"; node_job_result "$t" failed)   # sourcing ends with a non-zero status
  [[ $out == *"CLI exit code: 1"* && $out == *"invalid argument --frobnicate"* ]]
}

@test "agent _remote_member: a remote member's pause, with its reason, reaches the proxy task and the human" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  sed -i '/^KIND=/d' "$AGENTS_HOME/tasks/$t/meta.env"
  echo 'KIND=remote-member' >> "$AGENTS_HOME/tasks/$t/meta.env"   # as the lead's proxy task of a remote member
  agent _remote_member "$t" paused "" "the provider's quota or usage limit is exhausted — resumes at 01.11 00:01"
  [ "$(sed -n 's/^STATUS=//p' "$AGENTS_HOME/tasks/$t/meta.env")" = paused ]
  grep -q 'usage limit is exhausted' "$AGENTS_HOME/tasks/$t/activity.jsonl"
  wait_grep "usage limit is exhausted" "$HOME/notifications.log"
}
