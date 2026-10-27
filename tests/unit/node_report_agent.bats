#!/usr/bin/env bats
# Job reports retain the chosen agent when live activity ends.

load helpers

@test "node reports carry the chosen agent from start through completion" {
  sed -n '/^node_transition() {/,/^}/p' "$BATS_TEST_DIRNAME/../../bin/agent" > "$HOME/node-transition.sh"
  run bash -c '
    set -euo pipefail
    source "$HOME/node-transition.sh"
    INSTANCE_NAME=node-a
    NODE_STATE="{\"jobs\":{\"job-1\":{\"agent\":\"actual-agent\",\"assignment\":1,\"task_id\":\"task-1\",\"branch\":\"topic\",\"assigned_at\":\"2026-01-01T00:00:00Z\"}}}"
    node_time() { printf "%s" "$2"; }
    node_save() { printf "%s\n" "$NODE_STATE" >> "$HOME/node-reports.jsonl"; }
    node_transition job-1 running 2026-01-01T00:00:01Z
    node_transition job-1 done 2026-01-01T00:00:02Z finished
  '
  [ "$status" -eq 0 ]
  jq -se 'length == 2 and all(.[]; .jobs["job-1"].report.Agent == "actual-agent") and .[0].jobs["job-1"].report.State == "running" and .[1].jobs["job-1"].report.State == "done" and (.[1].jobs["job-1"].report.Transitions|length) == 2' "$HOME/node-reports.jsonl"
}
