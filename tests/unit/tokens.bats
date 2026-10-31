#!/usr/bin/env bats
# agent tokens <team|task>: what the runs used, from their activity's usage lines (and opencode's step lines) — per task,
# a team's helpers and investigations included, and in all.

load helpers

# task id meta — a task directory with that meta.env and an empty activity
task() { mkdir -p "$AGENTS_HOME/tasks/$1"; printf '%s' "$2" > "$AGENTS_HOME/tasks/$1/meta.env"; : > "$AGENTS_HOME/tasks/$1/activity.jsonl"; }
# act id text — an info line in the task's activity
act() { jq -nc --arg t "$2" '{ts: "2026-10-31 10:00:00", k: "info", t: $t}' >> "$AGENTS_HOME/tasks/$1/activity.jsonl"; }

@test "agent tokens sums what a team's runs used: per task, with its helpers and investigations, and in all" {
  task T1-lead $'ID=T1-lead\nAGENT=lead\nTEAM=T1-team\nKIND=worker\n'
  act T1-lead 'usage: in 11000 (cached 9000), out 300, cost $0.0420'
  act T1-lead 'usage: in 5000 (cached 4000), out 200'
  task T1-member $'ID=T1-member\nAGENT=member\nTEAM=T1-team\nKIND=worker\n'
  act T1-member 'step: stop, tokens in/out 1000/50, cost $0.0010'
  act T1-member 'step: tool-calls, tokens in/out 2000/100, cost $0.0020'
  task H1-helper $'ID=H1-helper\nAGENT=openrouter-minimax\nKIND=helper\nPARENT_TASK=T1-lead\n'
  act H1-helper 'step: stop, tokens in/out 500/20, cost $0.0005'
  task X-other $'ID=X-other\nAGENT=x\nTEAM=X-team\nKIND=worker\n'
  act X-other 'usage: in 999999, out 1'
  run agent tokens T1-team
  [ "$status" -eq 0 ]
  echo "$output" >&3
  [[ $output == *"T1-lead"*"16000"*"13000"*"500"*"0.0420"* ]]
  [[ $output == *"T1-member"*"3000"*"150"*"0.0030"* ]]
  [[ $output == *"H1-helper"*"500"*"20"* ]]
  [[ $output == *"total"*"19500"*"13000"*"670"*"0.0455"* ]]
  [[ $output != *"999999"* ]]
}

@test "agent tokens shows a task whose runs reported no usage, instead of leaving it out" {
  task T2-lead $'ID=T2-lead\nAGENT=lead\nTEAM=T2-team\nKIND=worker\n'
  act T2-lead 'usage: in 100 (cached 50), out 10'
  task T2-member $'ID=T2-member\nAGENT=member\nTEAM=T2-team\nKIND=worker\n'
  act T2-member 'something else'
  run agent tokens T2-team
  [ "$status" -eq 0 ]
  [[ $output == *"T2-member"*"no usage reported"* ]]
}

@test "agent tokens --runs <task>: per run, its steps (tool calls) and tokens" {
  task R1 $'ID=R1\nAGENT=codex-1\nTEAM=R-team\nKIND=worker\n'
  act R1 '▶ run 1 (new)'
  jq -nc '{ts: "2026-10-31 10:00:01", k: "cmd", t: "make gate"}' >> "$AGENTS_HOME/tasks/R1/activity.jsonl"
  jq -nc '{ts: "2026-10-31 10:00:02", k: "mcp", t: "read_messages {}"}' >> "$AGENTS_HOME/tasks/R1/activity.jsonl"
  act R1 'usage: in 100 (cached 50), out 10'
  act R1 '▶ run 2 (resume)'
  jq -nc '{ts: "2026-10-31 10:05:01", k: "tool", t: "Read x"}' >> "$AGENTS_HOME/tasks/R1/activity.jsonl"
  act R1 'usage: in 300 (cached 250), out 30'
  run agent tokens --runs R1
  [ "$status" -eq 0 ]
  echo "$output" >&3
  [[ $output == *$'\n'*"1"*"2"*"100"*"50"*"10"* ]]
  [[ $output == *"2"*"1"*"300"*"250"*"30"* ]]
}
