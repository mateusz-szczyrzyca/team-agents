#!/usr/bin/env bash
# A stand-in for `agentbus` that fails while the file `flaky-bus-down` is in `$AGENTS_HOME` and runs the real binary,
# whose path `real-agentbus` in `$AGENTS_HOME` holds, otherwise: for the tests of a bus that is down for a while.
# A committed script, never written by a test.
set -eu
if [[ -e "$AGENTS_HOME/flaky-bus-down" ]]; then
  echo "agentbus: the bus directory is not readable" >&2
  exit 8
fi
exec "$(< "$AGENTS_HOME/real-agentbus")" "$@"
