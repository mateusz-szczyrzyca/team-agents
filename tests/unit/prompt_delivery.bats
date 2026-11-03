#!/usr/bin/env bats
# What the prompts deliver to each role: a reviewer gets its review request (independence, re-review, the previous
# review) and the rules for writing on GitHub; an independent investigator's later runs do not repeat the first
# prompt; a helper is not told to fill the repository's skipped tests; Claude Code adds no attribution of its own.

load helpers

# task id key=value... — a task directory with meta.env (the given fields over a reviewer's defaults) and task.md
task() {
  local id=$1; shift
  mkdir -p "$AGENTS_HOME/tasks/$id"
  { printf 'ID=%s\nAGENT=inv\nTYPE=claude\nREPO_NAME=repo\nREPO_ROOT=%s\nWORKTREE=%s\nBRANCH=b\nBASE=main\nRUN=1\n' \
      "$id" "$HOME/repo" "$HOME/repo"
    printf '%s\n' "$@"; } > "$AGENTS_HOME/tasks/$id/meta.env"
  echo "Investigate the cache" > "$AGENTS_HOME/tasks/$id/task.md"
}

# prompt id mode — the prompt build_prompt renders for that task in that mode
prompt() {
  ( source "$REPO_ROOT/bin/agent"
    github_trusted() { printf 'owner'; }
    meta_load "$1"
    FRESH_WHY=test build_prompt "$1" "$2" "$AGENTS_HOME/tasks/$1/questions-1.md" 1 )
}

@test "a reviewer's task gives its review request: independence, the re-review and the previous review" {
  local j out
  j=$(jq -n '{kind: "review", pr_url: "https://github.com/o/r/pull/7", pr_number: 7,
              question: "Review PR #7 (https://github.com/o/r/pull/7) — one of two independent reviews: do not read other reviews. Re-review after fix pass 1: first check the findings.\n\nYour previous review:\n1. major: the cache never expires"}')
  task C1
  echo "Add a cache" > "$AGENTS_HOME/tasks/C1/task.md"
  out=$(source "$REPO_ROOT/bin/agent"; investigation_task_text "$j" C1 lead)
  [[ $out == *"## Review PR #7"* ]]
  [[ $out == *"one of two independent reviews"* ]]
  [[ $out == *"Re-review after fix pass 1"* ]]
  [[ $out == *"the cache never expires"* ]]
  [[ $out == *"Add a cache"* ]]
}

@test "a plain review request is not repeated; an investigation request keeps its question and context" {
  local j out
  task C2
  j=$(jq -n '{kind: "review", pr_url: "https://github.com/o/r/pull/8", pr_number: 8, question: "Review PR #8 (https://github.com/o/r/pull/8)"}')
  out=$(source "$REPO_ROOT/bin/agent"; investigation_task_text "$j" C2 lead)
  [ "$(grep -c 'Review PR #8' <<< "$out")" -eq 1 ]
  j=$(jq -n '{question: "Why does it leak?", context: "valgrind output"}')
  out=$(source "$REPO_ROOT/bin/agent"; investigation_task_text "$j" C2 lead)
  [[ $out == *"## Question from lead"*"Why does it leak?"*"valgrind output"* ]]
}

@test "a reviewer gets the rules for writing on GitHub: files and commits as links" {
  task R1 KIND=investigation REVIEW=1 PR_NUMBER=7 PR_URL=https://github.com/o/r/pull/7 INV_ID=3
  out=$(prompt R1 new)
  [[ $out == *"WORKING CONTEXT — PR REVIEW"* ]]
  [[ $out == *"## Writing for the human"* ]]
  [[ $out == *"**A file is a link to it**"* ]]
  [[ $out != *"{{"* ]]
}

