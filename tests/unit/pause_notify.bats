#!/usr/bin/env bats
# A remote member's pause reaches its team as a local member's does; notifications that cannot be delivered are kept
# for a day with the process that sent them (the dashboard warns); an answers watch is stopped once its task no
# longer waits for answers.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
# member_state team member — the member's state on the bus
member_state() { agentbus team-get --dir "$AGENTS_HOME/bus" --team "$1" | jq -r --arg m "$2" '.members[$m].state'; }

@test "a remote member's pause reaches its team: the coordinator hears why and until when; back at work it is active again" {
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  wait_status "$m" sleeping 60
  team=$(meta "$m" TEAM)
  sed -i '/^KIND=/d' "$AGENTS_HOME/tasks/$m/meta.env"
  echo 'KIND=remote-member' >> "$AGENTS_HOME/tasks/$m/meta.env"   # as the lead's proxy task of a remote member
  until=$(date -d '+3 days' '+%d.%m %H:%M')
  agent _remote_member "$m" paused "" "00:44:40 ■ end of run 6: paused (the provider's quota or usage limit is exhausted — resumes at $until — error: Individual quota reached.)"
  [ "$(meta "$m" PAUSE_REASON)" = quota ]
  [ -n "$(meta "$m" WAKE_AT)" ]
  grep '"notice":"member_paused"' "$AGENTS_HOME/bus/messages.jsonl" | grep -q "\"until\":\"$until\""
  grep '"notice":"member_paused"' "$AGENTS_HOME/bus/messages.jsonl" | grep -q '"reason":"quota"'
  [ "$(member_state "$team" member)" = paused ]
  agent _remote_member "$m" running ""
  [ "$(member_state "$team" member)" = active ]
  agent team "$team" --close > /dev/null
}

@test "notifications that cannot be delivered are kept for a day, with the process that sent them, whatever is delivered since" {
  cat > "$BATS_TEST_TMPDIR/ntfy.sh" <<'NTFY'
#!/usr/bin/env bash
echo "./ntfy.sh: line 11: NTFY_URL: set NTFY_URL" >&2
exit 1
NTFY
  chmod 755 "$BATS_TEST_TMPDIR/ntfy.sh"
  printf 'NOTIFY_CMD=%q\n' "$BATS_TEST_TMPDIR/ntfy.sh" >> "$AGENTS_HOME/agents.conf"
  f=$AGENTS_HOME/notify-failures.jsonl
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task"
  wait_status "$(task_of solo)" 'done'
  for _ in $(seq 1 50); do [ -s "$f" ] && break; sleep 0.1; done
  jq -se 'length >= 1 and all(.[]; (.error | test("NTFY_URL")) and .from == "_runner" and (.command | test("ntfy.sh")))' "$f"
  n=$(wc -l < "$f")
  sed -i 's|^NOTIFY_CMD=.*|NOTIFY_CMD=true|' "$AGENTS_HOME/agents.conf"   # delivered from now on, by this process
  agent run solo "another task"
  wait_status "$(task_of solo)" 'done'
  [ "$(wc -l < "$f")" -eq "$n" ]   # a delivery elsewhere does not hide the failures
  # older than a day: left out with the next failure
  jq -nc --argjson e "$(( $(date +%s) - 90000 ))" '{at: "old", epoch: $e, from: "_merge", command: "x", error: "old"}' >> "$f"
  printf 'NOTIFY_CMD=%q\n' "$BATS_TEST_TMPDIR/ntfy.sh" >> "$AGENTS_HOME/agents.conf"
  agent run solo "a third task"
  wait_status "$(task_of solo)" 'done'
  for _ in $(seq 1 50); do [ "$(wc -l < "$f")" -gt "$n" ] && ! grep -q '"error":"old"' "$f" && break; sleep 0.1; done
  run ! grep -q '"error":"old"' "$f"
  [ "$(wc -l < "$f")" -gt "$n" ]
}

@test "the supervisor stops an answers watch whose task no longer waits for answers, and keeps a waiting task's" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  touch "$STUB_UNITS/agent-$t-answers-1.path"   # left from its questions, answered another way
  scenario '{"solo": {"new": "questions-file"}}'
  agent run solo "a task with questions"
  w=$(task_of solo)
  wait_status "$w" waiting
  for _ in $(seq 1 50); do compgen -G "$STUB_UNITS/agent-$w-answers-*" > /dev/null && break; sleep 0.1; done
  compgen -G "$STUB_UNITS/agent-$w-answers-*" > /dev/null
  AGENT_SUPERVISE_GRACE_SEC=0 agent _supervise
  [ ! -e "$STUB_UNITS/agent-$t-answers-1.path" ]
  compgen -G "$STUB_UNITS/agent-$w-answers-*" > /dev/null   # it still waits: its watch stays
}
