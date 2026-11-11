#!/usr/bin/env bats
# An --investigation whose review asked for changes stopped there, waiting for the human, and what came after made
# new pull requests: a new run, or a replacement merger, opened a PR of its own branch. Now a review asking for
# changes (needs-fixes, incomplete) sends the merger back to fix them on its branch — the same PR — then the checks
# and a new review run again, each pass reviewed by a new investigator from outside the team (when all have
# reviewed, the one whose review is the oldest; never the merger), up to VERIFY_MAX_FIX_PASSES passes; then the
# human decides. Once its PR exists a failed merger is not replaced: a replacement would open another PR.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# inv [phase] — investigation P (team a,b; merger c, an outsider; outsiders d,e) with PR #81 of merger M's branch,
# reviewed by d in task R, whose review asks for changes
inv() {
  mkdir -p "$AGENTS_HOME/tasks/P" "$AGENTS_HOME/tasks/M" "$AGENTS_HOME/tasks/R"
  printf '%s\n' ID=P AGENT=a KIND=investigation_team STATUS=running RUN=1 TEAM= REPO_NAME=repo "REPO_ROOT=$HOME/repo" \
    INVESTIGATION_PIPELINE=1 "INVESTIGATION_PHASE=${1:-3}" INVESTIGATION_TEAM=a,b INVESTIGATION_SEED=7 \
    INVESTIGATION_MERGER=c INVESTIGATION_MERGER_TASK=M INVESTIGATION_REVIEWER=d INVESTIGATION_REVIEWER_TASK=R \
    INVESTIGATION_PR=https://github.com/example/repo/pull/81 PR_URL=https://github.com/example/repo/pull/81 \
    PR_NUMBER=81 BRANCH=agents/c/1 > "$AGENTS_HOME/tasks/P/meta.env"
  printf 'the task\n' > "$AGENTS_HOME/tasks/P/task.md"
  printf '%s\n' ID=M AGENT=c KIND=investigation STATUS=done RUN=1 TEAM= PARENT_TASK=P BRANCH=agents/c/1 BASE=main \
    > "$AGENTS_HOME/tasks/M/meta.env"
  printf 'merge the results\n' > "$AGENTS_HOME/tasks/M/task.md"
  printf '%s\n' ID=R AGENT=d KIND=investigation STATUS=done RUN=1 TEAM= PARENT_TASK=P REVIEW=1 \
    > "$AGENTS_HOME/tasks/R/meta.env"
  printf '## Findings\n\n- the parser drops a feature with no items\n\nVERDICT: needs-fixes\n' > "$AGENTS_HOME/tasks/R/final.md"
}

load_agent() {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  unset INVESTIGATORS_TYPE
  # shellcheck disable=SC2034 # read by the sourced bin/agent
  declare -gA INVESTIGATORS_TYPE=([a]=claude [b]=claude [c]=claude [d]=claude [e]=claude)
  start_runner() { printf '%s\n' "$*" >> "$HOME/started"; }
  dispatch() { :; }
}

@test "a review asking for changes sends the merger back on its branch with the findings: the same PR, no other" {
  inv
  load_agent
  investigation_pipeline_tick P
  [ "$(meta P INVESTIGATION_PHASE)" = 2 ]
  [ "$(meta P INVESTIGATION_FIX_PASSES)" = 1 ]
  [ "$(meta P INVESTIGATION_MERGER_TASK)" = M ]
  [ "$(meta P INVESTIGATION_PR)" = https://github.com/example/repo/pull/81 ]
  grep -qx 'M resume' "$HOME/started"
  grep -q 'Review findings to fix' "$AGENTS_HOME/tasks/M/task.md"
  grep -q 'the parser drops a feature with no items' "$AGENTS_HOME/tasks/M/task.md"
  grep -q 'Do not open another pull request' "$AGENTS_HOME/tasks/M/task.md"
  [ "$(meta P INVESTIGATION_PAST_REVIEWERS)" = d ]
  [ -z "$(meta P INVESTIGATION_REVIEWER_TASK)" ]
}

@test "an incomplete review is a pass to fix as well" {
  inv
  sed -i 's/needs-fixes/incomplete/' "$AGENTS_HOME/tasks/R/final.md"
  load_agent
  investigation_pipeline_tick P
  [ "$(meta P INVESTIGATION_FIX_PASSES)" = 1 ]
  grep -qx 'M resume' "$HOME/started"
}

@test "after VERIFY_MAX_FIX_PASSES passes a review asking for changes goes to the human" {
  inv
  printf 'INVESTIGATION_FIX_PASSES=2\n' >> "$AGENTS_HOME/tasks/P/meta.env"
  load_agent
  # shellcheck disable=SC2034 # read by the sourced bin/agent
  VERIFY_MAX_FIX_PASSES=2
  investigation_pipeline_tick P
  [ "$(meta P INVESTIGATION_PHASE)" = 4 ]
  [ ! -e "$HOME/started" ]
  [[ $(meta P INVESTIGATION_HANDOFF_STAGE) == *"after 2 fix passes"* ]]
}

@test "a ready review ends the loop as before: the PR waits for the human's merge" {
  inv
  sed -i 's/needs-fixes/ready-to-merge/' "$AGENTS_HOME/tasks/R/final.md"
  sed -i 's/^- the parser.*//' "$AGENTS_HOME/tasks/R/final.md"
  load_agent
  investigation_pipeline_tick P
  [ "$(meta P INVESTIGATION_PHASE)" = 4 ]
  [ ! -e "$HOME/started" ]
}

@test "once its PR exists a merger that fails is not replaced: no other PR, the human decides" {
  inv 2
  sed -i 's/^STATUS=done/STATUS=failed/' "$AGENTS_HOME/tasks/M/meta.env"
  load_agent
  investigation_pipeline_tick P
  [ "$(meta P INVESTIGATION_PHASE)" = 4 ]
  [ "$(meta P INVESTIGATION_MERGER_TASK)" = M ]
  [ -z "$(meta P INVESTIGATION_MERGER_REPLACEMENTS)" ]
  [ ! -e "$HOME/started" ]
}

@test "each pass is reviewed by a new outsider; with every outsider used, the one whose review is the oldest" {
  inv
  load_agent
  meta_set P INVESTIGATION_REVIEWER ""
  investigation_phase_pick P 3
  [[ $INVESTIGATION_PICK == d || $INVESTIGATION_PICK == e ]]
  meta_set P INVESTIGATION_PAST_REVIEWERS d
  investigation_phase_pick P 3
  [ "$INVESTIGATION_PICK" = e ]
  meta_set P INVESTIGATION_PAST_REVIEWERS d,e
  investigation_phase_pick P 3
  [ "$INVESTIGATION_PICK" = d ]
  meta_set P INVESTIGATION_PAST_REVIEWERS e,d
  investigation_phase_pick P 3
  [ "$INVESTIGATION_PICK" = e ]
}

@test "the prompts: a reviewer commits nothing and opens no PR; a merger sent back pushes to its branch" {
  grep -q 'Do not commit, push or open a pull request' "$REPO_ROOT/prompts/inv-review-task.md"
  grep -q 'never open another pull request' "$REPO_ROOT/prompts/inv-merge.md"
}
