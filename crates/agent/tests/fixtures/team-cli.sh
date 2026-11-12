#!/usr/bin/env bash
# A stand-in for the `claude` CLI, run by the tests of the daemon's teams. Its configuration says what it does:
# the agent's `args` hold `--scenario LIST`, a comma-separated list of actions; the n-th run of the task takes the n-th
# action, and the last one stays for the runs after it.
#
#   done            ends with STATUS: DONE
#   done:TEXT       ends with TEXT, a line break and STATUS: DONE
#   sleep           ends with STATUS: SLEEPING
#   fail            ends with STATUS: FAILED
#   nostatus        ends without a STATUS line
#   slowdone        waits a second, then ends with STATUS: DONE
#
# Every run counts itself in `.fake-runs` and keeps the prompt it was given in `.fake-prompt-N`, in its working
# directory (the task's worktree), and the line `N <session> <branch>` in `.fake-log`, with the branch the worktree has
# out. A committed script, never written by a test, so that no test races an `exec` of a file that is still open.
set -eu

scenario=""
session=""
args=("$@")
while (($#)); do
  case $1 in
    --scenario) scenario=$2; shift ;;
    --session-id | --resume) session=$2; shift ;;
  esac
  shift
done
prompt=${args[${#args[@]}-1]}

n=1
if [[ -f .fake-runs ]]; then n=$(( $(< .fake-runs) + 1 )); fi
echo "$n" > .fake-runs
printf '%s\n' "$prompt" > ".fake-prompt-$n"
printf '%s\n' "${args[@]}" > ".fake-args-$n"
echo "$n $session $(git rev-parse --abbrev-ref HEAD)" >> .fake-log

IFS=, read -r -a actions <<< "$scenario"
action=${actions[$((n - 1))]:-${actions[${#actions[@]}-1]}}

init() {
  printf '{"type":"system","subtype":"init","session_id":"%s","model":"fake-model"}\n' "$session"
}

# result TEXT — TEXT is JSON text: `\n` is a line break.
result() {
  printf '{"type":"result","subtype":"success","is_error":false,"result":"%s","session_id":"%s"}\n' "$1" "$session"
}

init
case $action in
  done) result 'Finished the work.\nSTATUS: DONE' ;;
  done:*) result "${action#done:}"'\nSTATUS: DONE' ;;
  sleep) result 'Nothing to do now.\nSTATUS: SLEEPING' ;;
  fail) result 'I could not do it.\nSTATUS: FAILED' ;;
  nostatus) result 'I think that is all.' ;;
  slowdone) sleep 1; result 'Finished the work.\nSTATUS: DONE' ;;
  *) echo "unknown action: $action" >&2; exit 64 ;;
esac
