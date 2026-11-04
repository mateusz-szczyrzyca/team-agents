#!/usr/bin/env bats
# The PR rule names the PR the script knows, instead of asking an agent to look for "a PR about the subject" — which
# could have it push to another task's PR. It goes where a PR is opened or kept only (a task that owns its PR, a
# team); the agents' gh refuses a second PR to a task whose PR is open.

load helpers

setup_gh() {
  mkdir -p "$BATS_TEST_TMPDIR/ghw"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"
  GH=$BATS_TEST_TMPDIR/ghw/gh
  export AGENT_REAL_GH=$REPO_ROOT/tests/unit/stubs/gh
  export AGENT_GITHUB_TRUSTED_RESOLVED="mateusz-szczyrzyca ms-ai-agents"
  log=$AGENTS_HOME/logs/agent.jsonl
  mkdir -p "$AGENTS_HOME/logs"
}

@test "pr_ownership_note: an owner's known PR with its branch, else one to open; a team's rule; nothing for a reviewer" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  out=$(KIND=worker TEAM='' REVIEW='' PR_NUMBER=7 PR_URL='' BRANCH=agents/solo/1 BASE=main pr_ownership_note)
  [[ $out == *"This task's PR is #7 (branch agents/solo/1)"* ]]
  out=$(KIND=worker TEAM='' REVIEW='' PR_NUMBER='' PR_URL='' BRANCH=agents/solo/1 BASE=main pr_ownership_note)
  [[ $out == *"opens one PR"* ]]
  out=$(KIND=worker TEAM=T-team REVIEW='' PR_NUMBER='' PR_URL='' pr_ownership_note)
  [[ $out == *"a member opens none"* ]]
  out=$(KIND=investigation REVIEW=1 TEAM='' pr_ownership_note)
  [ -z "$out" ]
}

@test "the agents' gh refuses a second PR to a task whose PR is open, and lets one be opened once it is merged" {
  setup_gh
  printf '{"state": "OPEN"}\n' > "$HOME/gh-pr-view.json"
  AGENT_GH_OWN_PR=7 run "$GH" pr create --base main --head agents/solo/1 --title t --body b
  [ "$status" -ne 0 ]
  [[ $output == *"#7"* ]]
  grep -q '"event":"github_second_pr_refused"' "$log"
  printf '{"state": "MERGED"}\n' > "$HOME/gh-pr-view.json"
  : > "$STUB_HOME/gh.log"
  AGENT_GH_OWN_PR=7 run "$GH" pr create --base main --head agents/solo/1 --title t --body b
  grep -q 'gh pr create --base main --head agents/solo/1' "$STUB_HOME/gh.log"
}
