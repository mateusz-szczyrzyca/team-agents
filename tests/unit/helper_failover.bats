#!/usr/bin/env bats
# A worker's model was overloaded at the provider for 8 hours: its request paused and resumed 76 times, every six
# minutes, and the member waiting for its result waited with it — the failover of an overloaded model was a team
# member's only. A worker request whose model is out of service past failover_due now goes to another free worker: the
# worker's task is stopped without a result for the requester, the request queued again and dispatched; and
# free_worker passes over a worker out of service while its pause lasts.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "a worker request whose model is out of service goes to another worker; the requester hears no failure" {
  export AGENT_FAILOVER_MIN=0
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "idle"}, "worker": {"new": "capacity"}, "worker2": {"new": "capacity"}}'
  agent run --team lead,member "team task" > /dev/null
  m=$(task_of member); team=$(meta "$m" TEAM)
  wait_status "$m" sleeping
  "$BUILD_DIR/agentbus" delegate --dir "$AGENTS_HOME/bus" --team "$team" --as member --task "$m" --repo repo \
    --branch "$(meta "$m" BRANCH)" --worktree "$(meta "$m" WORKTREE)" --agent-cmd agent --home "$AGENTS_HOME" \
    --request "count the files" --context "c" > /dev/null
  first=""
  for _ in $(seq 1 150); do
    first=$(grep -lx 'KIND=helper' "$AGENTS_HOME"/tasks/*/meta.env 2> /dev/null | head -n 1 | xargs -r dirname | xargs -r basename)
    [ -n "$first" ] && [ "$(meta "$first" FAILOVER_PENDING)" = 1 ] && break
    sleep 0.2
  done
  [ "$(meta "$first" FAILOVER_PENDING)" = 1 ]
  w1=$(meta "$first" AGENT)
  agent _supervise
  [ "$(meta "$first" STATUS)" = stopped ]
  wait_grep '"event":"helper_failover"' "$AGENTS_HOME/logs/agent.jsonl"
  second=""
  for _ in $(seq 1 150); do
    second=$(grep -lx 'KIND=helper' "$AGENTS_HOME"/tasks/*/meta.env | xargs -n1 dirname | xargs -n1 basename | grep -vx "$first" | head -n 1)
    [ -n "$second" ] && break
    sleep 0.2
  done
  [ -n "$second" ]
  [ "$(meta "$second" AGENT)" != "$w1" ]
  [ "$(meta "$second" HELPER_ID)" = "$(meta "$first" HELPER_ID)" ]
  ! grep -q "RESULT H$(meta "$first" HELPER_ID) \[failed\]" "$AGENTS_HOME/logs/agent.jsonl" || false
  agent stop --running > /dev/null
}

@test "free_worker passes over a worker whose model is out of service while its pause lasts" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # read by free_worker
  WORKER_AGENTS=(worker worker2)
  agent_running_task() { return 1; }
  mkdir -p "$TASKS/W1"
  printf 'ID=W1\nAGENT=worker\nKIND=helper\nSTATUS=stopped\nFAILOVER_UNTIL=%s\n' "$(( $(date +%s) + 600 ))" > "$TASKS/W1/meta.env"
  for _ in $(seq 1 20); do [ "$(free_worker)" = worker2 ]; done
  meta_set W1 FAILOVER_UNTIL "$(( $(date +%s) - 1 ))"   # its pause over: it may work again
  declare -A seen=()
  for _ in $(seq 1 40); do seen[$(free_worker)]=1; done
  [ -n "${seen[worker]:-}" ]
}
