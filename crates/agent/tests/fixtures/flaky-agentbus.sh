#!/usr/bin/env bash
# A stand-in for `agentbus` that fails while the file `flaky-bus-down` is in `$AGENTS_HOME` and runs the real binary,
# whose path `real-agentbus` in `$AGENTS_HOME` holds, otherwise: for the tests of a bus that is down for a while.
# Each call it fails while the bus is down adds its subcommand as a line to `flaky-bus-refused` in `$AGENTS_HOME`.
# While `flaky-bus-refuse-once` in `$AGENTS_HOME` holds a subcommand, the first call of it is refused for good, as by a
# bus that does not know what the call names (exit status 3), and the file is removed.
# A committed script, never written by a test.
set -eu
if [[ -e "$AGENTS_HOME/flaky-bus-down" ]]; then
  echo "${1:-}" >> "$AGENTS_HOME/flaky-bus-refused"
  echo "agentbus: the bus directory is not readable" >&2
  exit 8
fi
if [[ -e "$AGENTS_HOME/flaky-bus-refuse-once" && "$(< "$AGENTS_HOME/flaky-bus-refuse-once")" == "${1:-}" ]]; then
  rm "$AGENTS_HOME/flaky-bus-refuse-once"
  echo "agentbus: not found" >&2
  exit 3
fi
exec "$(< "$AGENTS_HOME/real-agentbus")" "$@"
