#!/usr/bin/env bats
# A resumed run is followed by a fresh session only when its context outgrew the task's own session starts: more than
# AGENT_FRESH_SESSION_RATIO times their median input a step (and more than AGENT_FRESH_SESSION_TOKENS). A fresh session
# itself reads tens of thousands of tokens a step from its start, and a fixed threshold sent nearly every run to a new one.

load helpers

# run_lines task n mode steps in — the activity of one run: its start, its tool calls, its usage
run_lines() {
  local a="$AGENTS_HOME/tasks/$1/activity.jsonl" i
  jq -nc --arg t "▶ run $2 ($3)" '{ts: "2026-11-01 10:00:00", k: "info", t: $t}' >> "$a"
  for ((i = 0; i < $4; i++)); do jq -nc '{ts: "2026-11-01 10:00:01", k: "cmd", t: "ls"}' >> "$a"; done
  jq -nc --arg t "usage: in $5 (cached 0), out 10" '{ts: "2026-11-01 10:00:02", k: "info", t: $t}' >> "$a"
}

@test "a resumed run starts a fresh session only past twice the task's session starts; --runs shows each run's mode" {
  mkdir -p "$AGENTS_HOME/tasks/F"
  printf 'ID=F\nAGENT=gemini-1\nTYPE=gemini\nKIND=worker\n' > "$AGENTS_HOME/tasks/F/meta.env"
  run_lines F 1 new 3 180000      # 60,000 a step
  run_lines F 2 fresh 3 180000    # 60,000 a step
  run_lines F 3 resume 3 300000   # 100,000: under twice the starts
  ( source "$REPO_ROOT/bin/agent"; ! fresh_session_due F )
  run_lines F 4 resume 3 450000   # 150,000: over
  ( source "$REPO_ROOT/bin/agent"; fresh_session_due F && [[ $FRESH_WHY == *"60000"* ]] )
  run agent tokens --runs F
  [[ $(grep '^2 ' <<< "$output") == *fresh* ]]
  [[ $(grep '^4 ' <<< "$output") == *resume* ]]
}
