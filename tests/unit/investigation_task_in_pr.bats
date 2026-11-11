#!/usr/bin/env bats
# An --investigation's reviewer got the task (agent run -f) under a heading that did not say what it was, after
# one line in the code, and the PR the merger opened did not say which task it did: the reviewer could not tell
# what to check, and the human could not see on the PR what was asked. Now the precheck writes the task, whole,
# first in the PR description, between marks of its own that it restores on every check; the format check and the
# rules do not read it as the agents' text; the reviewer's prompt holds it as the task under review, with what to
# check against it (prompts/inv-review-task.md); the merger is told to keep it.

load helpers

pipeline_setup() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude [inv3]=claude)/' \
    "$AGENTS_HOME/agents.conf"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/independent-fixture"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-pipeline-cli" "$HOME/.local/bin/claude"
  printf '\nAGENT_REVIEW_GATE=true\nAGENT_STAGE_POLL_SEC=1\n' >> "$AGENTS_HOME/agents.conf"
  git config url."$HOME/remote.git".insteadOf https://github.com/example/repo.git
  git remote set-url origin https://github.com/example/repo.git
  export PIPELINE_REAL_GIT
  PIPELINE_REAL_GIT=$(command -v git)
  mkdir -p "$HOME/pipeline-bin"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-git" "$HOME/pipeline-bin/git"
  export PATH="$HOME/pipeline-bin:$PATH"
  printf '\nRUN_PATH=%q\n' "$PATH" >> "$AGENTS_HOME/agents.conf"
  export AGENT_INVESTIGATION_SEED=7
}

parent_field() {
  bash -c 'source "$1"; printf "%s" "${!2-}"' bash "$AGENTS_HOME/tasks/$1/meta.env" "$2"
}

@test "the task block goes first in a PR description, replaced in place later; without it, the agents' text remains" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  block=$(printf '%s\n## Task\n\n%s\n%s' "$PR_TASK_START" "Do X." "$PR_TASK_END")
  body=$(pr_body_with_task "## Summary
Merged." "$block")
  [[ $body == "$PR_TASK_START"*"Do X."*"$PR_TASK_END"$'\n\n'"## Summary"$'\n'"Merged." ]]
  newer=$(printf '%s\n## Task\n\n%s\n%s' "$PR_TASK_START" "Do Y." "$PR_TASK_END")
  body2=$(pr_body_with_task "$body" "$newer")
  [[ $body2 == *"Do Y."* && $body2 != *"Do X."* && $body2 == *"## Summary"$'\n'"Merged." ]]
  [ "$(grep -c "$PR_TASK_START" <<< "$body2")" -eq 1 ]
  [ "$(pr_body_with_task "$body2" "$newer")" = "$body2" ]          # the same task again: nothing to change
  [ "$(pr_body_without_task "$body2")" = "## Summary
Merged." ]
  [ "$(pr_body_with_task "" "$block")" = "$block" ]
}

@test "a task longer than the PR description can hold is cut, saying so; the reviewers still get it whole" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/T1"
  printf '%s' "$(head -c 50000 /dev/zero | tr '\0' 'a')" > "$TASKS/T1/task.md"
  # shellcheck disable=SC2034 # read by the sourced bin/agent
  PR_TASK_MAX_CHARS=1000
  block=$(pr_task_block T1)
  [ "${#block}" -lt 1300 ]
  [[ $block == *"cut at 1000 of 50000 characters"* && $block == *"$PR_TASK_END" ]]
}

@test "--investigation: the PR description gets the task first, and the reviewer's prompt holds it as the task under review" {
  pipeline_setup
  agent _investigation-team --pipeline --team inv,inv2 "Implement the fixture feature
Expected result: fixture.txt and its test" > "$HOME/pipeline-start.log" 2>&1
  parent=$(task_of investigation-team)
  wait_status "$parent" '^done$' 45
  grep -q 'gh pr edit' "$HOME/gh.log"
  grep -q '## Task' "$HOME/gh.log"
  grep -q 'Expected result: fixture.txt and its test' "$HOME/gh.log"
  # the merger's text kept: the first test (this fixture's gh answers a PR's JSON for its description)
  reviewer=$(parent_field "$parent" INVESTIGATION_REVIEWER_TASK)
  grep -q '^# TASK UNDER REVIEW' "$HOME/phase3-$reviewer.prompt"
  grep -q 'Expected result: fixture.txt and its test' "$HOME/phase3-$reviewer.prompt"
  run grep -q '{{' "$HOME/phase3-$reviewer.prompt"
  [ "$status" -eq 1 ]
}

@test "the merger is told the script keeps the task first in the PR description" {
  grep -q "keep that part as it is" "$REPO_ROOT/prompts/inv-merge.md"
}
