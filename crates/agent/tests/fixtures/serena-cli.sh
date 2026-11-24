#!/usr/bin/env bash
# A stand-in for the `claude` CLI, used by the end-to-end tests of a run that has Serena. It takes `--prompt-file FILE`
# (where it writes the prompt it was given), `--tool NAME` (a tool it calls, once a line, optional), `--gate FILE` (the
# record of a gate it passes for the commit it makes, optional) and `--status LINE` (the status line of its final
# message). It writes the plan a solo run of a task is held to, so that the run is not sent back for it. With `--tool`
# it prints a stream of Claude Code's JSON that calls each tool named, for a step to reach the history.
set -eu

prompt_file=""
gate=""
status="STATUS: DONE"
tools=()
args=("$@")
while (($#)); do
  case $1 in
    --prompt-file) prompt_file=$2; shift ;;
    --tool) tools+=("$2"); shift ;;
    --gate) gate=$2; shift ;;
    --status) status=$2; shift ;;
  esac
  shift
done

printf '%s\n' "${args[${#args[@]}-1]}" > "$prompt_file"
mkdir -p .agent
printf 'The thing.\n\n1. Do\n[x] 1.1 The work\n    Check: exists README.md\n' > .agent/plan.md

printf '{"type":"system","subtype":"init","session_id":"fake-session","model":"fake-model"}\n'
if ((${#tools[@]})); then
  # The work is a file of its own; the stand-in's files are hidden from git like the other stand-ins' ones.
  exclude=$(git rev-parse --git-dir)/info/exclude
  mkdir -p "$(dirname "$exclude")"
  printf '.fake-*\n' >> "$exclude"
  printf 'the work\n' > work.txt
  git add work.txt
  for tool in "${tools[@]}"; do
    printf '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"%s","input":{}}]}}\n' "$tool"
  done
  git -c commit.gpgsign=false commit -q -m "Do the work"
  if [[ -n $gate ]]; then
    head=$(git rev-parse HEAD)
    printf '{"head":"%s","tree":"clean","finished_at":"%s","outcome":"passed"}\n' "$head" \
      "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$gate"
  fi
fi
printf '{"type":"result","subtype":"success","is_error":false,"result":"Finished.\\n%s","session_id":"fake-session"}\n' "$status"
