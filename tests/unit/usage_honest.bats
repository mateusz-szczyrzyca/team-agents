#!/usr/bin/env bats
# What the tables of usage said was short in two ways. opencode reports a cost of 0 for a provider whose prices it does
# not know: agent spend priced such a run from MODEL_PRICES, agent tokens showed 0 — now it prices it the same way,
# marked. And opencode (2.1.3) never reports the usage of a run's last step: of 622 runs, 596 lacked exactly that one
# step_finish, and a run that answered at once reported nothing — agent tokens and agent spend now say their numbers
# are short. agent debug --agent counts the runs in which the agent did nothing — no command, no edit, no tool call:
# it answered from its prompt alone (four of five runs of two models).

load helpers

# task id meta — a task directory with that meta.env and an empty activity
task() { mkdir -p "$AGENTS_HOME/tasks/$1"; printf '%s' "$2" > "$AGENTS_HOME/tasks/$1/meta.env"; : > "$AGENTS_HOME/tasks/$1/activity.jsonl"; }
# act id kind text — a line in the task's activity
act() { jq -nc --arg k "$2" --arg t "$3" '{ts: "2026-10-31 10:00:00", k: $k, t: $t}' >> "$AGENTS_HOME/tasks/$1/activity.jsonl"; }

@test "agent tokens prices a run whose provider reported no cost from MODEL_PRICES, marked; a reported cost stays" {
  printf "MODEL_PRICES[deepinfra/m]='1 0.5 4'\n" >> "$AGENTS_HOME/agents.conf"
  task P-free $'ID=P-free\nAGENT=w\nKIND=helper\nMODEL=deepinfra/m\nTYPE=claude\n'
  act P-free info 'usage: in 1000000 (cached 500000), out 250000, cost $0.0000'
  task P-paid $'ID=P-paid\nAGENT=w\nKIND=helper\nMODEL=deepinfra/m\nTYPE=claude\n'
  act P-paid info 'usage: in 1000 (cached 0), out 10, cost $0.0420'
  run agent tokens P-free
  [ "$status" -eq 0 ]
  [[ $(grep '^P-free ' <<< "$output") == *" 1.7500* "* ]]   # (500k × 1 + 500k × 0.5 + 250k × 4) / 1M
  [[ $(grep '^total ' <<< "$output") == *" 1.7500"* ]]
  [[ $output == *"* priced from MODEL_PRICES: the provider reported no cost"* ]]
  run agent tokens P-paid
  [[ $(grep '^P-paid ' <<< "$output") == *" 0.0420 "* ]]
  [[ $output != *"priced from MODEL_PRICES"* ]]
}

@test "agent tokens and agent spend say that opencode's usage is short: its runs' last step is not reported" {
  task O-1 $'ID=O-1\nAGENT=w\nKIND=helper\nMODEL=deepinfra/m\nTYPE=opencode\nCREATED=2026-10-31T10:00:00+01:00\n'
  act O-1 info '▶ run 1 (new)'   # agent spend dates a run by its start
  act O-1 info 'step: tool-calls, tokens in/out 1000/50, cost $0.0010'
  run agent tokens O-1
  [[ $output == *"opencode does not report the usage of a run's last step"* ]]
  task C-1 $'ID=C-1\nAGENT=w\nKIND=helper\nMODEL=m\nTYPE=claude\n'
  act C-1 info 'usage: in 1000 (cached 0), out 10, cost $0.0420'
  run agent tokens C-1
  [[ $output != *"opencode does not report"* ]]
  run agent spend --days 100000
  [[ $output == *"opencode does not report the usage of a run's last step"* ]]
}

@test "agent debug --agent counts the runs in which the agent did nothing: no command, edit or tool call" {
  now=$(date -Is)
  task D-1 "$(printf 'ID=D-1\nAGENT=wx\nKIND=helper\nMODEL=m\nTYPE=opencode\nSTATUS=done\nCREATED=%s\nFINISHED=%s\n' "$now" "$now")"
  act D-1 info '▶ run 1 (new)'
  act D-1 text 'STATUS: UNCLEAR'
  act D-1 info '▶ run 2 (resume)'
  act D-1 cmd 'ls'
  act D-1 text 'STATUS: DONE'
  task D-2 "$(printf 'ID=D-2\nAGENT=wx\nKIND=helper\nMODEL=m\nTYPE=opencode\nSTATUS=done\nCREATED=%s\nFINISHED=%s\n' "$now" "$now")"
  act D-2 info '▶ run 1 (new)'
  act D-2 text 'STATUS: DONE'
  agent debug --agent wx --out "$BATS_TEST_TMPDIR/d.txt" > /dev/null
  grep -q "^runs without an action (no command, edit or tool call): 2 of 3 (66%)" "$BATS_TEST_TMPDIR/d.txt"
}
