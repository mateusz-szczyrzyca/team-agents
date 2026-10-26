#!/usr/bin/env bats
# "done" is a status name passed as an argument (wait_status "$t" done), not the end of a loop.
# shellcheck disable=SC1010
# What the coordinator's failure on 2026-10-26 showed: a model "at capacity" is a short pause, not a failure; a
# coordinator's failed run does not close its team; resuming never throws away a task's uncommitted changes.

load helpers

# meta id key — a value from a task's meta.env
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "a model at capacity (the provider overloaded) is a short pause, not a failure" {
  printf 'AGENT_RATE_LIMIT_PAUSE_MIN=4\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "capacity"}}'
  before=$(date +%s)
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" paused
  [ "$(meta "$t" PAUSE_REASON)" = overloaded ]
  wake=$(meta "$t" WAKE_AT)
  (( wake >= before + 4 * 60 + 60 && wake <= before + 4 * 60 + 60 + 30 ))
  wait_grep "overloaded" "$HOME/notifications.log"
}

@test "a coordinator's failed run does not close the team; the members hear that it is out" {
  scenario '{"lead": {"new": "team-lead-crash"}, "member": {"new": "team-sleep"}}'
  agent run --team lead,member "team task"
  l=$(task_of lead)
  wait_status "$l" failed
  team=$(meta "$l" TEAM)
  wait_grep 'COORDINATOR lead IS OUT' "$AGENTS_HOME/bus/messages.jsonl"
  [ "$(agentbus team-get --dir "$AGENTS_HOME/bus" --team "$team" | jq -r '.done')" = false ]
  [ "$(agentbus team-get --dir "$AGENTS_HOME/bus" --team "$team" | jq -r '.members.member.state')" != done ]
}

@test "resuming carries a task's uncommitted changes into its new worktree" {
  scenario '{"solo": {"new": "done", "resume": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  old=$(meta "$t" WORKTREE)
  echo "unfinished work" > "$old/wip.txt"
  echo "changed" >> "$old/README.md"
  agent run --resume "$t" "go on" < /dev/null
  t2=$(ls -d "$AGENTS_HOME"/tasks/*-solo | tail -1); t2=${t2##*/}
  [ "$t2" != "$t" ]
  new=$(meta "$t2" WORKTREE)
  [ "$(cat "$new/wip.txt")" = "unfinished work" ]
  tail -1 "$new/README.md" | grep -qx changed
  [ -z "$(git -C "$HOME/repo" stash list)" ]
}
