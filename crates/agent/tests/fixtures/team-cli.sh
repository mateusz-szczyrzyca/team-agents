#!/usr/bin/env bash
# A stand-in for the `claude` CLI, run by the tests of the daemon's teams. Its configuration says what it does:
# the agent's `args` hold `--scenario LIST`, a comma-separated list of actions; the n-th run of the task takes the n-th
# action, and the last one stays for the runs after it. The slow actions let the other agents of a test finish first.
#
#   done            ends with STATUS: DONE
#   done:TEXT       ends with TEXT, a line break and STATUS: DONE
#   sleep           ends with STATUS: SLEEPING
#   fail            ends with STATUS: FAILED
#   nostatus        ends without a STATUS line
#   questions       ends with STATUS: QUESTIONS and writes no questions file
#   slowdone        waits a second, then ends with STATUS: DONE
#   slowfail        waits a second, then ends with STATUS: FAILED
#   work            commits a file in the worktree, records that the repository's gate passed at the new head (the file
#                   `gate.json` in the task's directory in the home the configuration names) and ends with STATUS: DONE
#   commit          commits a file in the worktree, records no gate and ends with STATUS: DONE: the gate that was recorded
#                   is of an older commit
#   ask             writes a questions file to the path the prompt names (the first `.../questions-N.md` in it) and ends
#                   with STATUS: QUESTIONS
#   integrate       merges every other local branch of the form agents/*/task-*-N.N into the worktree's branch, records
#                   the gate as `work` does and ends with STATUS: DONE
#
# Like an agent that does what its prompt says, every run of a team's task first reads its messages through the
# agentbus MCP server (`read_messages` moves the agent's cursor past them; without it the bus would wake the agent again
# and again for the message of the team's creation). The configuration gives the way there: `--as NAME`, `--bus-bin
# PATH`, `--bus-dir DIR` and `--home DIR`; the team and the task come from the prompt.
#
# Every run counts itself in `.fake-runs` and keeps the prompt it was given in `.fake-prompt-N`, in its working
# directory (the task's worktree), and the line `N <session> <branch>` in `.fake-log`, with the branch the worktree has
# out. A committed script, never written by a test, so that no test races an `exec` of a file that is still open.
set -eu

scenario=""
session=""
name=""
bus_bin=""
bus_dir=""
home=""
args=("$@")
while (($#)); do
  case $1 in
    --scenario) scenario=$2; shift ;;
    --as) name=$2; shift ;;
    --bus-bin) bus_bin=$2; shift ;;
    --bus-dir) bus_dir=$2; shift ;;
    --home) home=$2; shift ;;
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
branch=$(git rev-parse --abbrev-ref HEAD 2>&1) || branch="git failed: $branch"
echo "$n $session $branch" >> .fake-log

IFS=, read -r -a actions <<< "$scenario"
action=${actions[$((n - 1))]:-${actions[${#actions[@]}-1]}}

init() {
  printf '{"type":"system","subtype":"init","session_id":"%s","model":"fake-model"}\n' "$session"
}

# result TEXT — TEXT is JSON text: `\n` is a line break.
result() {
  printf '{"type":"result","subtype":"success","is_error":false,"result":"%s","session_id":"%s"}\n' "$1" "$session"
}

# Reads the messages of the agent: what a well-behaved agent of a team does first.
read_messages() {
  local team task
  team=$(grep -o 'team-[0-9]\{8\}-[0-9]\{6\}-[a-z0-9]\{4\}' <<< "$prompt" | head -n 1)
  task=$(grep -o 'task-[0-9]\{8\}-[0-9]\{6\}-[a-z0-9]\{4\}' <<< "$prompt" | head -n 1)
  [[ -n $bus_bin && -n $team ]] || return 0
  echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read_messages","arguments":{}}}' |
    "$bus_bin" serve --agent "$name" --task "$task" --team "$team" --dir "$bus_dir" --home "$home" > /dev/null
}

# Arguments that give git an identity and no signing: the harness sets neither in the environment of a run.
GIT_FIXTURE=(-c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false)

# Commits a new file in the worktree: what a run of an agent that did its item leaves.
commit_work() {
  echo "run $n of $name" > "work-$n.txt"
  git add "work-$n.txt"
  git "${GIT_FIXTURE[@]}" commit -q -m "Work of run $n of $name"
}

# Records that the repository's gate passed on a clean tree at the head, where the daemon looks for it: the task is the
# name of the worktree's directory.
record_gate() {
  local task head dir
  task=$(basename "$PWD")
  head=$(git rev-parse HEAD)
  dir="$home/tasks/$task"
  mkdir -p "$dir"
  printf '{"tree":"clean","result":{"head":"%s","finished_at":"2026-11-13T10:00:00Z","outcome":{"kind":"passed"},"log":"%s/gate-1.log"}}\n' \
    "$head" "$dir" > "$dir/gate.json"
}

init
read_messages
case $action in
  done) result 'Finished the work.\nSTATUS: DONE' ;;
  done:*) result "${action#done:}"'\nSTATUS: DONE' ;;
  sleep) result 'Nothing to do now.\nSTATUS: SLEEPING' ;;
  fail) result 'I could not do it.\nSTATUS: FAILED' ;;
  nostatus) result 'I think that is all.' ;;
  questions) result 'I have to ask.\nSTATUS: QUESTIONS' ;;
  slowdone) sleep 1; result 'Finished the work.\nSTATUS: DONE' ;;
  slowfail) sleep 1; result 'I could not do it.\nSTATUS: FAILED' ;;
  work)
    commit_work
    record_gate
    result 'Finished the item.\nSTATUS: DONE'
    ;;
  commit)
    commit_work
    result 'Changed something.\nSTATUS: DONE'
    ;;
  ask)
    file=$(grep -o '/[^`[:space:]]*questions-[0-9]*\.md' <<< "$prompt" | head -n 1)
    mkdir -p "$(dirname "$file")"
    printf '# Questions — fake\n\n## 1. What should the team do about the pull request?\na) one more fix pass — the reviewers are asked again\nb) leave it to me — I merge or close it\n\n**Recommended:** a) — a finding was not fixed (fake)\n\n**Verdict:**\n' > "$file"
    result 'I have to ask.\nSTATUS: QUESTIONS'
    ;;
  integrate)
    current=$(git rev-parse --abbrev-ref HEAD)
    for other in $(git for-each-ref --format='%(refname:short)' 'refs/heads/agents/*/task-*-*.*'); do
      [[ $other == "$current" ]] || git "${GIT_FIXTURE[@]}" merge --no-edit "$other" > /dev/null
    done
    record_gate
    result 'Integrated.\nSTATUS: DONE'
    ;;
  *) echo "unknown action: $action" >&2; exit 64 ;;
esac
