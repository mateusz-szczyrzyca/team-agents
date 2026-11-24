#!/usr/bin/env bash
# A stand-in for the `claude` CLI, used by the end-to-end tests of a run that has Serena. It takes `--home DIR` (the
# `AGENTS_HOME` of the daemon), `--prompt-file FILE` (where it writes the prompt it was given), `--tool NAME` (a tool it
# calls, once a line, optional) and `--status LINE` (the status line of its final message). It writes the plan a solo
# run of a task is held to, so that the run is not sent back for it. With `--tool` it prints a stream of Claude Code's
# JSON that calls each tool named, commits a file of its own and records the gate it passes at the head, where the
# daemon looks for it, so that the steps of the run reach the history.
set -eu

home=""
prompt_file=""
status="STATUS: DONE"
tools=()
args=("$@")
while (($#)); do
  case $1 in
    --home) home=$2; shift ;;
    --prompt-file) prompt_file=$2; shift ;;
    --tool) tools+=("$2"); shift ;;
    --status) status=$2; shift ;;
  esac
  shift
done

printf '%s\n' "${args[${#args[@]}-1]}" > "$prompt_file"
mkdir -p .agent
printf 'The thing.\n\n1. Do\n[x] 1.1 The work\n    Check: exists README.md\n' > .agent/plan.md

printf '{"type":"system","subtype":"init","session_id":"fake-session","model":"fake-model"}\n'
if ((${#tools[@]})); then
  # The plan and the files of this stand-in stay out of git's sight, so that the gate sees a clean tree.
  exclude=$(git rev-parse --git-path info/exclude)
  grep -qxF '.agent/' "$exclude" 2> /dev/null || printf '%s\n' '.agent/' >> "$exclude"
  printf 'the work\n' > work.txt
  git add work.txt
  for tool in "${tools[@]}"; do
    printf '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"%s","input":{}}]}}\n' "$tool"
  done
  git -c user.name=Stand-in -c user.email=standin@example.invalid -c commit.gpgsign=false \
    commit -q -m "Do the work"
  # The task is the name of the worktree's directory, where the daemon keeps the record of its gate.
  head=$(git rev-parse HEAD)
  dir="$home/tasks/$(basename "$PWD")"
  mkdir -p "$dir"
  printf '{"tree":"clean","result":{"head":"%s","finished_at":"2026-11-24T10:00:00Z","outcome":{"kind":"passed"},"log":"%s/gate-1.log"}}\n' \
    "$head" "$dir" > "$dir/gate.json"
fi
printf '{"type":"result","subtype":"success","is_error":false,"result":"Finished.\\n%s","session_id":"fake-session"}\n' "$status"
