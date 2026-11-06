#!/usr/bin/env bats
# A new team was started on PR #67's review findings: on its own branches, unknown to the script as that PR's team, so
# its round went unreviewed. agent run --pr N starts a team on an open PR: its tasks start at the PR's head, their base
# is the PR's base, and the round's PR is N from the first run. And a task that names an open PR without --pr is told
# how to continue it.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# open_pr — PR #67, open, on the pushed branch agents/gemini-1/old one commit ahead of main; prints its head
open_pr() {
  git -C "$HOME/repo" checkout -q -b agents/gemini-1/old main
  echo fix > "$HOME/repo/fix.txt"
  git -C "$HOME/repo" add fix.txt && git -C "$HOME/repo" -c user.name=t -c user.email=t@t commit -q -m "the PR's work"
  git -C "$HOME/repo" push -q origin agents/gemini-1/old
  sha=$(git -C "$HOME/repo" rev-parse HEAD)
  git -C "$HOME/repo" checkout -q main
  printf '[{"number": 67, "url": "https://github.com/o/repo/pull/67", "headRefName": "agents/gemini-1/old", "headRefOid": "%s", "baseRefName": "main", "state": "OPEN"}]' \
    "$sha" > "$HOME/gh-prs.json"
}

@test "agent run --pr: the team starts at the PR's head, on the PR's base, with the PR as its round's PR" {
  open_pr
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member --pr 67 "Fix the findings of the review" > /dev/null
  t=$(task_of lead); m=$(task_of member)
  [ "$(git -C "$(meta "$t" WORKTREE)" rev-parse HEAD)" = "$sha" ]
  [ "$(git -C "$(meta "$m" WORKTREE)" rev-parse HEAD)" = "$sha" ]
  [ "$(meta "$t" BASE)" = main ]
  [ "$(meta "$t" ROUND_PR)" = 67 ]
  p=$AGENTS_HOME/tasks/$t/prompt-1.md
  wait_grep "This round's PR is #67" "$p"
  grep -q "agents/gemini-1/old" "$p"
  agent stop --running > /dev/null
}

@test "agent run --pr refuses a PR that is not open here, and starts nothing" {
  echo '[]' > "$HOME/gh-prs.json"
  run agent run --pr 99 solo "a task"
  [ "$status" -ne 0 ]
  [[ $output == *"#99"* ]]
  [ -z "$(task_of solo)" ]
}

@test "a task that names an open PR, without --pr, is told how to continue that PR" {
  open_pr
  scenario '{"solo": {"new": "done"}}'
  run agent run solo "Fix the findings of PR #67"
  [ "$status" -eq 0 ]
  [[ $output == *"--pr 67"* ]]
}
