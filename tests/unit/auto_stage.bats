#!/usr/bin/env bats
# What --verify and --autopilot do for a round shows on its coordinator's task (AUTO_STAGE, the dashboard's
# stage): the checks before the review with the gate's progress, the reviews, the merge — and nothing once the
# work is back with the team or the human has it.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

leads() { local d; for d in "$AGENTS_HOME"/tasks/*-lead/; do d=${d%/}; echo "${d##*/}"; done; }
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
# stage task — the task's AUTO_STAGE as the script reads it (meta.env values are shell-quoted)
# shellcheck source=/dev/null
stage() { (source "$AGENTS_HOME/tasks/$1/meta.env" 2> /dev/null && printf '%s' "${AUTO_STAGE:-}"); }

@test "--autopilot: the coordinator's task shows the checks with the gate's progress, then the two reviews, then nothing once the next round starts" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN", "isDraft": false}]' > "$HOME/gh-prs.json"
  cat > "$BATS_TEST_TMPDIR/gate.sh" <<'GATE'
#!/usr/bin/env bash
echo "gate: 3 steps, full logs in build/gate/"
sleep 2; echo "ok    one                 1s"
sleep 4; echo "ok    two                 1s"
sleep 1; echo "ok    three               1s"
echo "gate: all 3 steps passed"
GATE
  chmod 755 "$BATS_TEST_TMPDIR/gate.sh"
  printf 'INVESTIGATORS_TYPE[inv2]=claude\nAGENT_REVIEW_GATE=%q\nAGENT_STAGE_POLL_SEC=1\n' "$BATS_TEST_TMPDIR/gate.sh" >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-round:1/2", "resume": "team-lead-wait"}, "member": {"new": "team-sleep", "resume": "team-sleep"}, "inv": {"new": "review-ready"}, "inv2": {"new": "review-ready-slow"}}'
  agent run --team lead,member --autopilot "a task in two rounds"
  for _ in $(seq 1 100); do [ -n "$(leads)" ] && break; sleep 0.1; done
  lead=$(leads | head -n 1)
  : > "$BATS_TEST_TMPDIR/stages"
  ( while :; do s=$(stage "$lead"); [[ $s == "$prev" ]] || { echo "[$s]" >> "$BATS_TEST_TMPDIR/stages"; prev=$s; }; sleep 0.2; done ) &
  poller=$!
  wait_grep "round 2/2 starts" "$HOME/notifications.log" 150
  for _ in $(seq 1 100); do [ -z "$(stage "$lead")" ] && break; sleep 0.2; done
  kill "$poller"
  cat "$BATS_TEST_TMPDIR/stages" >&3
  grep -q '^\[checks before review: .*gate.sh — 1 of 3 steps passed (one)\]$' "$BATS_TEST_TMPDIR/stages"
  grep -q '^\[two reviews: inv.* (#[0-9]*), inv.* (#[0-9]*)\]$' "$BATS_TEST_TMPDIR/stages"
  [ -z "$(stage "$lead")" ]   # the next round has the work: nothing left on this task
  second=$(leads | sed -n 2p)
  for _ in $(seq 1 300); do grep -q '^STATUS=' "$AGENTS_HOME/tasks/$second/meta.env" 2> /dev/null && break; sleep 0.2; done   # not there yet: polled again
  wait_status "$second" 'starting|running|sleeping|done' 120
  agent team "$(meta "$second" TEAM)" --close > /dev/null   # the next round's work is not part of this test
  sleep 3
}

@test "a local error of the machine is neither a provider's limit nor a network failure; the provider's own are" {
  source "$REPO_ROOT/bin/agent"
  d=$BATS_TEST_TMPDIR/t
  mkdir -p "$d"
  for line in 'bash: fork: retry: Resource temporarily unavailable' \
              'read unix @->/run/user/1000/agentbus.sock: i/o timeout' \
              'dial unix /tmp/agent.sock: connect: connection refused'; do
    printf '%s\n' "$line" > "$d/err-1.log"
    run limit_kind "$d" 1 ""
    [ "$status" -eq 1 ] || { echo "taken for a limit ($output): $line"; false; }
  done
  printf 'Error: 503 Service Temporarily Unavailable\n' > "$d/err-1.log"
  [ "$(limit_kind "$d" 1 "")" = overloaded ]
  printf 'Post "https://api.example.com/v1/chat": dial tcp 203.0.113.7:443: i/o timeout\n' > "$d/err-1.log"
  [ "$(limit_kind "$d" 1 "")" = network ]
  printf 'read tcp 10.0.0.2:55512->203.0.113.7:443: read: connection reset by peer\n' > "$d/err-1.log"
  [ "$(limit_kind "$d" 1 "")" = network ]
}
