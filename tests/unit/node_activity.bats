#!/usr/bin/env bats
# The node's heartbeat (agent node): the activity of its running jobs, so the lead shows them like local tasks; an
# agent whose CLI is missing is not offered as free; a failed job without a final message reports its task's note.

load helpers

# a running job's task T1 (agent solo) with two activity lines, the node's state knowing it as job-1
job_task() {
  mkdir -p "$TASKS/T1"
  printf 'AGENT=solo\nSTATUS=running\nJOB_ID=job-1\nRUN=1\n' > "$TASKS/T1/meta.env"
  printf '%s\n' '{"ts":"2026-10-27 10:00:01","k":"cmd","t":"make gate"}' '{"ts":"2026-10-27 10:00:05","k":"text","t":"checking the parser"}' > "$TASKS/T1/activity.jsonl"
  # shellcheck disable=SC2034 # read by the sourced script
  NODE_STATE='{"jobs":{"job-1":{"task_id":"T1","launched":true,"state":"running"}}}'
}

@test "the heartbeat carries what the node's running jobs are doing" {
  source "$REPO_ROOT/bin/agent"
  job_task
  out=$(node_heartbeat_json)
  [ "$(jq -r '.Activity[0].ID' <<< "$out")" = job-1 ]
  [ "$(jq -r '.Activity[0].Agent' <<< "$out")" = solo ]
  [ "$(jq -r '.Activity[0].Stage' <<< "$out")" = running ]
  [ "$(jq -r '.Activity[0].Lines | length' <<< "$out")" = 2 ]
  [[ $(jq -r '.Activity[0].Lines[-1]' <<< "$out") == *"10:00:05"*"checking the parser"* ]]
}

@test "an agent whose CLI is missing is not offered as free" {
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034,SC2154 # the sourced configuration's associative array and settings
  AGENT_TYPE[ghost]=codex
  # shellcheck disable=SC2034
  CODEX_BIN=/nonexistent/codex
  out=$(node_heartbeat_json)
  run jq -e '.FreeAgents[] | select(.Name == "ghost")' <<< "$out"
  [ "$status" -ne 0 ]
  jq -e '.FreeAgents[] | select(.Name == "solo")' <<< "$out" > /dev/null
}

@test "a failed job without a final message reports its task's note" {
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/T2"
  printf 'AGENT=solo\nSTATUS=failed\nNOTE=CLI\\ exit\\ code:\\ 127\nRUN=1\n' > "$TASKS/T2/meta.env"
  [[ $(node_job_result T2 failed) == *"CLI exit code: 127"* ]]
}
