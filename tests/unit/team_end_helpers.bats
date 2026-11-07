#!/usr/bin/env bats
# The coordinator announced the team done, and four workers went on with requests nobody waited for any more — and
# queued ones could still start. A team that finishes, however it does (finish_team), now ends its worker requests:
# those under way are stopped, those under way and queued are closed as failed, and no worker starts on them.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
helpers() { grep -lx 'KIND=helper' "$AGENTS_HOME"/tasks/*/meta.env 2> /dev/null | xargs -r -n1 dirname | xargs -r -n1 basename; }

@test "a team whose coordinator announces it done stops its workers' tasks and closes its queued requests" {
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"   # team-sleep claims the role too
  scenario '{"lead": {"new": "team-lead-wait", "resume": "team-lead-done"}, "member": {"new": "team-sleep"}, "worker": {"new": "sleep:300"}, "worker2": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member); team=$(meta "$t" TEAM)
  wait_status "$t" sleeping
  wait_status "$m" sleeping
  for r in one two three; do
    "$BUILD_DIR/agentbus" delegate --dir "$AGENTS_HOME/bus" --team "$team" --as member --task "$m" --repo repo \
      --branch "$(meta "$m" BRANCH)" --worktree "$(meta "$m" WORKTREE)" --agent-cmd agent --home "$AGENTS_HOME" \
      --request "count $r" --context "c" > /dev/null
  done
  for _ in $(seq 1 150); do [ "$(helpers | wc -l)" -ge 2 ] && break; sleep 0.2; done
  for h in $(helpers); do wait_status "$h" running; done
  started=$(helpers | wc -l)
  "$BUILD_DIR/agentbus" send --dir "$AGENTS_HOME/bus" --from human --to lead "finish the team now" > /dev/null
  wait_status "$t" 'done'
  for h in $(helpers); do wait_status "$h" stopped; done
  for _ in $(seq 1 100); do
    "$BUILD_DIR/agentbus" helper-list --dir "$AGENTS_HOME/bus" --all | grep -qE '^H[0-9]+ \[(queued|starting|running)' || break
    sleep 0.2
  done
  ! "$BUILD_DIR/agentbus" helper-list --dir "$AGENTS_HOME/bus" --all | grep -E '^H[0-9]+ \[(queued|starting|running)' || false
  [ "$(grep -c '"event":"team_helper_ended"' "$AGENTS_HOME/logs/agent.jsonl")" -eq 3 ]
  sleep 2
  [ "$(helpers | wc -l)" -eq "$started" ]   # the queued request never started
}
