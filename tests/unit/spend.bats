#!/usr/bin/env bats
# Where does the money go? agent tokens counts a team or a task; agent spend counts every task of the last days by
# model: what a model paid by the token (opencode, AGENT_PAID_CLIS) spent — its provider's cost, else the price list's —
# and what a subscription's model would have cost at its API price (MODEL_PRICES; else its CLI's own cost), as an
# equivalent: not money spent, but what eats its limit.

load helpers

# task id type model day usage-line… — a task of one run on the day, with its usage lines
task() {
  local id=$1 type=$2 model=$3 day=$4 l
  shift 4
  mkdir -p "$AGENTS_HOME/tasks/$id"
  printf 'ID=%s\nAGENT=a-%s\nTYPE=%s\nMODEL=%s\nKIND=worker\n' "$id" "$id" "$type" "$model" > "$AGENTS_HOME/tasks/$id/meta.env"
  jq -nc --arg ts "$day 10:00:00" '{ts: $ts, k: "info", t: "▶ run 1 (new)"}' > "$AGENTS_HOME/tasks/$id/activity.jsonl"
  jq -nc --arg ts "$day 10:00:01" '{ts: $ts, k: "cmd", t: "ls"}' >> "$AGENTS_HOME/tasks/$id/activity.jsonl"
  for l in "$@"; do jq -nc --arg ts "$day 10:00:02" --arg t "$l" '{ts: $ts, k: "info", t: $t}' >> "$AGENTS_HOME/tasks/$id/activity.jsonl"; done
}

@test "agent spend: a paid model's spending, a subscription's equivalent, by model" {
  today=$(date +%F)
  task P1 opencode deepinfra/qwen "$today" 'usage: in 1000 (cached 0), out 100, cost $0.5'
  task P2 opencode deepinfra/qwen "$today" 'usage: in 2000 (cached 0), out 200, cost $0.25'
  task S1 claude haiku "$today" 'usage: in 5000 (cached 4000), out 300, cost $0.75'
  run agent spend
  [ "$status" -eq 0 ]
  [[ $(grep 'deepinfra/qwen' <<< "$output") == *"paid"*"3000"*"0.7500"* ]]
  [[ $(grep 'haiku' <<< "$output") == *"subscription"*"5000"*"4000"*"300"*"0.7500"* ]]
  [[ $output == *"total"* ]]
}

@test "agent spend: a subscription's model in MODEL_PRICES is priced at its API rates, cached input at its own" {
  printf 'MODEL_PRICES[gpt-6]="1.25 0.125 10"\n' >> "$AGENTS_HOME/agents.conf"
  task C1 codex gpt-6 "$(date +%F)" 'usage: in 1000000 (cached 600000), out 100000'
  run agent spend
  [ "$status" -eq 0 ]
  [[ $(grep 'gpt-6' <<< "$output") == *"subscription"*"1.5750"* ]]   # 0.4M at 1.25, 0.6M at 0.125, 0.1M at 10
}

@test "agent spend --days: older runs are left out; --daily shows a line a day and model" {
  task O1 opencode deepinfra/qwen "$(date -d '-10 days' +%F)" 'usage: in 9000 (cached 0), out 900, cost $9'
  task N1 opencode deepinfra/qwen "$(date +%F)" 'usage: in 1000 (cached 0), out 100, cost $1'
  run agent spend --days 7
  [ "$status" -eq 0 ]
  [[ $(grep 'deepinfra/qwen' <<< "$output") == *"1000"*"1.0000"* ]]
  run agent spend --days 30 --daily
  [ "$status" -eq 0 ]
  [ "$(grep -c 'deepinfra/qwen' <<< "$output")" -eq 2 ]
}
