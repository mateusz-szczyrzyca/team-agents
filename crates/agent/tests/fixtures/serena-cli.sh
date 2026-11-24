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
  # The work is a file of its own, committed like the other stand-ins' work; the plan and this stand-in's files stay out
  # of git's sight, so that the tree is clean.
  exclude=$(git rev-parse --git-path info/exclude)
  printf '%s\n' '.agent/plan.md' >> "$exclude"
  printf 'the work\n' > work.txt
  git add work.txt
  git -c commit.gpgsign=false commit -q -m "Do the work"
  if [[ -n $gate ]]; then
    head=$(git rev-parse HEAD)
    dir=$(dirname "$gate")
    mkdir -p "$dir"
    printf '{"tree":"clean","result":{"head":"%s","finished_at":"%s","outcome":{"kind":"passed"},"log":"%s/gate-1.log"}}\n' "$head" \
      "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$dir" > "$gate"
  fi
fi
printf '{"type":"result","subtype":"success","is_error":false,"result":"Finished.\\n%s","session_id":"fake-session"}\n' "$status"
