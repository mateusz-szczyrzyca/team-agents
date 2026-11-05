#!/usr/bin/env bash
# tmp-sweep.sh dir… — removes what earlier test runs left in the directories: agent-bats.sh's staged copies of the
# repository (staged-oracle-*, staged-agent-*) and its reports (agent-bats-reports-*), the runs of unit-bats.sh and of
# bats (bats-??????, bats-run-*) — the user's own, older than TMP_SWEEP_MIN minutes (default 360), used by no running
# process. A run stopped hard (a killed check, a restart) leaves them; on a tmpfs they take memory.
set -uo pipefail

min=${TMP_SWEEP_MIN:-360}
for d in "$@"; do
  [[ -d $d ]] || continue
  while IFS= read -r -d '' p; do
    if pgrep -f -- "$p" > /dev/null 2>&1; then continue; fi   # a run still at work in it
    rm -rf -- "$p" 2> /dev/null || echo "tmp-sweep: could not remove $p" >&2
  done < <(find "$d" -mindepth 1 -maxdepth 1 -type d -user "$(id -u)" -mmin +"$min" \
    \( -name 'staged-oracle-*' -o -name 'staged-agent-*' -o -name 'agent-bats-reports-*' \
       -o -name 'bats-??????' -o -name 'bats-run-*' \) -print0 2> /dev/null)
done
exit 0
