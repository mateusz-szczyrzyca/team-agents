#!/usr/bin/env bash
# A stand-in for the gate command of a repository, run by the tests of `agent gate`: it notes when it starts and when
# it ends, with the name of the worktree it runs in, in the file its first argument names, and takes a second between
# the two, so that gates that overlap show in that file. A committed script, never written by a test, so that no test
# races an `exec` of a file that is still open.
set -eu

order=$1
name=$(basename "$PWD")
echo "start $name" >> "$order"
sleep 1
echo "end $name"
echo "end $name" >> "$order"
