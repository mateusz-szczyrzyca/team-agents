#!/usr/bin/env bats
# claude-haiku, a member of a team of three, went its own way again and again: it never reached the team's bus in
# about a quarter of its runs (Claude Code deferred the agentbus tools behind its tool search, and haiku called
# them in the shell instead of as tools), merged the round's PR itself, started the next round alone, opened its
# own PRs, and pushed to its coordinator's branch as the round's text told it. Claude Code now gets the agentbus
# tools up front; the agents' gh refuses a member's merge, ready and close as well as its create, and asks the bus
# who the coordinator is when the command runs; a round's text tells a member to push its own branch, keeps only
# its newest heading, and says when the round's PR is merged already.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# setup_gh — the agents' gh (this script run as gh) in front of the stub gh, as the runner sets it up
setup_gh() {
  mkdir -p "$BATS_TEST_TMPDIR/ghw"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"
  GH=$BATS_TEST_TMPDIR/ghw/gh
  export AGENT_REAL_GH=$REPO_ROOT/tests/unit/stubs/gh
  export AGENT_GITHUB_TRUSTED_RESOLVED="mateusz-szczyrzyca ms-ai-agents"
  log=$AGENTS_HOME/logs/agent.jsonl
  mkdir -p "$AGENTS_HOME/logs"
  : > "$STUB_HOME/gh.log"
}

# bus_team id lead — a team on the bus of members lead and member, coordinated by lead ("" = nobody yet)
bus_team() {
  jq -nc --arg id "$1" --arg lead "$2" --arg home "$AGENTS_HOME" '{id: $id, repo: "repo", wake_cmd: "/bin/true",
      home: $home, lead: $lead, members: {
        lead: {agent: "lead", task: "L1", state: "active", tier: "normal"},
        member: {agent: "member", task: "M1", state: "active", tier: "normal"}}}' \
    | agentbus team-create --dir "$AGENTS_HOME/bus"
}

