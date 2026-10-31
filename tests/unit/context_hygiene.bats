#!/usr/bin/env bats
# The agents keep their context small: what a step prints is read again at every later step. The prompts say what that
# costs and how to read large files and diffs; the coordinator has unknown code summed up by a worker first; a reviewer
# reads the PR file by file.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "a team member's prompt says what output costs, how to read large files and diffs; the coordinator asks a worker first" {
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  p=$AGENTS_HOME/tasks/$m/prompt-1.md
  # the prompt is written before the run starts: no need to wait for the member's sleep
  for _ in $(seq 1 300); do grep -q "Keep tool output short" "$p" 2> /dev/null && break; sleep 0.2; done
  grep -q "300 steps" "$p"
  grep -q "sed -n" "$p"
  grep -q "git diff --stat" "$p"
  grep -q "a worker's summary" "$p"
  wait_status "$m" sleeping 60   # closed asleep, as agent team --close expects
  agent team "$(meta "$m" TEAM)" --close > /dev/null
}

@test "the review prompt reads the PR file by file, not its whole diff at once" {
  # the template itself: rendering a review needs a PR
  grep -q -- "--name-only" "$REPO_ROOT/prompts/review.md"
  run ! grep -qE '`gh pr diff \{\{PR_NUMBER\}\}`' "$REPO_ROOT/prompts/review.md"
}
