#!/usr/bin/env bats
# PR #67 was reviewed and merged by --autopilot — found at its commit, on another team's branch —, and the next round
# did not start: "round 1/2 has no PR (branches …)", for the next round looked for its PR among its team's branches
# only. It now takes, when they have none, the PR the round was reviewed on (PR_NUMBER of its tasks).

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
set_meta() { (source "$REPO_ROOT/bin/agent"; meta_set "$1" "$2" "$3"); }

@test "the next round starts after a merged PR the round was reviewed on, on another team's branch" {
  scenario '{"lead": {"new": "done"}, "member": {"new": "done"}}'
  agent run --team lead,member "round work" > /dev/null
  t=$(task_of lead)
  wait_status "$t" 'done'
  wait_status "$(task_of member)" 'done'
  team=$(meta "$t" TEAM)
  set_meta "$t" WORK_ROUND 1
  set_meta "$t" WORK_ROUNDS 2
  set_meta "$t" PR_NUMBER 67
  printf '[{"number": 67, "url": "https://github.com/o/repo/pull/67", "headRefName": "agents/gemini-1/old", "state": "MERGED"}]' > "$HOME/gh-prs.json"
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  run agent run --resume "$team" "round 2"
  [[ $output != *"has no PR"* ]]
  [ "$status" -eq 0 ]
  t2=$(task_of lead)
  [ "$t2" != "$t" ]
  [ "$(meta "$t2" BRANCH)" != "$(meta "$t" BRANCH)" ]   # a fresh branch: the next round
  grep -q "Round 2 of 2" "$AGENTS_HOME/tasks/$t2/task.md"
  agent stop --running > /dev/null
}
