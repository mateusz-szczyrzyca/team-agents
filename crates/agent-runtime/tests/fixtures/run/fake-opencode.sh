#!/usr/bin/env bash
# A stand-in for the `opencode` CLI, run by the tests of the reminder of Serena: it ends the way the file
# `.fake-scenario` in its working directory says, speaking the `run --format json` stream of opencode. A committed
# script, never written by a test.
#
#   serena-reads    reads forty-two code files of `src` with its `read` tool, then ends with STATUS: DONE
#   serena-globs    finds code files forty-two times with its `glob` tool, then ends with STATUS: DONE
#   serena-calls    calls a tool of Serena, then reads code files as `serena-reads` does
#   serena-text     reads forty-two text files as `serena-reads` reads code ones
#
# Every run records its arguments, one a line, in `.fake-args`.
set -eu

scenario=$(cat .fake-scenario)
printf '%s\n' "$@" > .fake-args

session=ses_fakeopencode0001

# tool_use NAME INPUT_JSON — one tool call of the stream, completed.
tool_use() {
  printf '{"type":"step_start","sessionID":"%s","part":{"type":"step-start"}}\n' "$session"
  printf '{"type":"tool_use","sessionID":"%s","part":{"type":"tool","tool":"%s","state":{"status":"completed","input":%s}}}\n' \
    "$session" "$1" "$2"
  printf '{"type":"step_finish","sessionID":"%s","part":{"type":"step-finish","reason":"tool-calls"}}\n' "$session"
}

# many TOOL KEY VALUE_PREFIX VALUE_SUFFIX N — N calls of TOOL, the values numbered over three names.
many() {
  local tool=$1 key=$2 prefix=$3 suffix=$4 n=$5 i
  for ((i = 0; i < n; i++)); do
    tool_use "$tool" "{\"$key\":\"$prefix$((i % 3))$suffix\"}"
  done
}

finish() {
  printf '{"type":"step_start","sessionID":"%s","part":{"type":"step-start"}}\n' "$session"
  printf '{"type":"text","sessionID":"%s","part":{"type":"text","text":"Finished the item.\\nSTATUS: DONE"}}\n' "$session"
  printf '{"type":"step_finish","sessionID":"%s","part":{"type":"step-finish","reason":"stop"}}\n' "$session"
}

case $scenario in
  serena-reads) many read filePath src/module- .rs 42 ;;
  serena-globs) many glob pattern 'src/module-' '/**/*.rs' 42 ;;
  serena-calls)
    tool_use serena_find_symbol '{}'
    many read filePath src/module- .rs 42
    ;;
  serena-text) many read filePath docs/note- .md 42 ;;
  *) echo "unknown scenario $scenario" >&2; exit 2 ;;
esac
finish
