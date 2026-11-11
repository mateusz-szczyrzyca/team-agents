#!/usr/bin/env bats
# An --investigation's merged PR that changed what only the human may allow (AGENTS.md "Never") failed its checks
# like any other: the merger was sent back to fix what it could not fix, asked the human, and the second failure
# — and one replacement merger — ended the investigation as "merge failed", with no review. Now checks that find
# only what the human must allow record the allow command and wait for the human: the investigation is paused, the
# merger is not sent back, no retry is spent. agent allow for the investigation runs its checks again, and the
# pipeline goes on to the review.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# inv_parent [extra-file] — an investigation P in phase 2 whose merger M is done and whose published PR branch
# changes AGENTS.md (only the human may allow it), and with an extra file the round's own fault
inv_parent() {
  cd "$HOME/repo" || return 1
  printf '# rules\n' > AGENTS.md
  git add AGENTS.md && git -c user.name=t -c user.email=t@t commit -qm rules && git push -q origin main
  git checkout -q -b agents/inv/merge
  printf '# rules, changed\n' > AGENTS.md
  if [[ -n ${1:-} ]]; then mkdir -p "$(dirname "$1")"; printf 'echo x\n' > "$1"; fi
  git add -A && git -c user.name=t -c user.email=t@t commit -qm merged && git push -q -u origin agents/inv/merge
  git checkout -q main
  printf '[{"number": 80, "url": "https://github.com/example/repo/pull/80", "state": "OPEN", "headRefName": "agents/inv/merge", "baseRefName": "main"}]\n' \
    > "$HOME/gh-prs.json"
  printf '## Summary\nMerged.\n' > "$HOME/gh-pr-body.md"
  mkdir -p "$AGENTS_HOME/tasks/P" "$AGENTS_HOME/tasks/M"
  printf '%s\n' ID=P AGENT=inv KIND=investigation_team STATUS=running RUN=1 TEAM= REPO_NAME=repo "REPO_ROOT=$HOME/repo" \
    BRANCH=agents/inv/merge BASE=main INVESTIGATION_PIPELINE=1 INVESTIGATION_PHASE=2 INVESTIGATION_MERGER_TASK=M \
    > "$AGENTS_HOME/tasks/P/meta.env"
  printf 'the task\n' > "$AGENTS_HOME/tasks/P/task.md"
  printf '%s\n' ID=M AGENT=inv KIND=investigation STATUS=done TEAM= PARENT_TASK=P BRANCH=agents/inv/merge BASE=main \
    > "$AGENTS_HOME/tasks/M/meta.env"
  printf 'merge the results\n' > "$AGENTS_HOME/tasks/M/task.md"
  printf "\nAGENT_REVIEW_GATE='true'\n" >> "$AGENTS_HOME/agents.conf"
}

@test "checks that find only what the human must allow wait for the human: paused, with the allow command, not failed" {
  inv_parent
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  dispatch() { :; }
  cmd__precheck P
  [ "$(meta P INVESTIGATION_PRECHECK)" = human ]
  [ "$(investigation_field P INVESTIGATION_PRECHECK_ALLOW)" = "agent allow P AGENTS.md" ]
  grep -q '"event":"investigation_waits_for_allow"' "$AGENTS_HOME/logs/agent.jsonl"
}

@test "checks that also find what the team can fix fail as before: the merger goes back with the reasons" {
  inv_parent bin/tool.sh
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  dispatch() { :; }
  cmd__precheck P
  [ "$(meta P INVESTIGATION_PRECHECK)" = failed ]
  [ -z "$(investigation_field P INVESTIGATION_PRECHECK_ALLOW)" ]
}

@test "while the allowance is pending the pipeline waits: the merger is not sent back and no retry is spent" {
  inv_parent
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  meta_set P INVESTIGATION_PRECHECK human
  investigation_pipeline_tick P
  [ "$(meta P INVESTIGATION_PRECHECK)" = human ]
  [ -z "$(investigation_field P INVESTIGATION_MERGE_RETRIES)" ]
  [ "$(meta P INVESTIGATION_PHASE)" = 2 ]
  run grep -q 'Checks to fix' "$AGENTS_HOME/tasks/M/task.md"
  [ "$status" -ne 0 ]
}

@test "agent allow for an investigation waiting on it runs its checks again" {
  inv_parent
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  meta_set P INVESTIGATION_PRECHECK human
  meta_set P INVESTIGATION_PRECHECK_ALLOW "agent allow P AGENTS.md"
  run agent allow P AGENTS.md
  [ "$status" -eq 0 ]
  [[ $output == *"checks run again"* ]]
  [ "$(meta P INVESTIGATION_PRECHECK)" != human ]
  grep -q '"event":"investigation_precheck_again"' "$AGENTS_HOME/logs/agent.jsonl"
}
