#!/usr/bin/env bats
# Is a model fit for the work? agent debug --agent NAME (or --model NAME, every agent on that model) writes one file:
# a summary — tasks by kind and status, failures and their reasons, turns without STATUS, pauses, long runs cut,
# failovers, durations, tokens and cost, worker requests that failed before a task, results the requesters rejected
# (an approximation from their messages) — and the details of its last tasks: what each got, what it returned.

load helpers

# mk_task id agent kind model status note created helper-id
mk_task() {
  local d="$AGENTS_HOME/tasks/$1"
  mkdir -p "$d"
  { printf 'ID=%s\nAGENT=%s\nKIND=%s\nMODEL=%s\nTYPE=opencode\nSTATUS=%s\n' "$1" "$2" "$3" "$4" "$5"
    printf 'NOTE=%q\nCREATED=%s\nFINISHED=%s\nHELPER_ID=%s\n' "$6" "$7" "$7" "${8:-}"; } > "$d/meta.env"
  printf 'the request of %s\n' "$1" > "$d/task.md"
  printf 'result of %s\nSTATUS: %s\n' "$1" "${5^^}" > "$d/final.md"
  jq -nc --arg ts "${7:0:10} 10:00:00" '{ts: $ts, k: "info", t: "▶ run 1 (new)"}' > "$d/activity.jsonl"
  jq -nc --arg ts "${7:0:10} 10:05:00" '{ts: $ts, k: "info", t: "usage: in 1000 (cached 0), out 50, cost $0.0100"}' >> "$d/activity.jsonl"
}

@test "agent debug --agent: a summary of its tasks and their failures, and the details of its tasks only" {
  now=$(date -Is)
  mk_task A1 wx helper mx "done" "" "$now" 1
  mk_task A2 wx helper mx failed "the CLI exited 1" "$now" 2
  mk_task A3 wx helper mx failed "the CLI exited 1" "$now" 3
  mk_task B1 wy helper my "done" "" "$now" 4
  agent debug --agent wx --out "$BATS_TEST_TMPDIR/d.txt" > /dev/null
  f=$BATS_TEST_TMPDIR/d.txt
  grep -q "^tasks: 3" "$f"
  grep -q "^failed: 2 (66%)" "$f"
  grep -qE "^ +2 +the CLI exited 1" "$f"
  grep -q "tokens: in 3000 (cached 0), out 150, cost \$0.0300" "$f"
  grep -q "^== task A2" "$f"
  ! grep -q "^== task B1" "$f" || false
  grep -q "the request of A2" "$f"
  grep -q "result of A1" "$f"
}

@test "agent debug --model: the tasks of every agent on that model" {
  now=$(date -Is)
  mk_task A1 wx helper mx "done" "" "$now" 1
  mk_task B1 wy helper my "done" "" "$now" 2
  mk_task C1 wz worker my failed "out of quota" "$now"
  agent debug --model my --out "$BATS_TEST_TMPDIR/d.txt" > /dev/null
  f=$BATS_TEST_TMPDIR/d.txt
  grep -q "^tasks: 2" "$f"
  grep -q "^== task B1" "$f"
  grep -q "^== task C1" "$f"
  ! grep -q "^== task A1" "$f" || false
}

@test "agent debug --agent --days and --last: older tasks out of the window; the details of the last ones only" {
  mk_task OLD wx helper mx failed "old failure" "$(date -Is -d '40 days ago')" 1
  mk_task N1 wx helper mx "done" "" "$(date -Is -d '2 hours ago')" 2
  mk_task N2 wx helper mx "done" "" "$(date -Is -d '1 hour ago')" 3
  agent debug --agent wx --days 30 --last 1 --out "$BATS_TEST_TMPDIR/d.txt" > /dev/null
  f=$BATS_TEST_TMPDIR/d.txt
  grep -q "^tasks: 2" "$f"
  ! grep -q "old failure" "$f" || false
  grep -q "^== task N2" "$f"
  ! grep -q "^== task N1" "$f" || false
}

@test "agent debug --agent: results the requesters rejected, an approximation from their messages" {
  now=$(date -Is)
  mk_task A2 wx helper mx "done" "" "$now" 2
  mkdir -p "$AGENTS_HOME/bus"
  jq -nc '{ts: "2026-11-07 10:00:00", from: "codex-1", to: "team:T", text: "H2 rejected: it counted the wrong files"}' >> "$AGENTS_HOME/bus/messages.jsonl"
  agent debug --agent wx --out "$BATS_TEST_TMPDIR/d.txt" > /dev/null
  grep -q "rejected (an approximation" "$BATS_TEST_TMPDIR/d.txt"
  grep -q "H2 rejected: it counted the wrong files" "$BATS_TEST_TMPDIR/d.txt"
}

@test "agent spend: a task without an activity file of its own is no jq error" {
  mkdir -p "$AGENTS_HOME/tasks/IT"
  printf 'ID=IT\nKIND=investigation_team\nCREATED=%s\n' "$(date -Is)" > "$AGENTS_HOME/tasks/IT/meta.env"
  run agent spend
  [ "$status" -eq 0 ]
  [[ $output != *"jq: error"* ]]
}
