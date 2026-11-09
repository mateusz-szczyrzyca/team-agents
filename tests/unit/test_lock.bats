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
    out=$(cd "$REPO_ROOT" && make -n "$target" 2> /dev/null)
    [[ $out == *"tools/test-lock.sh $target "* ]] || { echo "target $target has no test lock: $out"; false; }
  done
}

@test "a test recipe copied into a repository without the lock (the harness's fixtures) still runs, unlocked" {
  # the harness copies the unit-agent-bats recipe alone into a fixture repository, which has no tools/test-lock.sh
  d=$BATS_TEST_TMPDIR/fixture
  mkdir -p "$d/tools"
  printf '#!/bin/sh\necho agent-bats ran\n' > "$d/tools/agent-bats.sh"
  printf '#!/bin/sh\necho parity ran\n' > "$d/tools/check-agent-parity.sh"
  chmod +x "$d/tools/agent-bats.sh" "$d/tools/check-agent-parity.sh"
  printf 'build-agent:\n\t@:\n' > "$d/Makefile"
  awk '/^unit-agent-bats:/{copy=1} copy && /^$/{exit} copy{print}' "$REPO_ROOT/Makefile" >> "$d/Makefile"
  run make -C "$d" --no-print-directory unit-agent-bats
  [ "$status" -eq 0 ]
  [[ $output == *"agent-bats ran"* && $output == *"parity ran"* ]]
}

@test "every agent's prompt knows the busy message and what to do, and AGENTS.md describes the lock" {
  grep -q "another tests are running now - please wait before you try" "$REPO_ROOT/prompts/protocol.md"
  grep -q "Never get around it" "$REPO_ROOT/prompts/protocol.md"
  grep -q "GATE_WAIT=1 make gate" "$REPO_ROOT/prompts/protocol.md"
  grep -q "tools/test-lock.sh" "$REPO_ROOT/AGENTS.md"
}

@test "the test command reads /dev/null, whatever stdin the lock got: in its own process group a terminal read stops it" {
  # make gate runs the tests in their own process group (set -m): from an interactive terminal, a test that read the
  # terminal would be stopped (SIGTTIN) — and hang the gate — instead of reading nothing
  printf 'from the caller\n' > "$BATS_TEST_TMPDIR/stdin"
  run bash -c '"$1" stdin readlink /proc/self/fd/0 < "$2"' _ "$LOCK" "$BATS_TEST_TMPDIR/stdin"
  [ "$status" -eq 0 ]
  [ "$output" = /dev/null ]
}
