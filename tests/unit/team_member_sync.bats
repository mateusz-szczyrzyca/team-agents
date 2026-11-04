#!/usr/bin/env bats
# Team synchronization and round-PR guards:
# - agent-gh refuses gh pr merge by any agent (only the human or --autopilot merges PRs).
# - agent-gh refuses a team member's gh pr ready, close, reopen and edit, and resolves the team's coordinator
#   dynamically from AGENT_GH_TASK when AGENT_GH_COORDINATOR was empty at run start (Run 1 before claim_lead).
# - run_claude sets ENABLE_TOOL_SEARCH=false so Claude Code loads agentbus MCP tools directly without ToolSearch.
# - round_pr_note instructs only the coordinator to merge member branches into the round's PR branch.
# - a non-coordinator member's STATUS: ROUND_DONE does not override the coordinator's round number.

load helpers
bats_require_minimum_version 1.5.0

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

setup_gh() {
  mkdir -p "$BATS_TEST_TMPDIR/ghw"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"
  GH=$BATS_TEST_TMPDIR/ghw/gh
  export AGENT_REAL_GH=$REPO_ROOT/tests/unit/stubs/gh
  export AGENT_GITHUB_TRUSTED_RESOLVED="mateusz-szczyrzyca ms-ai-agents"
  log=$AGENTS_HOME/logs/agent.jsonl
  mkdir -p "$AGENTS_HOME/logs"
}

@test "agent-gh refuses gh pr merge from any agent, logs github_pr_merge_refused, and allows AGENT_AUTOPILOT_MERGE=1" {
  setup_gh
  : > "$STUB_HOME/gh.log"
  run ! "$GH" pr merge 53 --squash
  [[ $output == *"agents do not merge pull requests"* ]]
  run ! grep -q 'pr merge' "$STUB_HOME/gh.log"
  grep -q '"event":"github_pr_merge_refused"' "$log"

  AGENT_AUTOPILOT_MERGE=1 run "$GH" pr merge 53 --merge
  [ "$status" -eq 0 ]
  grep -q 'gh pr merge 53 --merge' "$STUB_HOME/gh.log"
}

@test "agent-gh resolves the coordinator dynamically during Run 1 and refuses member gh pr ready, close, reopen and edit" {
  setup_gh
  : > "$STUB_HOME/gh.log"
  mkdir -p "$AGENTS_HOME/tasks/M1" "$AGENTS_HOME/tasks/L1"
  printf 'ID=M1\nAGENT=claude-haiku\nTEAM=T1-team\nKIND=worker\n' > "$AGENTS_HOME/tasks/M1/meta.env"
  printf 'ID=L1\nAGENT=gemini-1\nTEAM=T1-team\nKIND=worker\n' > "$AGENTS_HOME/tasks/L1/meta.env"
  jq -n '{
    id: "T1-team", repo: "repo", wake_cmd: "true", home: env.AGENTS_HOME, lead: "gemini-1",
    members: {
      "gemini-1": {agent: "gemini-1", task: "L1", branch: "agents/gemini-1/1", worktree: "/tmp/l", state: "active", tier: "normal", provider: "google"},
      "claude-haiku": {agent: "claude-haiku", task: "M1", branch: "agents/claude-haiku/1", worktree: "/tmp/m", state: "active", tier: "normal", provider: "anthropic"}
    }
  }' | "$HOME/.local/bin/agentbus" team-create --dir "$AGENTS_HOME/bus"

  # Even when AGENT_GH_COORDINATOR was empty at run start, AGENT_GH_TASK=M1 resolves coordinator gemini-1:
  AGENT_GH_COORDINATOR="" AGENT_GH_TASK=M1 run ! "$GH" pr create --base main --head agents/claude-haiku/1 --title t --body b
  [[ $output == *"coordinator (gemini-1)"* ]]
  for verb in ready close reopen edit; do
    AGENT_GH_COORDINATOR="" AGENT_GH_TASK=M1 run ! "$GH" pr "$verb" 53
    [[ $output == *"coordinator (gemini-1)"* ]]
  done
  run ! grep -q 'gh pr ' "$STUB_HOME/gh.log"

  # The coordinator itself (L1) is allowed:
  AGENT_GH_COORDINATOR="" AGENT_GH_TASK=L1 run "$GH" pr ready 53
  [ "$status" -eq 0 ]
  grep -q 'gh pr ready 53' "$STUB_HOME/gh.log"
}

@test "run_claude disables Claude Code ToolSearch in claude-settings.json so agentbus MCP tools load directly" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  [ "$(jq -r '.env.ENABLE_TOOL_SEARCH // empty' "$AGENTS_HOME/tasks/$t/claude-settings.json")" = "false" ]
}

@test "round_pr_note instructs only the coordinator to merge member branches onto the round's PR branch" {
  mkdir -p "$AGENTS_HOME/tasks/L1" "$AGENTS_HOME/tasks/M1"
  printf 'ID=L1\nAGENT=gemini-1\nBRANCH=agents/gemini-1/1\nPR_NUMBER=52\n' > "$AGENTS_HOME/tasks/L1/meta.env"
  printf 'ID=M1\nAGENT=claude-haiku\nBRANCH=agents/claude-haiku/1\n' > "$AGENTS_HOME/tasks/M1/meta.env"
  note=$(source "$REPO_ROOT/bin/agent"; round_pr_note L1 M1)
  [[ $note == *"This round's PR is #52 (branch agents/gemini-1/1)."* ]]
  [[ $note == *"a member commits and pushes only to its own branch"* ]]
  [[ $note == *"the coordinator merges the members' branches"* ]]
  [[ $note != *"a member — or a new coordinator — merges"* ]]
}

@test "a non-coordinator member's STATUS: ROUND_DONE does not override the coordinator's WORK_ROUND" {
  scenario '{"lead": {"new": "team-lead-round:4/7"}, "member": {"new": "round-done:5/7"}}'
  agent run --team lead,member "task"
  t=$(task_of lead); m=$(task_of member)
  wait_status "$t" 'done'
  wait_status "$m" 'done'
  [ "$(meta "$t" WORK_ROUND)" = 4 ]
  [ -z "$(meta "$m" WORK_ROUND)" ]
}
