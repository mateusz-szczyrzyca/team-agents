#!/usr/bin/env bats
# A team's coordinator, its model "at capacity" at the provider again and again, held its team back for an hour at
# night. A member of an --autopilot team whose model is out of service — overloaded long or often, or out of quota —
# is now replaced: its work in progress committed and pushed on its branch, its task stopped, and the round handed
# back to the team with a free normal agent in its place; a coordinator replaced, the team gets a new one. And a
# coordinator is told never to wait in a loop for its members.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# team_of task — the team's tasks' agents, sorted
team_agents() { grep -lx "TEAM=$1" "$AGENTS_HOME"/tasks/*/meta.env | xargs -n1 sed -n 's/^AGENT=//p' | sort | paste -sd' '; }

@test "failover_due: an exhausted quota, an overload past AGENT_FAILOVER_MIN or the third in an hour; not a short one, nor a rate limit" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/F"
  printf 'ID=F\n' > "$TASKS/F/meta.env"
  export AGENT_FAILOVER_MIN=15 AGENT_FAILOVER_OVERLOADS=3
  failover_due F quota 60
  failover_due F overloaded 1200
  run failover_due F rate_limit 7200
  [ "$status" -ne 0 ]
  meta_set F OVERLOAD_TIMES ""
  run failover_due F overloaded 300
  [ "$status" -ne 0 ]
  run failover_due F overloaded 300
  [ "$status" -ne 0 ]
  failover_due F overloaded 300   # the third short overload within the hour
}

@test "a member whose model is out of service is replaced by a free normal agent; its work in progress is pushed on its branch" {
  export AGENT_FAILOVER_MIN=0
  scenario '{"lead": {"new": "done"}, "member": {"new": "capacity"}, "worker": {"new": "done"}}'
  agent run --team lead,member,worker --autopilot "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member); team=$(meta "$t" TEAM)
  wait_status "$m" paused
  wait_status "$t" 'done'
  [ "$(meta "$m" FAILOVER_PENDING)" = 1 ]
  echo "half done" > "$(meta "$m" WORKTREE)/wip.txt"
  agent _supervise
  [ "$(meta "$m" STATUS)" = stopped ]
  git -C "$HOME/remote.git" log --format=%s "$(meta "$m" BRANCH)" | grep -q '^wip: member hands its work over'
  s=$(task_of solo)
  [ -n "$s" ]
  new=$(meta "$s" TEAM)
  [ "$new" != "$team" ]
  [ "$(team_agents "$new")" = "lead solo worker" ]
  grep -q '"event":"failover_done"' "$AGENTS_HOME/logs/agent.jsonl"
  wait_grep "member is replaced by solo" "$HOME/notifications.log"
  agent stop --running > /dev/null
}

@test "a coordinator whose model is out of service is replaced: the new team has a new coordinator" {
  export AGENT_FAILOVER_MIN=0
  scenario '{"lead": {"new": "capacity"}, "member": {"new": "done"}, "worker": {"new": "done"}}'
  agent run --team lead,member,worker --autopilot "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member)
  wait_status "$t" paused
  wait_status "$m" 'done'
  agent _supervise
  [ "$(meta "$t" STATUS)" = stopped ]
  s=$(task_of solo)
  [ -n "$s" ]
  [ "$(team_agents "$(meta "$s" TEAM)")" = "member solo worker" ]
  [ "$(task_of lead)" = "$t" ]   # no new task for the replaced coordinator
  agent stop --running > /dev/null
}

@test "without a free normal agent the member waits its pause out, and the human hears of it" {
  export AGENT_FAILOVER_MIN=0
  scenario '{"solo": {"new": "sleep:300"}, "lead": {"new": "done"}, "member": {"new": "capacity"}, "worker": {"new": "done"}}'
  agent run solo "busy elsewhere" > /dev/null
  wait_status "$(task_of solo)" running
  agent run --team lead,member,worker --autopilot "team task" > /dev/null
  m=$(task_of member)
  wait_status "$m" paused
  agent _supervise
  [ "$(meta "$m" STATUS)" = paused ]
  grep -q '"event":"failover_no_agent"' "$AGENTS_HOME/logs/agent.jsonl"
  wait_grep "no free normal agent" "$HOME/notifications.log"
  agent stop --running > /dev/null
}

@test "a coordinator's prompt says never to wait in a loop for its members" {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead)
  p=$AGENTS_HOME/tasks/$t/prompt-1.md
  for _ in $(seq 1 300); do grep -q "The coordinator" "$p" 2> /dev/null && break; sleep 0.2; done
  grep -q "Never wait in a loop for a member" "$p"
  agent stop --running > /dev/null
}
