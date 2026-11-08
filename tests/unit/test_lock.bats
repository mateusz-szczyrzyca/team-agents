#!/usr/bin/env bats
# Agents ran make gate at the same time, in their worktrees, and their runs raced: one passed, another failed on the
# other's load or files. Every Makefile target that runs tests now goes through tools/test-lock.sh: one test run at a
# time on the machine (a lock file, /tmp/.make.command.lck by default). A busy lock refuses at once with a message
# every agent knows (exit 75); a holder that died frees it at once; a holder alive past TEST_LOCK_STALE_MIN minutes is
# taken for hung — its process group is killed and the lock taken over.

load helpers

setup() {
  unset TEST_LOCK_HELD TEST_LOCK_WAIT GATE_WAIT   # the gate running these tests holds the real lock: not ours
  export TEST_LOCK_FILE="$BATS_TEST_TMPDIR/test.lck"
  LOCK="$REPO_ROOT/tools/test-lock.sh"
}

# hold seconds — a test run that holds the lock that long, in the background; sets HOLDER (its pid)
hold() {
  "$LOCK" holder sleep "$1" &
  HOLDER=$!
  for _ in $(seq 1 50); do grep -qs '^pid=' "$TEST_LOCK_FILE" && return 0; sleep 0.1; done
  return 1
}

@test "a free lock: the command runs, its exit status comes through, and the lock is free again after it" {
  run "$LOCK" first bash -c 'echo ran; exit 3'
  [ "$status" -eq 3 ]
  [ "$output" = ran ]
  run "$LOCK" second true
  [ "$status" -eq 0 ]
}

@test "a busy lock refuses at once with the message every agent knows, exit 75, and says who holds it" {
  hold 30
  run "$LOCK" mine echo ran
  [ "$status" -eq 75 ]
  [[ $output == *"another tests are running now - please wait before you try again"* ]]
  [[ $output == *"holder"* ]]
  [[ $output != *ran* ]]
  kill "$HOLDER"
}

@test "a holder that died frees the lock at once: no wait for the limit" {
  hold 30
  child=$(pgrep -P "$HOLDER" | head -n 1)
  kill -KILL -- "-$child"                   # the whole test run dies, and the wrapper with it
  kill -KILL "$HOLDER" 2> /dev/null || true  # (it may have ended already, its command gone)
  wait "$HOLDER" 2> /dev/null || true
  run "$LOCK" after echo ran
  [ "$status" -eq 0 ]
  [ "$output" = ran ]
}

@test "a holder alive past TEST_LOCK_STALE_MIN is taken for hung: its process group is killed, the lock taken over" {
  hold 300
  child=$(pgrep -P "$HOLDER" | head -n 1)
  TEST_LOCK_STALE_MIN=0 run "$LOCK" rescuer echo ran
  [ "$status" -eq 0 ]
  [[ $output == *"ran"* ]]
  [[ $output == *"hung"* ]]
  ! kill -0 "$child" 2> /dev/null || false   # the hung test run is gone
}

@test "a test run inside a test run (make gate's own steps, the tests of the gate) does not lock again" {
  hold 30
  TEST_LOCK_HELD=1 run "$LOCK" nested echo ran
  [ "$status" -eq 0 ]
  [ "$output" = ran ]
  kill "$HOLDER"
}

@test "TEST_LOCK_WAIT=1 (or GATE_WAIT=1) waits for the holder instead of refusing" {
  hold 2
  GATE_WAIT=1 run "$LOCK" waiter echo ran
  [ "$status" -eq 0 ]
  [[ $output == *"ran"* ]]
}

@test "an interrupted test run stops its commands and frees the lock" {
  hold 300
  child=$(pgrep -P "$HOLDER" | head -n 1)
  kill -TERM "$HOLDER"
  wait "$HOLDER" 2> /dev/null || true
  ! kill -0 "$child" 2> /dev/null || false
  run "$LOCK" after echo ran
  [ "$status" -eq 0 ]
}

@test "every Makefile target that runs tests goes through tools/test-lock.sh" {
  for target in gate test unit-go unit-bats harness unit-agent-bats; do
    recipe=$(awk -v t="$target:" '$1 == t { on = 1; next } on && /^\t/ { print; next } on { exit }' "$REPO_ROOT/Makefile")
    [[ $recipe == *tools/test-lock.sh* ]] || { echo "target $target has no test lock: $recipe"; false; }
  done
}

@test "every agent's prompt knows the busy message and what to do, and AGENTS.md describes the lock" {
  grep -q "another tests are running now - please wait before you try" "$REPO_ROOT/prompts/protocol.md"
  grep -q "Never get around it" "$REPO_ROOT/prompts/protocol.md"
  grep -q "GATE_WAIT=1 make gate" "$REPO_ROOT/prompts/protocol.md"
  grep -q "tools/test-lock.sh" "$REPO_ROOT/AGENTS.md"
}
