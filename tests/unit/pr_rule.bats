#!/usr/bin/env bats
# The PR rule is one fragment, prompts/pr-ownership.md: the script renders it into a solo task's prompt, into a team's
# coordinator's role and into its member's role, and into no other prompt. It names this task's PR when the script
# knows it (PR_NUMBER, else the ROUND_PR a resumed round carries); otherwise it tells the coordinator to open one draft
# PR. The agents' gh refuses gh pr create while the task's PR is open, and names that PR.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

# rule_task [key=value…] — a task P whose prompt can be rendered without launching an agent: a solo task, unless the
# pairs say otherwise (KIND, TEAM, PR_NUMBER, ROUND_PR, …)
rule_task() {
  local kv
  unset PR_NUMBER ROUND_PR TEAM   # an earlier call's, loaded into this shell: task_field would see them
  mkdir -p "$TASKS/P"
  printf 'ID=P\nAGENT=solo\nTYPE=claude\nKIND=worker\nREPO_NAME=repo\nREPO_ROOT=%s\nWORKTREE=%s\nBRANCH=agents/solo/1\nBASE=main\nRUN=1\n' \
    "$HOME/repo" "$HOME/repo" > "$TASKS/P/meta.env"
  printf 'Fix the cache.\n' > "$TASKS/P/task.md"
  for kv in "$@"; do meta_set P "${kv%%=*}" "${kv#*=}"; done
  meta_load P
}

# prompt mode — P's prompt of that mode (new, resume, fresh)
prompt() {
  FRESH_WHY='context limit' build_prompt P "$1" "$TASKS/P/questions-1.md" 1
}

# between text from to — the lines of text from the line matching from up to the one matching to
between() { sed -n "/$2/,/$3/p" <<< "$1"; }

# the coordinator's part of the rule, and the member's (a sentence only each of them has)
COORDINATOR_PART='head branch is not yours'
MEMBER_PART="and the team's task has one, the coordinator's"

@test "a solo task's prompt has the PR rule once, with the coordinator's part, and tells it to open one draft PR" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  rule_task
  out=$(prompt new)
  [ "$(grep -c '^## Pull request ownership' <<< "$out")" -eq 1 ]
  [ "$(grep -c 'Only the coordinator may open a PR' <<< "$out")" -eq 1 ]
  [[ $out == *"$COORDINATOR_PART"* ]]
  [[ $out != *"$MEMBER_PART"* ]]
  [[ $out == *'open **one draft PR**'* ]]
  [[ $out == *'gh pr create --draft --base main --head agents/solo/1'* ]]
  [[ $out != *"This task's PR is"* ]]
  [[ $out != *'{{'* ]]
}

@test "a solo task whose PR the script knows is told its number and branch and to continue it" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  rule_task PR_NUMBER=61
  out=$(prompt new)
  [[ $out == *"**This task's PR is #61 (branch \`agents/solo/1\`).**"* ]]
  [[ $out == *'gh pr edit 61'* ]]
  [[ $out != *'No PR of this task is known yet'* ]]
  [ "$(grep -c '^## Pull request ownership' <<< "$out")" -eq 1 ]
  [[ $out != *'{{'* ]]
}

@test "a resumed round's task names the ROUND_PR it carries, with the branch of the task that opened it" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/OLD"
  printf 'ID=OLD\nAGENT=lead\nKIND=worker\nREPO_NAME=repo\nBRANCH=agents/lead/1\nPR_NUMBER=58\n' > "$TASKS/OLD/meta.env"
  rule_task ROUND_PR=58
  out=$(prompt new)
  [[ $out == *"**This task's PR is #58 (branch \`agents/lead/1\`).**"* ]]
  rm -r "$TASKS/OLD"   # archived: the number is all the script knows; a review of that PR has another branch
  mkdir -p "$TASKS/REV"
  printf 'ID=REV\nAGENT=inv\nKIND=investigation\nREVIEW=1\nREPO_NAME=repo\nBRANCH=agents/inv/1\nPR_NUMBER=58\n' > "$TASKS/REV/meta.env"
  out=$(prompt new)
  [[ $out == *"**This task's PR is #58.**"* ]]
}

@test "a task's own PR_NUMBER comes before the ROUND_PR it carries" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  rule_task PR_NUMBER=61 ROUND_PR=58
  out=$(prompt new)
  [[ $out == *"This task's PR is #61"* ]]
  [[ $out != *'#58'* ]]
}

@test "a team's prompt has the coordinator's part in the coordinator's role and the member's part in the member's role" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  rule_task TEAM=T-team
  export TEAM=T-team
  out=$(prompt new)
  coord=$(between "$out" "^## 3\. The coordinator's role" "^## 4\. ")
  member=$(between "$out" "^## 4\. The member's role" "^## 5\. ")
  [[ $coord == *"$COORDINATOR_PART"* ]]
  [[ $coord == *'gh pr create --draft --base main --head agents/solo/1'* ]]
  [[ $coord != *"$MEMBER_PART"* ]]
  [[ $member == *"$MEMBER_PART"* ]]
  [[ $member != *"$COORDINATOR_PART"* ]]
  [[ $member != *'gh pr create --draft'* ]]
  [ "$(grep -c "$COORDINATOR_PART" <<< "$out")" -eq 1 ]
  [ "$(grep -c "$MEMBER_PART" <<< "$out")" -eq 1 ]
  [[ $out != *'{{'* ]]
}

