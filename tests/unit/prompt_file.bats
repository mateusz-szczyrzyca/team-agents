#!/usr/bin/env bats
# A prompt too long for one command-line argument (Linux takes 128 KiB) reaches the CLI through .agent-prompt.md in its
# working directory — kept out of git —, and the CLI is asked to read it; a prompt that fits goes as before.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "a prompt too long for the command line reaches the CLI through a file in its working directory, kept out of git" {
  scenario '{"solo": {"new": "done"}}'
  printf 'A long task: %s\n' "$(head -c 200000 /dev/zero | tr '\0' x)" > "$BATS_TEST_TMPDIR/task.md"
  agent run solo -f "$BATS_TEST_TMPDIR/task.md"
  t=$(task_of solo)
  wait_status "$t" 'done'
  wt=$(meta "$t" WORKTREE)
  grep -q 'A long task' "$wt/.agent-prompt.md"
  run ! grep -q 'agent-prompt' <<< "$(git -C "$wt" status --porcelain)"
}

@test "a prompt that fits is passed as before, without a file" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a short task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  [ ! -e "$(meta "$t" WORKTREE)/.agent-prompt.md" ]
}
