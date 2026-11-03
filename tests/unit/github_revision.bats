#!/usr/bin/env bats
# Regression coverage for GitHub identities, default reviewer rotation and prompt routing.

load helpers

# revision_task loads a minimal task without starting an agent or a systemd unit.
revision_task() {
  mkdir -p "$TASKS/X"
  cat > "$TASKS/X/meta.env" <<META
ID=X
AGENT=solo
TYPE=claude
MODEL=haiku
KIND=worker
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
BRANCH=main
BASE=main
SESSION_ID=test-session
RUN=1
ROUND=0
STATUS=running
PR_NUMBER=7
PR_URL=https://github.com/o/r/pull/7
HELPER_ID=1
INV_ID=66
META
  echo 'Inspect the parser' > "$TASKS/X/task.md"
  meta_load X
}

@test "default review selection rotates the same PR across processes and exhausts all six pairs" {
  local n pairs=() pair
  for n in 1 2 3 4 5 6; do
    pair=$(source "$REPO_ROOT/bin/agent"
      mkdir -p "$TASKS/T$n"
      printf 'ID=T%s\nREPO_NAME=repo\nPR_NUMBER=7\nPR_URL=https://github.com/o/r/pull/7\n' "$n" > "$TASKS/T$n/meta.env"
      free_investigator_list() { printf '%s\n' inv-a inv-b inv-c inv-d; }
      investigator_names() { free_investigator_list; }
      start_review_slot() { echo "$4" > "$2/$3.agent"; echo 1 > "$2/$3.inv"; }
      act() { :; }
      auto_stage() { :; }
      start_review_pair "T$n" > /dev/null
      printf '%s %s' "$(cat "$AGENTS_HOME/reviews/repo-7/A.agent")" "$(cat "$AGENTS_HOME/reviews/repo-7/B.agent")")
    pairs+=("$pair")
  done
  [ "${pairs[0]}" = 'inv-a inv-b' ]
  [ "${pairs[1]}" = 'inv-c inv-d' ]
  [ "${pairs[2]}" = 'inv-a inv-c' ]
  [ "$(printf '%s\n' "${pairs[@]}" | sort -u | wc -l)" -eq 6 ]
  grep -qx 'AGENT_REVIEW_DRAW=rotate' "$REPO_ROOT/config/agents.conf"
}

@test "GitHub identities are bold distinct and stable across model and roster changes" {
  source "$REPO_ROOT/bin/agent"
  revision_task
  local a first colors=() description
  declare -A AGENT_TYPE AGENT_MODEL
  export AGENT_TYPE AGENT_MODEL
  for a in gemini-1 claude-haiku codex-1 inv-astra inv-sol; do
    AGENT_TYPE[$a]=codex
    AGENT_MODEL[$a]=gpt-6-astra
    description=$(agent_desc "$a")
    [[ $description == *'\textbf{'"$a"'}'* ]]
    colors+=("$(sed -n 's/.*\\color{\([^}]*\)}.*/\1/p' <<< "$description")")
  done
  [ "$(printf '%s\n' "${colors[@]}" | sort -u | wc -l)" -eq 5 ]
  first=$(agent_desc inv-astra)
  AGENT_TYPE[another]=claude
  export ROUND=42
  [ "$(agent_desc inv-astra)" = "$first" ]
  AGENT_MODEL[inv-astra]=different
  [ "$(agent_desc inv-astra | cut -d '$' -f2)" = "$(cut -d '$' -f2 <<< "$first")" ]
  [[ $(agent_desc inv_test) == *'inv\_test'* ]]
}

@test "runner GitHub byline and review prompt use the same colored identity" {
  source "$REPO_ROOT/bin/agent"
  revision_task
  meta_set X KIND investigation
  meta_set X REVIEW 1
  meta_set X INV_ID 66
  meta_load X
  export RUNNER_ID=X
  agents_gh_env
  [[ $AGENT_REVIEW_BYLINE == *'\textbf{solo}'* ]]
  local prompt
  prompt=$(build_prompt X new "$TASKS/X/questions-1.md" 1)
  [[ $prompt == *"$AGENT_REVIEW_BYLINE"* ]]
}

@test "every publishing route receives file permalinks and vendor attribution rules once" {
  source "$REPO_ROOT/bin/agent"
  revision_task
  local role phase prompt
  for role in worker helper investigation; do
    KIND=$role
    REVIEW=1
    for phase in '' 1 2 3 5; do
      INVESTIGATION_TEAM=$phase
      INVESTIGATION_PHASE=${phase:-1}
      prompt=$(build_prompt X new "$TASKS/X/questions-1.md" 1)
      [ "$(grep -c '^## GitHub publication' <<< "$prompt")" -eq 1 ]
      [[ $prompt == *'/blob/<full-commit-SHA>/'* ]]
      [[ $prompt == *'deleted files'* && $prompt == *'URL-encode'* ]]
      [[ $prompt == *'Generated with Claude Code'* ]]
      [[ $prompt != *'{{'* ]]
    done
  done
}

@test "review recommendation leads the body with the notification icons and colored bold verdicts" {
  source "$REPO_ROOT/bin/agent"
  revision_task
  export KIND=investigation REVIEW=1
  local prompt
  prompt=$(build_prompt X new "$TASKS/X/questions-1.md" 1)
  [[ $prompt == *'first section after the byline'* ]]
  [[ $prompt == *'✅ $\color{green}{\textbf{Ready to merge}}$'* ]]
  [[ $prompt == *'🔧 $\color{red}{\textbf{Needs changes}}$'* ]]
  [[ $prompt == *'VERDICT: ready-to-merge | needs-fixes | incomplete'* ]]
}

