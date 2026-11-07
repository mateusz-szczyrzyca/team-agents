#!/usr/bin/env bats
# Diagnosing a team took hand-written scripts, and two of them were wrong (a loop zsh did not split, a log filter
# that missed most records). agent debug <team|task> writes one file with everything a diagnosis needs: the team on
# the bus, its board, its worker requests, every task's metadata, activity, errors, final report and checks, the
# script's log, the bus messages, the branches, the round's PR, its card and tokens — read-only, with tokens and the
# values of secrets.env redacted.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# open_team — lead coordinates, member sleeps; the team open, in $team
open_team() {
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member); team=$(meta "$t" TEAM)
  wait_status "$t" sleeping
  wait_status "$m" sleeping
}

@test "agent debug <team> writes one file with the team on the bus, its board, every task, the log and the branches" {
  open_team
  "$BUILD_DIR/agentbus" board-add --dir "$AGENTS_HOME/bus" --team "$team" --as lead --title "split the parser" --details "d" --level normal > /dev/null
  run agent debug "$team"
  [ "$status" -eq 0 ]
  f=$(awk '{print $1; exit}' <<< "$output")
  [ -f "$f" ]
  for s in "team on the bus" "board" "worker requests" "task $t" "task $m" "the script's log" "bus messages" "git" "round"; do
    grep -q "^== $s" "$f" || { echo "no section: $s"; false; }
  done
  grep -q "split the parser" "$f"
  grep -q '"lead": "lead"' "$f"
  grep -q "end of run 1: sleeping" "$f"
  agent stop --running > /dev/null
}

@test "agent debug redacts tokens and the values of secrets.env" {
  printf 'GH_TOKEN=s3cr3t-value-123\nOPENROUTER_API_KEY="another-secret-456"\n' > "$AGENTS_HOME/secrets.env"
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task" > /dev/null
  s=$(task_of solo)
  wait_status "$s" 'done'
  jq -nc '{ts: "2026-11-07 10:00:00", k: "text", t: "pushed with ghp_abcdefghijklmnopqrstuvwxyz0123456789 and s3cr3t-value-123"}' >> "$AGENTS_HOME/tasks/$s/activity.jsonl"
  echo "auth failed: another-secret-456 sk-abcdefghijklmnopqrstuvwx" > "$AGENTS_HOME/tasks/$s/err-1.log"
  run agent debug "$s"
  [ "$status" -eq 0 ]
  f=$(awk '{print $1; exit}' <<< "$output")
  ! grep -qE 'ghp_abcdef|s3cr3t-value-123|another-secret-456|sk-abcdefghij' "$f" || false
  grep -q '\[redacted\]' "$f"
}

@test "agent debug of a task: the last activity by default, all of it with --full, in the file --out names" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task" > /dev/null
  s=$(task_of solo)
  wait_status "$s" 'done'
  for i in $(seq 1 300); do jq -nc --arg i "$i" '{ts: "2026-11-07 10:00:00", k: "cmd", t: ("step-" + $i)}'; done >> "$AGENTS_HOME/tasks/$s/activity.jsonl"
  agent debug "$s" --out "$BATS_TEST_TMPDIR/short.txt" > /dev/null
  agent debug "$s" --full --out "$BATS_TEST_TMPDIR/full.txt" > /dev/null
  grep -q "step-300" "$BATS_TEST_TMPDIR/short.txt"
  ! grep -q "step-1$" "$BATS_TEST_TMPDIR/short.txt" || false
  grep -q "step-1$" "$BATS_TEST_TMPDIR/full.txt"
  grep -q "^== task $s" "$BATS_TEST_TMPDIR/full.txt"
}

@test "agent debug refuses an ID that is no team and no task" {
  run agent debug no-such-team
  [ "$status" -ne 0 ]
  [[ $output == *"no-such-team"* ]]
}
