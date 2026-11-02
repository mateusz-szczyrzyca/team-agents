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
# Each run has its own temporary directory, which this script removes once it has stopped the processes the tests
# left running in it: bats' own removal raced with them ("Directory not empty") and failed a run whose tests passed.
set -uo pipefail

jobs=${BATS_JOBS:-1}
timings=${BATS_TIMINGS:-build/bats-timings.txt}
read -ra files <<< "${BATS_FILES:-}"
read -ra serial <<< "${BATS_SERIAL:-}"
((${#files[@]})) || { echo "unit-bats: no test files (BATS_FILES)" >&2; exit 2; }
reports=$(mktemp -d)
trap 'rm -rf "$reports"' EXIT

# run report [bats options] file… — one bats run in a temporary directory of its own, removed afterwards; its JUnit
# report goes to the directory report
run() {
  local report=$1 dir rc
  shift
  dir=$(mktemp -u -d "${TMPDIR:-/tmp}/bats-run-XXXXXX")
  mkdir -p "$report"
  bats --no-tempdir-cleanup --tempdir "$dir" --report-formatter junit --output "$report" "$@" \
    2> >(grep -v '^BATS_RUN_TMPDIR' >&2)
  rc=$?
  pkill -f -- "$dir" 2> /dev/null   # what the tests left running there ("no process" is fine)
  sleep 1
  pkill -9 -f -- "$dir" 2> /dev/null
  rm -rf "$dir" 2> /dev/null || { sleep 2; rm -rf "$dir" || echo "unit-bats: could not remove $dir" >&2; }
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
  run "$reports/all" "${ordered[@]}"
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
  run "$reports/serial" "${fmt[@]}" "${alone[@]}" > "$held" 2>&1 &
  spid=$!
fi
if ((${#together[@]})); then
  mapfile -t together < <(slowest_first "${together[@]}")
  run "$reports/parallel" --jobs "$jobs" "${together[@]}" || rc=1
fi
if [[ -n $spid ]]; then
  wait "$spid" || rc=1
  echo
  cat "$held"
fi
rm -f "$held"
record
exit "$rc"