@test "independent fresh prompts include their base once and no unavailable notebook or reservation calls" {
  source "$REPO_ROOT/bin/agent"
  revision_task
  echo 'Repository instructions' > "$WORKTREE/AGENTS.md"
  export KIND=investigation INVESTIGATION_TEAM=P FRESH_WHY=timeout
  local phase prompt
  for phase in 1 2 3 5; do
    export INVESTIGATION_PHASE=$phase
    prompt=$(build_prompt X fresh "$TASKS/X/questions-2.md" 2)
    [ "$(grep -c '^# WORKING CONTEXT' <<< "$prompt")" -eq 1 ]
    [ "$(grep -c '^# TOPIC' <<< "$prompt")" -eq 1 ]
    [[ $prompt != *'notebook_append'* && $prompt != *'`reserve`/`release`'* ]]
    [[ $prompt != *'CLARIFICATION RULES point 6'* ]]
  done
}

@test "helper prompt does not expand the request to unrelated skipped tests" {
  source "$REPO_ROOT/bin/agent"
  revision_task
  export KIND=helper
  meta_set X KIND helper
  skips_in_repo() { echo 'elsewhere_test.go:10: skip'; }
  local prompt
  prompt=$(build_prompt X new "$TASKS/X/questions-1.md" 1)
  [[ $prompt != *'Fill every one'* ]]
  [[ $prompt == *'request_investigation'* && $prompt == *'checkpoint'* ]]
}

@test "review task retains independence and previous findings from the queued request" {
  source "$REPO_ROOT/bin/agent"
  revision_task
  bus() { :; }
  start_runner() { :; }
  start_investigation '{"id":66,"from_task":"X","from_agent":"solo","kind":"review","pr_number":7,"pr_url":"https://github.com/o/r/pull/7","question":"Independent re-review after fix pass 2. Previous finding: cache never expires."}' inv
  local tasks
  tasks=$(cat "$TASKS"/*inv66/task.md)
  [[ $tasks == *'Independent re-review after fix pass 2'* ]]
  [[ $tasks == *'cache never expires'* && $tasks == *'Inspect the parser'* ]]
}

@test "Claude disables vendor PR and commit attribution in new and resumed sessions" {
  source "$REPO_ROOT/bin/agent"
  revision_task
  cat > "$HOME/capture-cli" <<'CLI'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$HOME/claude-args"
printf '%s\n' '{"type":"result","result":"STATUS: DONE"}'
CLI
  chmod +x "$HOME/capture-cli"
  export CLAUDE_BIN="$HOME/capture-cli"
  CLI_SESSION=()
  export CLI_SESSION
  watch_run() { wait "$1"; }
  local mode settings
  for mode in new resume; do
    run_claude X "$mode" 1 "$TASKS/X/task.md" "$TASKS/X"
    settings=$(sed -n '/^--settings$/{n;p;}' "$HOME/claude-args")
    [ -n "$settings" ]
    jq -e '.attribution.pr == "" and .attribution.commit == ""' <<< "$settings"
  done
}

@test "rotation prefers models absent from the previous pair when member overlap ties" {
  source "$REPO_ROOT/bin/agent"
  declare -A INVESTIGATORS_TYPE=([a]=codex [b]=claude [c]=codex [d]=claude [e]=gemini [f]=codex)
  declare -A INVESTIGATORS_MODEL=([a]=astra [b]=opus [c]=astra [d]=opus [e]=flash [f]=sol)
  export INVESTIGATORS_TYPE INVESTIGATORS_MODEL
  mkdir -p "$AGENTS_HOME/reviews"
  echo 'a b' > "$AGENTS_HOME/reviews/history"
  [ "$(review_pair_pick strict a b c d e f)" = 'e f' ]
}

@test "remote helper jobs retain restricted helper scope and deliver only through the job" {
  source "$REPO_ROOT/bin/agent"
  local sha prompt
  sha=$(git -C "$HOME/repo" rev-parse HEAD)
  bus() { jq -n --arg root "$HOME/repo" --arg commit "$sha" '{root:$root,commit:$commit}'; }
  NEW_ID=$(exec 10>&-
    prepare_job_task remote-helper worker repo main "$sha" 'Report only; do not commit.' 1 '' '' true member@lead
    printf '%s' "$NEW_ID")
  meta_load "$NEW_ID"
  [ "$(task_field "$NEW_ID" KIND)" = helper ]
  [ "$(task_field "$NEW_ID" FROM_AGENT)" = member@lead ]
  prompt=$(build_prompt "$NEW_ID" new "$TASKS/$NEW_ID/questions-1.md" 1)
  [[ $prompt == *'Do not commit or push unless the request says so.'* ]]
  [[ $prompt == *'UNCLEAR:'* && $prompt != *'`send_message`'* ]]
  bus() { echo unexpected > "$HOME/helper-bus"; }
  notice() { echo unexpected > "$HOME/helper-notice"; }
  helper_finish "$NEW_ID" "done" 'Result only'
  [ ! -e "$HOME/helper-bus" ]
  [ ! -e "$HOME/helper-notice" ]
}
