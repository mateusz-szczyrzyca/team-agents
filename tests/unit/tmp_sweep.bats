#!/usr/bin/env bats
# The Bats runs filled /tmp, a tmpfs: what runs stopped hard had left there (agent-bats.sh's staged copies, the runs'
# directories) took memory, and GNU parallel could not write its buffers ("Is the disk full?") with room left — no
# test of the parallel group ran. The runs now keep their temporary files on disk (BATS_TMP), sweep what earlier
# runs left, and refuse to start without room.

load helpers

@test "tmp-sweep removes the old leftovers of test runs that no process uses; it keeps fresh ones, those in use, and anything else" {
  d=$BATS_TEST_TMPDIR/sweep
  mkdir -p "$d"/{staged-oracle-AAAAAA,staged-agent-EEEEEE,agent-bats-reports-BBBBBB,bats-CCCCCC,bats-run-DDDDDD,bats-FFFFFF,bats-GGGGGG,other-dir}
  touch -d '2 days ago' "$d"/{staged-oracle-AAAAAA,staged-agent-EEEEEE,agent-bats-reports-BBBBBB,bats-CCCCCC,bats-run-DDDDDD,bats-GGGGGG,other-dir}
  bash -c "exec -a 'in $d/bats-GGGGGG/x' sleep 30" &   # a run still at work in bats-GGGGGG
  pid=$!
  sleep 0.5
  "$REPO_ROOT/tools/tmp-sweep.sh" "$d"
  kill "$pid"
  for x in staged-oracle-AAAAAA staged-agent-EEEEEE agent-bats-reports-BBBBBB bats-CCCCCC bats-run-DDDDDD; do
    [ ! -e "$d/$x" ]
  done
  [ -d "$d/bats-FFFFFF" ]   # fresh
  [ -d "$d/bats-GGGGGG" ]   # in use
  [ -d "$d/other-dir" ]     # not a test run's
}

@test "make unit-bats runs the tests with their temporary files in BATS_TMP, after sweeping it and /tmp" {
  out=$(cd "$REPO_ROOT" && make -n unit-bats BATS_TMP=/var/tmp/t-a)
  [[ $out == *'tools/tmp-sweep.sh "/var/tmp/t-a" /tmp'* ]]
  [[ $out == *'TMPDIR="/var/tmp/t-a"'*'tools/agent-bats.sh --lane oracle'* ]]
}

@test "unit-bats.sh refuses to start without room for its temporary files, and says so" {
  cd "$REPO_ROOT"
  BATS_MIN_FREE_MB=999999999 BATS_JOBS=1 BATS_FILES=tests/unit/meta.bats run tools/unit-bats.sh
  [ "$status" -ne 0 ]
  [[ $output == *"MB free"* ]]
  [[ $output != *"ok 1"* ]]
}