# claude_settings [CLAUDE_TOOL_SEARCH value] — the settings file run_claude gives Claude Code (unset: the default)
claude_settings() {
  cat > "$HOME/stub_cli.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$HOME/cli-args"
printf '{"type":"result","is_error":false,"result":"STATUS: DONE"}\n'
EOF
  chmod +x "$HOME/stub_cli.sh"
  mkdir -p "$AGENTS_HOME/tasks/S2"
  printf 'ID=S2\nKIND=worker\nAGENT=solo\nSESSION_ID=sid\nWORKTREE=%s\n' "$HOME/repo" > "$AGENTS_HOME/tasks/S2/meta.env"
  touch "$AGENTS_HOME/tasks/S2/prompt-1.md"
  printf 'CLAUDE_BIN=%q\n' "$HOME/stub_cli.sh" >> "$AGENTS_HOME/agents.conf"
  ( if (($#)); then export CLAUDE_TOOL_SEARCH=$1; fi
    source "$REPO_ROOT/bin/agent"
    watch_run() { wait "$1"; }
    meta_load S2
    run_claude S2 new 1 "$AGENTS_HOME/tasks/S2/prompt-1.md" "$AGENTS_HOME/tasks/S2" ) > /dev/null
  grep -A1 -x -- --settings "$HOME/cli-args" | tail -n 1
}

# round_team — a team of lead and member in round 2 of 5 whose PR is #7 on the lead's branch, stopped while it ran;
# prints the lead's task
round_team() {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  local t
  t=$(task_of lead)
  wait_status "$t" running
  agent stop --running > /dev/null
  printf 'PR_NUMBER=7\nPR_URL=https://github.com/o/r/pull/7\n' >> "$AGENTS_HOME/tasks/$t/meta.env"
  in_round "$t"
  echo "$t"
}

# in_round task — the task ended round 2 of 5 (as the runner records a STATUS: ROUND_DONE 2/5)
in_round() { printf 'WORK_ROUND=2\nWORK_ROUNDS=5\n' >> "$AGENTS_HOME/tasks/$1/meta.env"; }

# round_pr state — the stub GitHub's PR #7 of the lead's branch, in that state (OPEN or MERGED)
round_pr() {
  printf '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "%s", "headRefName": "%s"}]\n' \
    "$1" "$(meta "$(task_of lead)" BRANCH)" > "$HOME/gh-prs.json"
}

@test "Claude Code gets the agentbus tools up front: its tool search is off, unless CLAUDE_TOOL_SEARCH says otherwise" {
  f=$(claude_settings)
  [ "$(jq -r '.env.ENABLE_TOOL_SEARCH' "$f")" = false ]
  [ "$(jq -r '.attribution.commit' "$f")" = "" ]   # the rest of the settings as before
  f=$(claude_settings auto)
  [ "$(jq -r '.env.ENABLE_TOOL_SEARCH' "$f")" = auto ]
  f=$(claude_settings default)
  [ "$(jq -r '.env.ENABLE_TOOL_SEARCH // "unset"' "$f")" = unset ]
}

@test "a team member's gh pr merge, ready and close are refused, logged, and never reach gh" {
  setup_gh
  for cmd in "merge 53 --squash" "ready 53" "close 53"; do
    read -ra a <<< "$cmd"
    AGENT_GH_COORDINATOR=gemini-1 run ! "$GH" pr "${a[@]}"
    [[ $output == *"gemini-1"* ]]
    run ! grep -q "pr ${a[0]}" "$STUB_HOME/gh.log"
    grep '"event":"github_member_pr_refused"' "$log" | grep -q "\"action\":\"${a[0]}\""
  done
  AGENT_GH_COORDINATOR="" run "$GH" pr merge 53 --squash   # anyone else's: to gh as before
  grep -q 'gh pr merge 53 --squash' "$STUB_HOME/gh.log"
}

@test "the agents' gh asks the bus who coordinates when a PR command runs, not when the run started" {
  setup_gh
  bus_team T1 ""
  bus_team T2 lead
  # nobody coordinates yet (a new team's first run): a member opens no PR
  AGENT_GH_TEAM=T1 AGENT_GH_SELF=member run ! "$GH" pr create --base main --head agents/member/1 --title t --body b
  [[ $output == *"no coordinator yet"* ]]
  # the coordinator claimed since: the member is refused and told whom to tell
  AGENT_GH_TEAM=T2 AGENT_GH_SELF=member run ! "$GH" pr create --base main --head agents/member/1 --title t --body b
  [[ $output == *"coordinator (lead)"* ]]
  run ! grep -q 'pr create' "$STUB_HOME/gh.log"
  # the coordinator itself — even one that took over after its run started — opens the round's PR
  AGENT_GH_COORDINATOR=old AGENT_GH_TEAM=T2 AGENT_GH_SELF=lead run "$GH" pr create --base main --head agents/lead/1 --title t --body b
  grep -q 'gh pr create --base main --head agents/lead/1' "$STUB_HOME/gh.log"
}

@test "a team task's gh knows its team and agent; a solo task's and an investigator's do not" {
  for x in "M1 member T-team worker" "S1 solo - worker" "I1 inv T-team investigation"; do
    read -r id agent team kind <<< "$x"
    mkdir -p "$AGENTS_HOME/tasks/$id"
    printf 'ID=%s\nAGENT=%s\nKIND=%s\nREPO_ROOT=%s\n' "$id" "$agent" "$kind" "$HOME/repo" > "$AGENTS_HOME/tasks/$id/meta.env"
    [[ $team == - ]] || printf 'TEAM=%s\n' "$team" >> "$AGENTS_HOME/tasks/$id/meta.env"
  done
  env_of() { (source "$REPO_ROOT/bin/agent"; RUNNER_ID=$1 REPO_ROOT=$HOME/repo agents_gh_env > /dev/null
              printf '%s/%s' "${AGENT_GH_TEAM:-}" "${AGENT_GH_SELF:-}"); }
  [ "$(env_of M1)" = T-team/member ]
  [ "$(env_of S1)" = / ]
  [ "$(env_of I1)" = / ]
}

@test "a round resumed tells a member to push its own branch: its coordinator merges it into the round's PR" {
  t=$(round_team)
  round_pr OPEN
  agent run --resume "$(meta "$t" TEAM)" --same-round "go on" > /dev/null
  f=$AGENTS_HOME/tasks/$(task_of lead)/task.md
  grep -q "This round's PR is #7" "$f"
  grep -q "a member pushes its own branch" "$f"
  grep -q "A member never pushes to the coordinator's branch" "$f"
  run ! grep -q "a member — or a new coordinator — merges" "$f"
  agent stop --running > /dev/null
}

@test "a round continued again has one heading and one PR note, the newest; what its reviews sent back stays" {
  t=$(round_team)
  round_pr OPEN
  AGENT_FIX_PASS=1 agent run --resume "$(meta "$t" TEAM)" --same-round "fix finding X" > /dev/null
  t2=$(task_of lead)
  wait_status "$t2" running
  agent stop --running > /dev/null
  in_round "$t2"
  agent run --resume "$(meta "$t2" TEAM)" --same-round "human note Y" > /dev/null
  f=$AGENTS_HOME/tasks/$(task_of lead)/task.md
  [ "$(grep -c '^## Round 2 of 5 continues' "$f")" -eq 1 ]
  [ "$(grep -c "^## The round's PR" "$f")" -eq 1 ]
  grep -q "fix finding X" "$f"
  grep -q "human note Y" "$f"
  agent stop --running > /dev/null
}

@test "a round whose PR is merged continues as its follow-up, and the next round is not started early" {
  t=$(round_team)
  round_pr MERGED
  agent run --resume "$(meta "$t" TEAM)" --same-round "one more test for round 2" > /dev/null
  f=$AGENTS_HOME/tasks/$(task_of lead)/task.md
  grep -q "PR #7 is merged" "$f"
  grep -q "Round 3 has not started" "$f"
  run ! grep -q "its PR is not merged" "$f"
  wait_status "$(task_of lead)" running
  agent stop --running > /dev/null
  t=$(task_of lead)
  in_round "$t"
  round_pr OPEN
  agent run --resume "$(meta "$t" TEAM)" --same-round "go on" > /dev/null
  f=$AGENTS_HOME/tasks/$(task_of lead)/task.md
  grep -q "its PR is not merged" "$f"
  grep -q "Round 3 has not started" "$f"
  run ! grep -q "PR #7 is merged" "$f"
  agent stop --running > /dev/null
}

@test "a team member's gh pr reopen and edit are refused, logged, and never reach gh" {
  setup_gh
  for cmd in "reopen 53" "edit 53 --title new"; do
    read -ra a <<< "$cmd"
    AGENT_GH_COORDINATOR=gemini-1 run ! "$GH" pr "${a[@]}"
    [[ $output == *"gemini-1"* ]]
    run ! grep -q "pr ${a[0]}" "$STUB_HOME/gh.log"
    grep '"event":"github_member_pr_refused"' "$log" | grep -q "\"action\":\"${a[0]}\""
  done
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

@test "a same-round resume prefers the coordinator's WORK_ROUND over a member's stale later round" {
  scenario '{"lead": {"new": "team-lead-round:2/5"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member)
  wait_status "$t" 'done'
  agent stop --running > /dev/null
  printf 'PR_NUMBER=7\nPR_URL=https://github.com/o/r/pull/7\n' >> "$AGENTS_HOME/tasks/$t/meta.env"
  printf 'WORK_ROUND=3\nWORK_ROUNDS=5\n' >> "$AGENTS_HOME/tasks/$m/meta.env"
  round_pr OPEN
  agent run --resume "$(meta "$t" TEAM)" --same-round "fix the review" > /dev/null
  f=$AGENTS_HOME/tasks/$(task_of lead)/task.md
  grep -q '^## Round 2 of 5 continues' "$f"
  run ! grep -q '^## Round 3 of 5 continues' "$f"
  agent stop --running > /dev/null
}

@test "agent archive packs and removes each team member's own checkpoint" {
  command -v 7z >/dev/null || skip "7z not installed"
  t=$(round_team)
  m=$(task_of member)
  team=$(meta "$t" TEAM)
  mkdir -p "$AGENTS_HOME/bus/checkpoints"
  printf '\n## Checkpoint — member\n\nmember own record\n' > "$AGENTS_HOME/bus/checkpoints/task-$m.md"
  agent archive "$team" > /dev/null
  archive=$AGENTS_HOME/tasks-archived/$team.7z
  [ -f "$archive" ]
  run 7z x -so "$archive" "checkpoints/task-$m.md"
  [ "$status" -eq 0 ]
  [[ $output == *"member own record"* ]]
  [ ! -e "$AGENTS_HOME/bus/checkpoints/task-$m.md" ]
}

@test "a team member cannot create, merge, close or edit a PR through gh api" {
  setup_gh
  commands=(
    "api -X PUT repos/o/r/pulls/53/merge -f merge_method=squash"
    "api repos/o/r/pulls/53 --method PATCH -f state=closed"
    "api repos/o/r/pulls -X POST -f title=member"
    "api repos/o/r/pulls -f title=member -f head=member -f base=main"
    "api --method=PUT repos/o/r/pulls/53/merge"
  )
  for cmd in "${commands[@]}"; do
    read -ra a <<< "$cmd"
    : > "$STUB_HOME/gh.log"
    AGENT_GH_COORDINATOR=gemini-1 run ! "$GH" "${a[@]}"
    [[ $output == *"gemini-1"* ]]
    run ! grep -q '^gh api ' "$STUB_HOME/gh.log"
    grep '"event":"github_member_pr_refused"' "$log" | grep -q '"action":"api"'
  done
  AGENT_GH_COORDINATOR=gemini-1 run "$GH" api repos/o/r/pulls/53
  [ "$status" -eq 0 ]
  grep -q '^gh api repos/o/r/pulls/53$' "$STUB_HOME/gh.log"
  AGENT_GH_COORDINATOR="" run "$GH" api -X PUT repos/o/r/pulls/53/merge -f merge_method=squash
  [ "$status" -eq 0 ]
  grep -q '^gh api -X PUT repos/o/r/pulls/53/merge' "$STUB_HOME/gh.log"
}