@test "a team member of a resumed round is told the round's PR in its role" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  rule_task TEAM=T-team ROUND_PR=58
  export TEAM=T-team
  out=$(prompt new)
  member=$(between "$out" "^## 4\. The member's role" "^## 5\. ")
  [[ $member == *"**This task's PR is #58.**"* ]]
  coord=$(between "$out" "^## 3\. The coordinator's role" "^## 4\. ")
  [[ $coord == *"**This task's PR is #58.**"* ]]
  [[ $coord == *'gh pr edit 58'* ]]
}

@test "helpers, requested investigators, reviews and independent investigations get no PR rule, in no mode" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  rule_task PR_NUMBER=61
  local role mode out
  export KIND REVIEW INVESTIGATION_TEAM INVESTIGATION_PHASE
  for role in helper investigation review independent merger independent-review; do
    KIND=investigation REVIEW='' INVESTIGATION_TEAM='' INVESTIGATION_PHASE=''
    case $role in
      helper) KIND=helper ;;
      review) REVIEW=1 ;;
      independent) INVESTIGATION_TEAM=I INVESTIGATION_PHASE=1 ;;
      merger) INVESTIGATION_TEAM=I INVESTIGATION_PHASE=2 ;;
      independent-review) INVESTIGATION_TEAM=I INVESTIGATION_PHASE=3 ;;
    esac
    meta_set P INVESTIGATION_TEAM "$INVESTIGATION_TEAM"
    for mode in new resume fresh; do
      out=$(prompt "$mode")
      [[ $out != *'Pull request ownership'* ]] || { echo "$role $mode: the rule's heading"; return 1; }
      [[ $out != *'Only the coordinator may open a PR'* ]] || { echo "$role $mode: the rule"; return 1; }
      [[ $out != *"This task's PR is"* ]] || { echo "$role $mode: this task's PR"; return 1; }
      [[ $out != *'Never open a new PR'* ]] || { echo "$role $mode: the old sentence"; return 1; }
    done
  done
}

@test "no prompt file repeats the PR rule: it is in the fragment only" {
  local f
  for f in "$REPO_ROOT"/prompts/*.md; do
    [[ $f == */pr-ownership.md ]] && continue
    run ! grep -q 'Never open a new PR' "$f"
    run ! grep -q 'Only the coordinator may open a PR' "$f"
  done
}

@test "a resumed session repeats the rule of its role: a solo task's and a coordinator's, or a member's" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  rule_task PR_NUMBER=61
  out=$(prompt resume)
  [ "$(grep -c '^## Pull request ownership' <<< "$out")" -eq 1 ]
  [[ $out == *"$COORDINATOR_PART"* && $out == *"This task's PR is #61"* ]]
  rule_task TEAM=T-team ROUND_PR=58
  export TEAM=T-team
  team_lead() { echo solo; }   # P's agent coordinates the team
  out=$(prompt resume)
  [[ $out == *"$COORDINATOR_PART"* && $out != *"$MEMBER_PART"* && $out == *"This task's PR is #58"* ]]
  team_lead() { echo lead; }   # another agent coordinates it
  out=$(prompt resume)
  [[ $out == *"$MEMBER_PART"* && $out != *"$COORDINATOR_PART"* && $out == *"This task's PR is #58"* ]]
  team_lead() { :; }   # nobody yet: a member's part, which opens no PR
  out=$(prompt resume)
  [[ $out == *"$MEMBER_PART"* && $out != *"$COORDINATOR_PART"* ]]
}

@test "the runner gives the agents' gh its task's meta file, and nothing without a task" {
  mkdir -p "$AGENTS_HOME/tasks/S1"
  printf 'ID=S1\nAGENT=solo\nKIND=worker\nREPO_ROOT=%s\n' "$HOME/repo" > "$AGENTS_HOME/tasks/S1/meta.env"
  env_of() { (source "$REPO_ROOT/bin/agent"; RUNNER_ID=$1 REPO_ROOT=$HOME/repo agents_gh_env > /dev/null
              printf '%s' "${AGENT_GH_TASK_META-unset}"); }
  [ "$(env_of S1)" = "$AGENTS_HOME/tasks/S1/meta.env" ]
  [ -z "$(env_of '')" ]
}

