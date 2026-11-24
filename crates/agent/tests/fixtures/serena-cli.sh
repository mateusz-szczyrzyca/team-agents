#!/usr/bin/env bash
# A stand-in for the `claude` CLI, used by the end-to-end tests of a run that has Serena. It takes `--prompt-file FILE`
# (where it writes the prompt it was given) and `--status LINE` (the status line of its final message). It writes the
# plan a solo run of a task is held to, so that the run is not sent back for it, and changes nothing else.
set -eu

prompt_file=""
status="STATUS: DONE"
args=("$@")
while (($#)); do
  case $1 in
    --prompt-file) prompt_file=$2; shift ;;
    --status) status=$2; shift ;;
  esac
  shift
done

printf '%s\n' "${args[${#args[@]}-1]}" > "$prompt_file"
mkdir -p .agent
printf 'The thing.\n\n1. Do\n[x] 1.1 The work\n    Check: exists README.md\n' > .agent/plan.md
printf '{"type":"system","subtype":"init","session_id":"fake-session","model":"fake-model"}\n'
printf '{"type":"result","subtype":"success","is_error":false,"result":"Finished.\\n%s","session_id":"fake-session"}\n' "$status"
