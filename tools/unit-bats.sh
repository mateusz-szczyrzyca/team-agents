#!/usr/bin/env bash
# unit-bats.sh — runs the Bats tests (make unit-bats): in parallel, BATS_JOBS at once (GNU parallel), apart from the
# files of BATS_SERIAL, which run one after another once the others are done. Both groups always run; the result is a
# failure when either fails. With BATS_JOBS=1, or without GNU parallel, every file runs one after another.
#
# Each run has its own temporary directory, which this script removes once it has stopped the processes the tests
# left running in it: bats' own removal raced with them ("Directory not empty") and failed a run whose tests passed.
set -uo pipefail

jobs=${BATS_JOBS:-1}
read -ra files <<< "${BATS_FILES:-}"
read -ra serial <<< "${BATS_SERIAL:-}"
((${#files[@]})) || { echo "unit-bats: no test files (BATS_FILES)" >&2; exit 2; }

# run [bats options] file… — one bats run in a temporary directory of its own, removed afterwards
run() {
  local dir rc
  dir=$(mktemp -u -d "${TMPDIR:-/tmp}/bats-run-XXXXXX")
  bats --no-tempdir-cleanup --tempdir "$dir" "$@" 2> >(grep -v '^BATS_RUN_TMPDIR' >&2)
  rc=$?
  pkill -f -- "$dir" 2> /dev/null   # what the tests left running there ("no process" is fine)
  sleep 1
  pkill -9 -f -- "$dir" 2> /dev/null
  rm -rf "$dir" 2> /dev/null || { sleep 2; rm -rf "$dir" || echo "unit-bats: could not remove $dir" >&2; }
  return "$rc"
}

if ((jobs <= 1)) || ! command -v parallel > /dev/null; then
  ((jobs <= 1)) || echo "unit-bats: no GNU parallel — the tests run one after another (apt install parallel)" >&2
  run "${files[@]}"
  exit
fi

together=() alone=()
for f in "${files[@]}"; do
  found=0
  for s in "${serial[@]}"; do [[ $f != "$s" ]] || found=1; done
  if ((found)); then alone+=("$f"); else together+=("$f"); fi
done
rc=0
if ((${#together[@]})); then run --jobs "$jobs" "${together[@]}" || rc=1; fi
if ((${#alone[@]})); then run "${alone[@]}" || rc=1; fi
exit "$rc"
