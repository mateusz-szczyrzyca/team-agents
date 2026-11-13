#!/usr/bin/env bash
# A stand-in for an editor, run by the tests of `agent answer`: `fake-editor.sh MODE FILE` does what MODE says to FILE.
#
#   accept    puts `ok` after every empty `**Verdict:**`
#   own       puts `Use the disk` after every empty `**Verdict:**`
#   nothing   leaves the file as it is
#   fail      exits with status 3
#
# A committed script, never written by a test.
set -eu
mode=$1
file=$2
case $mode in
  accept) sed -i 's/^\*\*Verdict:\*\*[[:space:]]*$/**Verdict:** ok/' "$file" ;;
  own) sed -i 's/^\*\*Verdict:\*\*[[:space:]]*$/**Verdict:** Use the disk/' "$file" ;;
  nothing) ;;
  fail) echo "the editor failed" >&2; exit 3 ;;
  *) echo "unknown mode: $mode" >&2; exit 64 ;;
esac
