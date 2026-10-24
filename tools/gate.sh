#!/usr/bin/env bash
# make gate — the checks of `make lint test` with short output: one line per step, the lines that tell what failed,
# and the full output of every step in build/gate/<step>.log. Every step runs, even after a failure, so one run
# shows every problem.
#
# Why: a tool's output stays in an agent's context and is read again at every later step of its session. Read the
# logs with grep or tail when the excerpt is not enough — never print a whole log.
#
#   tools/gate.sh                    all steps of make lint test
#   tools/gate.sh unit-go lint-go    only these Makefile targets
set -euo pipefail

ROOT=${GATE_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}   # the repository (tests: a directory with a fake Makefile)
LOGS=${GATE_LOG_DIR:-$ROOT/build/gate}                 # where the full logs go
MAX_LINES=${GATE_MAX_LINES:-25}                        # at most this many lines of a failed step are shown
STEPS=(check-generated lint-shell lint-go lint-docs unit-go unit-bats)
if (($# > 0)); then STEPS=("$@"); fi

# excerpt step log — the lines of a failed step's log that tell what failed, by the tool behind the step
excerpt() {
  case $1 in
    unit-go)   # failed tests with their messages, panics, compile errors — not the passing tests
      awk '/^--- FAIL|^panic:|^FAIL|^[^ \t].*\.go:[0-9]+:[0-9]+: / { print; inblk = ($0 ~ /^--- FAIL/); next }
           /^(=== |--- PASS|--- SKIP|ok )/ { inblk = 0; next }
           inblk { print }' "$2" ;;
    unit-bats) # every failed test: its line and the first lines of its diagnosis
      awk '/^not ok / { print; n = 4; next } n > 0 && /^#/ { print; n--; next } { n = 0 }' "$2" ;;
    lint-go | lint-docs) # findings ("file:line:col: message") and the summary, without the code snippets
      { grep -E '^[^[:space:]].*:[0-9]+(:[0-9]+)?: |^[0-9]+ issues?|undocumented declarations' "$2" || (($? == 1)); } ;;
    lint-shell) # where (file and line) and what (the SC code and message); bash -n syntax errors
      { grep -E '^In .* line [0-9]+:|^[[:space:]]*\^-- SC[0-9]+|: line [0-9]+: ' "$2" || (($? == 1)); } ;;
    *) tail -n "$MAX_LINES" "$2" ;;
  esac | sed -n "1,${MAX_LINES}p" | sed 's/^/  /'   # sed reads to the end: no SIGPIPE for the writer
}

mkdir -p "$LOGS"
echo "gate: ${#STEPS[@]} steps, full logs in ${LOGS#"$ROOT"/}/"
failed=()
for step in "${STEPS[@]}"; do
  log=$LOGS/$step.log
  start=$SECONDS
  if make -s -C "$ROOT" "$step" > "$log" 2>&1; then
    printf 'ok    %-16s %4ss\n' "$step" $((SECONDS - start))
  else
    printf 'FAIL  %-16s %4ss  %s\n' "$step" $((SECONDS - start)) "${log#"$ROOT"/}"
    excerpt "$step" "$log"
    failed+=("$step")
  fi
done
if ((${#failed[@]} == 0)); then
  echo "gate: all ${#STEPS[@]} steps passed"
  exit 0
fi
echo "gate: ${#failed[@]} of ${#STEPS[@]} steps failed (${failed[*]}) — more: grep -n <pattern> ${LOGS#"$ROOT"/}/<step>.log"
exit 1