@test "independent investigators get the writing rules in every phase, the review's recommendation first" {
  local p
  for p in 1 2 3; do
    task "I$p" KIND=investigation INVESTIGATION_TEAM=T1 "INVESTIGATION_PHASE=$p" PR_NUMBER=7 INV_ID=4
    out=$(prompt "I$p" new)
    [[ $out == *"## Writing for the human"* ]]
    [[ $out != *"{{"* ]]
  done
  out=$(prompt I3 new)
  [[ $out == *'## Recommendation'* ]]
  [[ $out == *'✅ $\color{green}{\textbf{Ready to merge}}$'* ]]
  [[ $out == *'🔧 $\color{red}{\textbf{Needs changes}}$'* ]]
}

@test "an independent investigator's resume does not repeat its first prompt; a fresh session has it once" {
  task I4 KIND=investigation INVESTIGATION_TEAM=T1 INVESTIGATION_PHASE=1
  out=$(prompt I4 resume)
  [[ $out != *"WORKING CONTEXT — INVESTIGATION TEAM"* ]]
  [[ $out == *"Continue task I4"* ]]
  out=$(prompt I4 fresh)
  [ "$(grep -c 'WORKING CONTEXT — INVESTIGATION TEAM' <<< "$out")" -eq 1 ]
  [ "$(grep -c 'Investigate the cache' <<< "$out")" -eq 1 ]
}

@test "a helper is not told to fill the repository's skipped tests; a solo task is" {
  mkdir -p "$HOME/repo/tests"
  printf '@test "x" {\n  skip "needs the device"\n}\n' > "$HOME/repo/tests/x.bats"
  git -C "$HOME/repo" add tests/x.bats   # git grep finds the tracked files
  task H1 KIND=helper TIER=worker HELPER_ID=2 FROM_AGENT=lead
  out=$(prompt H1 new)
  [[ $out != *"Skipped tests in this repository"* ]]
  task S1 KIND=worker AGENT=solo
  out=$(prompt S1 new)
  [[ $out == *"Skipped tests in this repository"* ]]
}

@test "Claude Code runs with its own attribution off: no Co-Authored-By, no 'Generated with Claude Code'" {
  cat > "$HOME/stub_cli.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$HOME/cli-args"
printf '{"type":"result","is_error":false,"result":"STATUS: DONE"}\n'
EOF
  chmod +x "$HOME/stub_cli.sh"
  task S2 KIND=worker AGENT=solo SESSION_ID=sid
  touch "$AGENTS_HOME/tasks/S2/prompt-1.md"
  ( source "$REPO_ROOT/bin/agent"
    CLAUDE_BIN=$HOME/stub_cli.sh CLI_SESSION=()
    watch_run() { wait "$1"; }
    meta_load S2
    run_claude S2 new 1 "$AGENTS_HOME/tasks/S2/prompt-1.md" "$AGENTS_HOME/tasks/S2" )
  local f
  f=$(grep -A1 -x -- --settings "$HOME/cli-args" | tail -n 1)
  [ -f "$f" ]
  [ "$(jq -r '.attribution.commit' "$f")" = "" ]
  [ "$(jq -r '.attribution.pr' "$f")" = "" ]
  [ "$(jq -r '.includeCoAuthoredBy' "$f")" = false ]
}

@test "the agents' gh leaves out a 'Generated with Claude Code' line from what it publishes" {
  mkdir -p "$BATS_TEST_TMPDIR/ghw" "$AGENTS_HOME/logs"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"
  printf '#!/usr/bin/env bash\nprev=""; for a in "$@"; do [[ $prev != --body ]] || printf "%%s" "$a" > "$HOME/real-body"; prev=$a; done\n' \
    > "$BATS_TEST_TMPDIR/real-gh"
  chmod +x "$BATS_TEST_TMPDIR/real-gh"
  AGENT_REAL_GH=$BATS_TEST_TMPDIR/real-gh "$BATS_TEST_TMPDIR/ghw/gh" pr create --title t \
    --body $'Fixes it.\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\n\nOpened by team-agents:'
  [ "$(cat "$HOME/real-body")" = $'Fixes it.\n\nOpened by team-agents:' ]
}
