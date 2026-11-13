#!/usr/bin/env bash
# A stand-in for the remote command of a VS Code server, run by the tests of the opener of questions files: it is given
# the file to open and the socket of the window in `VSCODE_IPC_HOOK_CLI`.
#
#   - a socket whose name has `dead` in it is a window that was closed: the command fails
#   - a socket whose name has `slow` in it never answers
#   - otherwise the command records `FILE` in `<socket>.opened` and succeeds
#
# A committed script, never written by a test.
set -eu
socket=${VSCODE_IPC_HOOK_CLI:?}
case $socket in
  *dead*) echo "connect ECONNREFUSED $socket" >&2; exit 1 ;;
  *slow*) sleep 600 ;;
esac
printf '%s\n' "$1" >> "$socket.opened"
