#!/usr/bin/env bats
# A run interrupted by a restart of its machine (its boot ID changed) is not a dead runner: the task is resumed,
# told what happened, up to AGENT_RESTART_MAX restarts; a remote team member waits for its lead to wake it.

load helpers

# meta task key — a field of a task (as in verify_loop.bats)
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# restart task — the machine restarts under a running task: its run dies at once (nothing written on the way out),
# and the machine comes back with a new boot ID
restart() {
  local run p
  run=$(meta "$1" RUN)
  p=$(cat "$STUB_UNITS/agent-$1-r$run.service.pid")
  kill -KILL -- -"$p"
  for _ in $(seq 1 50); do
    kill -0 "$p" 2> /dev/null || break   # "no such process" is the answer waited for
    sleep 0.1
  done
  echo "boot-$RANDOM" > "$AGENT_BOOT_ID_FILE"
}

# first_boot — the machine's boot ID, in a file the script reads instead of /proc (not setup(): helpers' setup runs)
first_boot() {
  export AGENT_BOOT_ID_FILE="$BATS_TEST_TMPDIR/boot_id"
  echo "boot-first" > "$AGENT_BOOT_ID_FILE"
}

@test "a run interrupted by a restart of its machine is resumed, and the agent is told what happened" {
  first_boot
  scenario '{"solo": {"new": "hang", "resume": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" running
  restart "$t"
  AGENT_SUPERVISE_GRACE_SEC=0 agent _supervise
  wait_status "$t" 'done'
  [ "$(meta "$t" RESTARTS)" = 1 ]
  grep -qi 'restart' "$AGENTS_HOME/tasks/$t/prompt-2.md"
}

@test "a task interrupted by more restarts than AGENT_RESTART_MAX fails, with the reason" {
  first_boot
  scenario '{"solo": {"new": "hang", "resume": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" running
  echo "RESTARTS=2" >> "$AGENTS_HOME/tasks/$t/meta.env"
  restart "$t"
  AGENT_SUPERVISE_GRACE_SEC=0 AGENT_RESTART_MAX=2 agent _supervise
  [ "$(meta "$t" STATUS)" = failed ]
  # meta.env values are shell-quoted: read the note as the script does
  # shellcheck source=/dev/null
  (source "$AGENTS_HOME/tasks/$t/meta.env" && [[ $NOTE == *"restarted 3 times"* ]])
}

@test "a remote team member interrupted by a restart waits for its lead; the lead's wake resumes it" {
  first_boot
  scenario '{"solo": {"new": "hang", "resume": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" running
  printf 'TEAM=T\nMEMBER=solo@node2\nREMOTE_TEAM=1\nJOB_LEAD=http://127.0.0.1:9\n' >> "$AGENTS_HOME/tasks/$t/meta.env"
  restart "$t"
  AGENT_SUPERVISE_GRACE_SEC=0 agent _supervise
  [ "$(meta "$t" STATUS)" = interrupted ]
  [ "$(meta "$t" RUN)" = 1 ]          # not resumed by itself: its lead decides
  agent _wake "$t"
  wait_status "$t" 'done'
  grep -qi 'restart' "$AGENTS_HOME/tasks/$t/prompt-2.md"
}
