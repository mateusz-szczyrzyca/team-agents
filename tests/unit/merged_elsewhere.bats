#!/usr/bin/env bats
# A round's PR merged by someone else while its round goes on. #53 was merged by hand while its fix pass ran: the
# team went on working on the merged PR's branch, pushed a commit after the merge (on no PR, not on main), and a
# review of the merged PR started. _supervise now stops what still works on a round whose PR someone else merged —
# the team, its reviews — and tells the human, once, with the commits pushed after the merge. The autopilot's own
# merge (PR_MERGE_OWN) is left alone.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
set_meta() { (source "$REPO_ROOT/bin/agent"; meta_set "$1" "$2" "$3"); }

# merged_pr task — the task's branch pushed, a commit on it, the PR merged at that commit; then one more commit,
# pushed after the merge
merged_pr() {
  local t=$1 wt br sha
  wt=$(meta "$t" WORKTREE); br=$(meta "$t" BRANCH)
  git -C "$wt" -c user.name=t -c user.email=t@t commit -q --allow-empty -m "the round's work"
  sha=$(git -C "$wt" rev-parse HEAD)
  git -C "$wt" push -q origin "HEAD:$br"
  git -C "$wt" -c user.name=t -c user.email=t@t commit -q --allow-empty -m "pushed after the merge"
  git -C "$wt" push -q origin "HEAD:$br"
  printf '{"number": 7, "state": "MERGED", "mergedAt": "2026-11-04T08:46:34Z", "headRefOid": "%s", "headRefName": "%s", "baseRefName": "main", "url": "https://github.com/o/r/pull/7"}\n' \
    "$sha" "$br" > "$HOME/gh-pr-view.json"
  set_meta "$t" PR_NUMBER 7
  set_meta "$t" PR_URL https://github.com/o/r/pull/7
}

@test "a team still at work on a round whose PR someone else merged is stopped; the human hears of the commits after it, once" {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member)
  wait_status "$t" running
  wait_status "$m" running
  merged_pr "$t"
  agent _supervise
  wait_status "$t" stopped
  wait_status "$m" stopped
  wait_grep "PR #7" "$HOME/notifications.log"
  grep -q "pushed after the merge" "$HOME/notifications.log"
  grep -q '"event":"round_pr_merged_elsewhere"' "$AGENTS_HOME/logs/agent.jsonl"
  agent _supervise
  [ "$(grep -c "pushed after the merge" "$HOME/notifications.log")" -eq 1 ]
}

@test "a review of a PR someone else merged is stopped" {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead)
  wait_status "$t" running
  agent stop --running > /dev/null
  merged_pr "$t"
  mkdir -p "$AGENTS_HOME/tasks/R1"
  printf 'ID=R1\nAGENT=inv\nKIND=investigation\nREVIEW=1\nSTATUS=running\nPARENT_TASK=%s\nREPO_ROOT=%s\n' \
    "$t" "$(meta "$t" REPO_ROOT)" > "$AGENTS_HOME/tasks/R1/meta.env"
  agent _supervise
  [ "$(meta R1 STATUS)" = stopped ]
}

@test "the autopilot's own merge stops nothing" {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member)
  wait_status "$t" running
  merged_pr "$t"
  set_meta "$t" PR_MERGE_OWN 1
  agent _supervise
  [ "$(meta "$t" STATUS)" = running ]
  [ "$(meta "$m" STATUS)" = running ]
  run ! grep -q "round_pr_merged_elsewhere" "$AGENTS_HOME/logs/agent.jsonl"
  agent stop --running > /dev/null
}
