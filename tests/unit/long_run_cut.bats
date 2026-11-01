#!/usr/bin/env bats
# A team member's run that reaches AGENT_LONG_RUN_HARD steps is ended by the runner: asked to end at
# AGENT_LONG_RUN_STEPS, an agent may go on; the watchdog ends it, and its next run starts a fresh session.

load helpers

@test "the watchdog ends a run at its step limit and says so" {
  ( source "$REPO_ROOT/bin/agent"
    mkdir -p "$TASKS/X"
    : > "$TASKS/X/activity.jsonl"
    # shellcheck disable=SC2034  # read by watch_run, sourced above
    RUNNER_ID=X AGENT_RUN_HARD_AT=3 AGENT_HANG_POLL=1
    sleep 60 &
    pid=$!
    ( sleep 0.5
      for _ in 1 2 3; do jq -nc '{ts: "2026-11-01 10:00:00", k: "cmd", t: "ls"}' >> "$TASKS/X/activity.jsonl"; done ) &
    watch_run "$pid" "$TASKS/X" 1 || true   # the killed process's status
    [ "$LONG_CUT" = 1 ]
    ! kill -0 "$pid" 2> /dev/null
    grep -q "limit of 3 steps" "$TASKS/X/activity.jsonl" )
}

@test "a run under its step limit is not ended" {
  ( source "$REPO_ROOT/bin/agent"
    mkdir -p "$TASKS/Y"
    : > "$TASKS/Y/activity.jsonl"
    # shellcheck disable=SC2034  # read by watch_run, sourced above
    RUNNER_ID=Y AGENT_RUN_HARD_AT=5 AGENT_HANG_POLL=1
    sleep 3 &
    pid=$!
    jq -nc '{ts: "2026-11-01 10:00:00", k: "cmd", t: "ls"}' >> "$TASKS/Y/activity.jsonl"
    watch_run "$pid" "$TASKS/Y" 1
    [ "$LONG_CUT" = 0 ] )
}
