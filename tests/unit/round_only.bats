#!/usr/bin/env bats
# What the checks and the reviews send back to a team (a fix pass, a bounce) and a round's own heading are for that
# round only: between round-only marks in the task's text, left out when the next round starts. The human's own
# additions stay.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# stopped_team — a team of lead and member, stopped while it ran; prints the lead's task
stopped_team() {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  local old
  old=$(task_of lead)
  wait_status "$old" running
  agent stop --running > /dev/null
  echo "$old"
}

@test "a fix pass's text is for its round only; the human's addition is not" {
  old=$(stopped_team)
  AGENT_FIX_PASS=1 agent run --resume "$(meta "$old" TEAM)" --same-round "fix finding X" > /dev/null
  t=$(task_of lead)
  grep -q "^<!-- round only" "$AGENTS_HOME/tasks/$t/task.md"
  grep -q "fix finding X" "$AGENTS_HOME/tasks/$t/task.md"
  wait_status "$t" running
  agent stop --running > /dev/null
  agent run --resume "$(meta "$t" TEAM)" --same-round "human note Y" > /dev/null
  t2=$(task_of lead)
  grep -q "## Addition from the human" "$AGENTS_HOME/tasks/$t2/task.md"
  grep -q "fix finding X" "$AGENTS_HOME/tasks/$t2/task.md"   # the same round: still there
  agent stop --running > /dev/null
}

@test "the next round's text leaves out what was for the round before only" {
  printf 'the task\n\n<!-- round only: left out of the next round -->\n## For this round only\n\nfix finding X\n<!-- /round only -->\n\n## Addition from the human\n\nhuman note Y\n' \
    > "$BATS_TEST_TMPDIR/task.md"
  out=$(source "$REPO_ROOT/bin/agent"; round_text_strip < "$BATS_TEST_TMPDIR/task.md")
  [[ $out == *"the task"* && $out == *"human note Y"* ]]
  [[ $out != *"fix finding X"* && $out != *"round only"* ]]
}
