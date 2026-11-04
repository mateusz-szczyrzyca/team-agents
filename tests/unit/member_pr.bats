#!/usr/bin/env bats
# A team's round has one PR, opened by its coordinator. A member (claude-haiku, again and again) opened its own PR
# beside the round's: the agents' gh now refuses a team member's gh pr create, and tells it what to do instead — push
# its branch and tell its coordinator, which merges the member's work into the round's PR.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

setup_gh() {
  mkdir -p "$BATS_TEST_TMPDIR/ghw"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"   # agent run as gh, as the runner links it
  GH=$BATS_TEST_TMPDIR/ghw/gh
  export AGENT_REAL_GH=$REPO_ROOT/tests/unit/stubs/gh
  export AGENT_GITHUB_TRUSTED_RESOLVED="mateusz-szczyrzyca ms-ai-agents"
  log=$AGENTS_HOME/logs/agent.jsonl
  mkdir -p "$AGENTS_HOME/logs"
}

@test "a team member's gh pr create is refused, names its coordinator, is logged, and never reaches gh" {
  setup_gh
  : > "$STUB_HOME/gh.log"
  AGENT_GH_COORDINATOR=gemini-1 run ! "$GH" pr create --base main --head agents/claude-haiku/1 --title t --body b
  [[ $output == *"coordinator (gemini-1)"* && $output == *"Push your branch"* ]]
  run ! grep -q 'pr create' "$STUB_HOME/gh.log"
  grep -q '"event":"github_member_pr_refused"' "$log"
  grep -q '"coordinator":"gemini-1"' "$log"
}

@test "anyone else's gh pr create goes to gh as before" {
  setup_gh
  : > "$STUB_HOME/gh.log"
  AGENT_GH_COORDINATOR="" run "$GH" pr create --base main --head agents/gemini-1/1 --title t --body b
  [[ $output != *"does not open a PR"* ]]
  grep -q 'gh pr create --base main --head agents/gemini-1/1' "$STUB_HOME/gh.log"
}

@test "gh_coordinator: a team member's coordinator; nobody for the coordinator, a solo task or an investigator" {
  for x in "M1 member T-team worker" "L1 lead T-team worker" "S1 solo - worker" "I1 inv T-team investigation"; do
    read -r id agent team kind <<< "$x"
    mkdir -p "$AGENTS_HOME/tasks/$id"
    printf 'ID=%s\nAGENT=%s\nKIND=%s\n' "$id" "$agent" "$kind" > "$AGENTS_HOME/tasks/$id/meta.env"
    [[ $team == - ]] || printf 'TEAM=%s\n' "$team" >> "$AGENTS_HOME/tasks/$id/meta.env"
  done
  out() { (source "$REPO_ROOT/bin/agent"; team_lead() { echo lead; }; gh_coordinator "$1"); }
  [ "$(out M1)" = lead ]
  [ -z "$(out L1)" ]
  [ -z "$(out S1)" ]
  [ -z "$(out I1)" ]
}

@test "every mode's prompt states the rule never to open a new PR when not coordinator or when a Draft or Open PR already exists" {
  local f
  for f in protocol.md team.md helper.md investigator.md inv-work.md inv-merge.md review.md; do
    grep -qi 'never open a new PR' "$REPO_ROOT/prompts/$f"
    grep -q 'Draft' "$REPO_ROOT/prompts/$f"
    grep -q 'Open' "$REPO_ROOT/prompts/$f"
    grep -qi 'continue in the mentioned PR' "$REPO_ROOT/prompts/$f"
  done
}