# gh_task [key=value…] — the agents' gh ($GH) for a task whose meta file has these pairs, with the stub as the real gh
gh_task() {
  mkdir -p "$BATS_TEST_TMPDIR/ghw" "$AGENTS_HOME/logs" "$AGENTS_HOME/tasks/S1"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"   # agent run as gh, as the runner links it
  GH=$BATS_TEST_TMPDIR/ghw/gh
  printf 'ID=S1\nAGENT=solo\nKIND=worker\nBRANCH=agents/solo/1\n' > "$AGENTS_HOME/tasks/S1/meta.env"
  printf '%s\n' "$@" >> "$AGENTS_HOME/tasks/S1/meta.env"
  export AGENT_REAL_GH=$REPO_ROOT/tests/unit/stubs/gh AGENT_GH_TASK_META=$AGENTS_HOME/tasks/S1/meta.env
  export AGENT_GITHUB_TRUSTED_RESOLVED="mateusz-szczyrzyca ms-ai-agents"
  log=$AGENTS_HOME/logs/agent.jsonl
  : > "$STUB_HOME/gh.log"
}

# pr_state number state branch — what GitHub says of that PR (gh pr view --json, the stub's gh-pr-view.json)
pr_state() {
  printf '{"number": %s, "state": "%s", "headRefName": "%s", "url": "https://github.com/example/repo/pull/%s"}\n' \
    "$1" "$2" "$3" "$1" > "$STUB_HOME/gh-pr-view.json"
}

@test "gh pr create of a task whose PR is open is refused, names that PR, is logged, and never reaches gh" {
  gh_task PR_NUMBER=61
  pr_state 61 OPEN agents/solo/1
  run ! "$GH" pr create --draft --base main --head agents/solo/2 --title t --body b
  [[ $output == *"#61 (branch agents/solo/1)"* ]]
  [[ $output == *"https://github.com/example/repo/pull/61"* ]]
  [[ $output == *"gh pr edit 61"* ]]
  run ! grep -q 'pr create' "$STUB_HOME/gh.log"
  grep -q 'gh pr view 61' "$STUB_HOME/gh.log"
  [ "$(jq -r 'select(.event == "github_task_pr_refused") | "\(.pr) \(.branch) \(.action)"' "$log")" = "61 agents/solo/1 create" ]
}

@test "gh pr create of a task that carries its round's open PR (ROUND_PR) is refused, naming it" {
  gh_task ROUND_PR=58
  pr_state 58 OPEN agents/lead/1
  run ! "$GH" pr create --base main --head agents/solo/1 --title t --body b
  [[ $output == *"#58 (branch agents/lead/1)"* ]]
  run ! grep -q 'pr create' "$STUB_HOME/gh.log"
}

@test "a PR creation through gh api of a task whose PR is open is refused too" {
  gh_task PR_NUMBER=61
  pr_state 61 OPEN agents/solo/1
  run ! "$GH" api repos/example/repo/pulls --method POST -f head=agents/solo/2 -f base=main -f title=t
  [[ $output == *"#61 (branch agents/solo/1)"* ]]
  run ! grep -q 'gh api repos/example/repo/pulls' "$STUB_HOME/gh.log"
  [ "$(jq -r 'select(.event == "github_task_pr_refused") | .action' "$log")" = api ]
  "$GH" api repos/example/repo/pulls/61 --method GET   # reading it goes on
  grep -q 'gh api repos/example/repo/pulls/61 --method GET' "$STUB_HOME/gh.log"
}

@test "a task whose PR is merged or closed may open a new one; so may a task without a PR" {
  local state
  for state in MERGED CLOSED; do
    gh_task PR_NUMBER=61
    pr_state 61 "$state" agents/solo/1
    run "$GH" pr create --base main --head agents/solo/2 --title t --body b
    [ "$status" -eq 0 ]
    grep -q 'gh pr create --base main --head agents/solo/2' "$STUB_HOME/gh.log"
  done
  gh_task
  rm -f "$STUB_HOME/gh-pr-view.json"
  run "$GH" pr create --base main --head agents/solo/2 --title t --body b
  [ "$status" -eq 0 ]
  grep -q 'gh pr create --base main --head agents/solo/2' "$STUB_HOME/gh.log"
  run ! grep -q 'gh pr view' "$STUB_HOME/gh.log"
}

@test "when the task's PR cannot be read, gh pr create goes on to gh, and that is logged" {
  gh_task PR_NUMBER=61
  rm -f "$STUB_HOME/gh-pr-view.json"   # the stub answers nothing: no state to read
  run "$GH" pr create --base main --head agents/solo/2 --title t --body b
  [ "$status" -eq 0 ]
  grep -q 'gh pr create --base main --head agents/solo/2' "$STUB_HOME/gh.log"
  [ "$(jq -r 'select(.event == "github_task_pr_unknown") | .pr' "$log")" = 61 ]
}

@test "a team member's gh pr create is refused as a member's, before its task's PR is looked up" {
  gh_task ROUND_PR=58
  pr_state 58 OPEN agents/lead/1
  AGENT_GH_COORDINATOR=lead run ! "$GH" pr create --base main --head agents/member/1 --title t --body b
  [[ $output == *"coordinator (lead)"* ]]
  run ! grep -q 'gh pr' "$STUB_HOME/gh.log"
}
