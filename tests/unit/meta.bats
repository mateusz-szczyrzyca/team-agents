#!/usr/bin/env bats
# meta.env is written by several processes at once — a run, `agent stop`, the bus's wake-ups. Concurrent writes
# of different keys must all survive, and none may fail.

load helpers

@test "concurrent meta_set calls of different keys all survive, and none fails" {
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/racer"
  echo 'STATUS=running' > "$TASKS/racer/meta.env"
  for i in $(seq 1 20); do
    ( meta_set racer "K$i" "v$i" || echo "writer $i failed" >> "$HOME/failures" ) &
  done
  wait
  [ ! -s "$HOME/failures" ]
  for i in $(seq 1 20); do
    grep -qx "K$i=v$i" "$TASKS/racer/meta.env"
  done
  grep -qx 'STATUS=running' "$TASKS/racer/meta.env"
}
