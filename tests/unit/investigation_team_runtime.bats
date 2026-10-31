#!/usr/bin/env bats
# Phase one uses its own CLI fixture because independent investigators have no agentbus configuration.
load helpers

# independent_setup configures two investigators and a bus-free CLI only in this test's temporary home.
independent_setup() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude)/' "$AGENTS_HOME/agents.conf"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/claude"
  printf '%s\n' 'Independent topic' > "$HOME/topic.md"
}

# field decodes the shell-quoted metadata written by meta_set in this isolated test home.
field() { bash -c 'source "$1"; printf "%s" "${!2:-}"' _ "$AGENTS_HOME/tasks/$1/meta.env" "$2"; }

# launch_phase_one returns the newly allocated orchestration task through PHASE_ONE.
launch_phase_one() {
  run agent _investigation-team --team 'inv, inv2' -f "$HOME/topic.md"
  [ "$status" -eq 0 ]
  PHASE_ONE=$(task_of investigation-team)
  [ -n "$PHASE_ONE" ]
}

# path_absent rejects a complete path or a file beneath it, allowing a sibling with a longer name.
path_absent() {
  python3 - "$1" "$2" <<'MATCH_PATH'
import re
import sys
sys.exit(1 if re.search(re.escape(sys.argv[2]) + r"(?![\w.-])", sys.argv[1]) else 0)
MATCH_PATH
}

# recorded_status checks the immutable terminal snapshot of one phase-one member.
recorded_status() {
  jq -r --arg a "$2" '.members[] | select(.agent == $a) | .status' "$AGENTS_HOME/tasks/$1/phase1.json"
}

@test "phase one launches the whole cohort concurrently from one frozen commit" {
  independent_setup
  local base parent child agent wt branch
  base=$(git rev-parse HEAD)
  scenario '{"inv":{"new":"barrier"},"inv2":{"new":"barrier"}}'
  launch_phase_one
  parent=$PHASE_ONE
  wait_status "$parent" waiting
  [ "$(field "$parent" INVESTIGATION_BASE_SHA)" = "$base" ]
  [ "$(field "$parent" INVESTIGATION_TEAM)" = 'inv,inv2' ]
  for agent in inv inv2; do
    child=$(task_of "$agent")
    wt=$(field "$child" WORKTREE); branch=$(field "$child" BRANCH)
    [ "$(field "$child" KIND)" = investigation ]
    [ "$(field "$child" INVESTIGATION_TEAM)" = "$parent" ]
    [ "$(field "$child" PARENT_TASK)" = "$parent" ]
    [ -z "$(field "$child" TEAM)" ]
    [ "$(field "$child" BASE)" = "$base" ]
    [[ $branch == "agents/$agent/"* ]]
    [ "$(git -C "$wt" rev-parse HEAD^)" = "$base" ]
    [ "$(recorded_status "$parent" "$agent")" = 'done' ]
    [ "$(git ls-remote origin "refs/heads/$branch" | cut -f1)" = "$(git -C "$wt" rev-parse HEAD)" ]
  done
  [ "$(git rev-parse HEAD)" = "$base" ]
}

@test "independent prompts and MCP configurations reveal no peer worktree or bus tools" {
  independent_setup
  launch_phase_one
  wait_status "$PHASE_ONE" waiting
  local a b child peer cfg prompt
  for a in inv inv2; do
    b=inv; [ "$a" = inv ] && b=inv2
    child=$(task_of "$a"); peer=$(task_of "$b")
    cfg="$AGENTS_HOME/tasks/$child/mcp.json"
    [ "$(jq '.mcpServers | length' "$cfg")" -eq 0 ]
    prompt=$(cat "$HOME/independent-$a.prompt")
    [[ $prompt == *"$(field "$child" BRANCH)"* ]]
    path_absent "$prompt" "$(field "$peer" WORKTREE)"
    [[ $prompt != *"$(field "$peer" BRANCH)"* ]]
    [[ $prompt != *'read_messages'* && $prompt != *'claim_lead'* ]]
    [[ $prompt != *'{{'* ]]
  done
}

