#!/usr/bin/env bats
# A team fixed PR #67's review findings and pushed them to that PR's branch, but it was a new team on its own
# branches: no PR came from its coordinator's branch, so --verify skipped the review with a warning, and the round
# ended unreviewed. The round's PR is now found by its branch, else by the round's PR the team was given (ROUND_PR),
# else by an open PR whose head is the coordinator's commit; when none is, the human hears ⛔ — no review, and why.

load helpers

# repo_task id branch — a coordinator's task on a branch of the test repository, at a new commit of its own
repo_task() {
  git -C "$HOME/repo" branch -f "$2" main
  mkdir -p "$TASKS/$1"
  printf 'ID=%s\nTEAM=T1\nAGENT=lead\nREPO_NAME=repo\nREPO_ROOT=%s\nWORKTREE=%s\nBRANCH=%s\nBASE=main\nSTATUS=done\nRUN=1\n' \
    "$1" "$HOME/repo" "$HOME/repo" "$2" > "$TASKS/$1/meta.env"
}

@test "round_pr_for: the PR on the branch, else the round's PR it was given, else an open PR at the branch's commit" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo_task C1 agents/lead/1
  sha=$(git -C "$HOME/repo" rev-parse agents/lead/1)
  printf '[{"number": 67, "url": "https://github.com/o/repo/pull/67", "headRefName": "agents/gemini-1/old", "headRefOid": "%s", "state": "OPEN"}]' \
    "$sha" > "$HOME/gh-prs.json"
  [ "$(round_pr_for C1 number,url | jq -r '.[0].number')" = 67 ]
  printf '[{"number": 67, "url": "u67", "headRefName": "agents/gemini-1/old", "headRefOid": "0000000", "state": "OPEN"},
           {"number": 70, "url": "u70", "headRefName": "agents/x/y", "headRefOid": "1111111", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  [ "$(round_pr_for C1 number,url | jq -r 'length')" = 0 ]
  meta_set C1 ROUND_PR 70
  [ "$(round_pr_for C1 number,url | jq -r '.[0].number')" = 70 ]
}

@test "a review whose round has no PR anywhere tells the human ⛔: no review, and what to do" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo_task C2 agents/lead/2
  echo '[]' > "$HOME/gh-prs.json"
  request_review C2
  f=$(ls -t "$NOTIFY_DIR"/*-summary.md | head -n 1)
  [ "$(head -n 1 "$f")" = "⛔ repo" ]
  grep -q "no PR" "$f"
  grep -q -- "--same-round" "$f"
}

@test "closing a team finds the round's PR at its commit: no gap 'no PR from your branch'" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo_task C3 agents/lead/3
  sha=$(git -C "$HOME/repo" rev-parse agents/lead/3)
  printf '[{"number": 67, "url": "u67", "headRefName": "agents/gemini-1/old", "headRefOid": "%s", "state": "OPEN", "isDraft": false}]' "$sha" > "$HOME/gh-prs.json"
  # shellcheck disable=SC2034 # read by verify_gaps
  BRANCH=agents/lead/3 WORKTREE=$HOME/repo BASE=main
  out=$(verify_gaps C3)
  [[ $out != *"no PR from your branch"* ]]
}
