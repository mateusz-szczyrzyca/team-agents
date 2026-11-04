#!/usr/bin/env bash
# unit-bats.sh — runs the Bats tests (make unit-bats). The files of BATS_SERIAL start servers on local ports: they run
# one after another, from the start, beside all the others, which run in parallel, BATS_JOBS at once (GNU parallel).
# The serial group's output is held and printed after the parallel group's, so that the two never mix. Both groups
# always run; the result is a failure when either fails. With BATS_JOBS=1, or without GNU parallel, every file runs
# one after another.
#
# The slowest files start first: each run records how long every file took (bats' JUnit report) in BATS_TIMINGS, and
# the next run orders the files by it — a file it does not know comes first, as it may be slow.
#
# bats removes each test's temporary directory as the test ends; what the tests leave running is stopped, and every
# run's directory removed, when the script ends — interrupted too.
set -uo pipefail

jobs=${BATS_JOBS:-1}
timings=${BATS_TIMINGS:-build/bats-timings.txt}
read -ra files <<< "${BATS_FILES:-}"
read -ra serial <<< "${BATS_SERIAL:-}"
((${#files[@]})) || { echo "unit-bats: no test files (BATS_FILES)" >&2; exit 2; }
reports=$(mktemp -d)
runs=$(mktemp -d "${TMPDIR:-/tmp}/bats-XXXXXX")   # every run's temporary directory, under one
trap 'stop_left "$runs"; rm -rf "$reports"' EXIT   # stopped or interrupted: what runs under $runs is stopped too
trap 'exit 130' INT TERM

# stop_left dir — stops what the tests left running in dir, and removes it
stop_left() {
  pkill -f -- "$1" 2> /dev/null   # "no process" is fine
  sleep 1
  pkill -9 -f -- "$1" 2> /dev/null
  rm -rf "$1" 2> /dev/null || { sleep 2; rm -rf "$1" || echo "unit-bats: could not remove $1" >&2; }
}

# run name report [bats options] file… — one bats run in its own temporary directory (bats removes each test's as
# the test ends, so that a run needs no more room than the tests running at once), its JUnit report in the directory
# report. bats fails a run when it cannot remove a directory a test's processes still write to ("Directory not
# empty"): such a run counts as passed when its report has every test and no failure.
run() {
  local dir=$runs/$1 report=$2 rc err f expected tests failures
  local bats_files=()
  shift 2
  for f in "$@"; do [[ $f != *.bats ]] || bats_files+=("$f"); done
  err=$(mktemp)
  mkdir -p "$report"
  bats --tempdir "$dir" --report-formatter junit --output "$report" "$@" 2> >(tee "$err" >&2) &
  wait "$!"   # in the background: a signal to this script is handled at once, not when bats ends
  rc=$?
  stop_left "$dir"
  if ((rc != 0)) && grep -q 'Directory not empty' "$err"; then
    expected=$(bats --count "${bats_files[@]}" 2> /dev/null)
    read -r tests failures < <(grep -o '<testsuite [^>]*' "$report/report.xml" 2> /dev/null \
      | awk '{for (i = 1; i <= NF; i++) {split($i, kv, "="); gsub(/"/, "", kv[2]);
                if (kv[1] == "tests") t += kv[2]; if (kv[1] == "failures" || kv[1] == "errors") x += kv[2]}}
             END {print t + 0, x + 0}')
    if [[ -n $expected && $tests == "$expected" && $failures == 0 ]]; then
      echo "unit-bats: all $tests tests passed; bats only failed to remove a directory the tests' processes still wrote to" >&2
      rc=0
    fi
  fi
  rm -f "$err"
  return "$rc"
}

# slowest_first file… — the files, the slowest of the last run first; those it does not know before them
slowest_first() {
  [[ -s $timings ]] || { printf '%s\n' "$@"; return; }
  printf '%s\n' "$@" | awk 'NR == FNR {t[$1] = $2; next} {print (($0 in t) ? t[$0] : 1e9), $0}' "$timings" - \
    | sort -k1,1gr | cut -d' ' -f2-
}

# record — this run's times per file (from the JUnit reports) over the recorded ones
record() {
  local new
  new=$(cat "$reports"/*/report.xml 2> /dev/null \
    | sed -n 's|.*<testsuite name="\([^"]*\)".* time="\([0-9.]*\)".*|tests/unit/\1 \2|p')
  [[ -n $new ]] || return 0
  mkdir -p "$(dirname "$timings")"
  { [[ -f $timings ]] && cat "$timings"; echo "$new"; } \
    | awk '{t[$1] = $2} END {for (f in t) print f, t[f]}' | sort > "$timings.new" && mv "$timings.new" "$timings"
}

if ((jobs <= 1)) || ! command -v parallel > /dev/null; then
  ((jobs <= 1)) || echo "unit-bats: no GNU parallel — the tests run one after another (apt install parallel)" >&2
  mapfile -t ordered < <(slowest_first "${files[@]}")
  run all "$reports/all" "${ordered[@]}"
  rc=$?
  record
  exit "$rc"
fi

together=() alone=()
for f in "${files[@]}"; do
  found=0
  for s in "${serial[@]}"; do [[ $f != "$s" ]] || found=1; done
  if ((found)); then alone+=("$f"); else together+=("$f"); fi
done
rc=0 held=$(mktemp) spid=""
fmt=()
[[ -t 1 ]] && fmt=(--formatter pretty)   # held in a file, the serial group still prints as the terminal would
if ((${#alone[@]})); then
  mapfile -t alone < <(slowest_first "${alone[@]}")
  run serial "$reports/serial" "${fmt[@]}" "${alone[@]}" > "$held" 2>&1 &
  spid=$!
fi
if ((${#together[@]})); then
  mapfile -t together < <(slowest_first "${together[@]}")
  run parallel "$reports/parallel" --jobs "$jobs" "${together[@]}" || rc=1
fi
if [[ -n $spid ]]; then
  wait "$spid" || rc=1
  echo
  cat "$held"
fi
rm -f "$held"
record
exit "$rc"