@test "one failed investigator does not cancel a published independent result" {
  independent_setup
  scenario '{"inv":{"new":"no-status"},"inv2":{"new":"publish"}}'
  launch_phase_one
  wait_status "$PHASE_ONE" waiting
  [ "$(recorded_status "$PHASE_ONE" inv)" = failed ]
  [ "$(recorded_status "$PHASE_ONE" inv2)" = 'done' ]
  [ "$(field "$PHASE_ONE" AUTO_STAGE)" = 'waits for you: phase 1 complete (merge not available yet)' ]
}

@test "unpublished commits and bare DONE are failures and notify the human without a PR" {
  independent_setup
  scenario '{"inv":{"new":"unpublished"},"inv2":{"new":"empty"}}'
  launch_phase_one
  wait_status "$PHASE_ONE" waiting
  [ "$(recorded_status "$PHASE_ONE" inv)" = failed ]
  [ "$(recorded_status "$PHASE_ONE" inv2)" = failed ]
  [ "$(field "$PHASE_ONE" AUTO_STAGE)" = 'waits for you: all phase 1 runs failed' ]
  wait_grep "$PHASE_ONE" "$HOME/notifications.log"
  [ -z "$(field "$PHASE_ONE" PR_URL)" ]
  [ -z "$(git ls-remote origin 'refs/heads/agents/*')" ]
}

@test "a failed cohort releases its capacity lease after every run ends" {
  independent_setup
  scenario '{"inv":{"new":"fail"},"inv2":{"new":"no-status"}}'
  launch_phase_one
  wait_status "$PHASE_ONE" waiting
  [ "$(field "$PHASE_ONE" INVESTIGATION_LEASE)" != 1 ]
  [ "$(recorded_status "$PHASE_ONE" inv)" = failed ]
  [ "$(recorded_status "$PHASE_ONE" inv2)" = failed ]
}

@test "a cohort holds all investigator leases until its slowest run ends" {
  independent_setup
  scenario '{"inv":{"new":"publish"},"inv2":{"new":"hold"}}'
  launch_phase_one
  local first=$PHASE_ONE
  wait_status "$(task_of inv)" 'done'
  [[ $(field "$first" AUTO_STAGE) == 'phase 1:'* ]]
  launch_phase_one
  [ "$(field "$PHASE_ONE" AUTO_STAGE)" = 'waiting: capacity queue' ]
  [ "$(task_of inv)" = "$(field "$first" INVESTIGATION_PHASE1_TASKS | cut -d, -f1)" ]
  touch "$HOME/release"
  wait_status "$first" waiting
  wait_status "$PHASE_ONE" waiting
}

@test "quota-paused investigators queue the whole cohort without partial admission" {
  independent_setup
  mkdir -p "$AGENTS_HOME/tasks/old-inv"
  printf 'AGENT=inv\nKIND=investigation\nSTATUS=failed\nQUOTA_UNTIL=%s\n' "$(( $(date +%s) + 3600 ))" > "$AGENTS_HOME/tasks/old-inv/meta.env"
  launch_phase_one
  [ "$(field "$PHASE_ONE" AUTO_STAGE)" = 'waiting: capacity queue' ]
  [ -z "$(field "$PHASE_ONE" INVESTIGATION_LEASE)" ]
  [ -z "$(git branch --list 'agents/*')" ]
}

@test "an existing review pair waits for investigators held by a phase-one lease" {
  independent_setup
  scenario '{"inv":{"new":"publish"},"inv2":{"new":"hold"}}'
  launch_phase_one
  local parent=$PHASE_ONE
  wait_status "$(task_of inv)" 'done'
  source "$REPO_ROOT/bin/agent"
  run start_review_pair "$parent"
  [ "$status" -eq 1 ]
  [ -z "$(free_investigator_list)" ]
  run free_investigator
  [ "$status" -eq 1 ]
  [ ! -d "$AGENTS_HOME/reviews" ]
  touch "$HOME/release"
  wait_status "$parent" waiting
}
