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
#   fail-first:TEXT  the first task of the home to run this action ends with STATUS: FAILED; every other one ends with
#                   TEXT, a line break and STATUS: DONE: of two reviewers, exactly one cannot work
#   slowdone        waits a second, then ends with STATUS: DONE
#   slowfail        waits a second, then ends with STATUS: FAILED
#   gated           commits a file in the worktree, excludes its own `.fake-*` files from git, creates `.fake-committed-N` and waits
#                   for `.fake-gate-done-N` (N is the number of the run; the test creates it once the daemon ran the gate), then
#                   ends with STATUS: DONE: the gate that gates the commit is the daemon's own
#   integrate-gated  integrates like `integrate` and waits for the daemon's gate like `gated`
#   lead-gated      integrates like `integrate`, or commits a fix when the prompt is a fix pass; waits for the daemon's gate
#   late:TEXT       waits until the other reviewer of the pass has its verdict in `state.json`, then ends with TEXT and STATUS: DONE
#   work            commits a file in the worktree, records that the repository's gate passed at the new head (the file
#                   `gate.json` in the task's directory in the home the configuration names) and ends with STATUS: DONE
#   work-pushed     like `work`, and pushes the branch to origin itself before it ends
#   commit-clean    commits a file in the worktree, keeps its own `.fake-*` files out of git's sight, records no gate and
#                   ends with STATUS: DONE: the tree is clean and nobody ran the gate
#   break           like `commit-clean`, and what it commits is the file `gate-fails`, with which the gate of the tests'
#                   repository fails
#   mend            removes the file `gate-fails`, commits and ends with STATUS: DONE, without a gate
#   commit          commits a file in the worktree, records no gate and ends with STATUS: DONE: the gate that was recorded
#                   is of an older commit
#   ask-here        writes questions-1.md in this task's directory, including on a resumed assignment
#   ask             writes a questions file to the path the prompt names (the first `.../questions-N.md` in it) and ends
#                   with STATUS: QUESTIONS
#   script:NAME     runs the bash script `fake-scripts/NAME.sh` of the home the configuration names, in the worktree, then
#                   records the gate as `work` does and ends with STATUS: DONE: a test writes the script, which commits
#                   whatever the run is to have done
#   stream:NAME     prints the lines of the file `fake-scripts/NAME.jsonl` of the home the configuration names — events of Claude
#                   Code's stream JSON, which a test writes — and ends with STATUS: DONE: the steps the history is to record
#   fix-held:K      like `fix-work:K`, but first creates `.fake-started-N` and waits for the test to create `.fake-release-N` (N is
#                   the number of the run): a message can reach the agent while its run works
#   answer:K        ends with FINDING K: FIXED and STATUS: DONE, and changes nothing in the worktree
#   deaf-ACTION     does ACTION without reading its messages first, like an agent that ignores the wake-up text
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

# Keep evidence used after a merged round removes its worktrees.
records_dir="$(git rev-parse --git-common-dir)/fake-records/$(basename "$PWD")"
mkdir -p "$records_dir"
cp .fake-runs ".fake-prompt-$n" "$records_dir/"

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
  [[ $deaf == true ]] && return 0
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

# Keeps the stand-in's own files out of git's sight, so that a gate the daemon runs sees a clean tree.
hide_own_files() {
  local exclude
  exclude=$(git rev-parse --git-path info/exclude)
  grep -qxF '.fake-*' "$exclude" 2> /dev/null || printf '%s\n' '.fake-*' >> "$exclude"
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

# Keeps the stand-in's own files out of git's sight, says that the run has committed and waits for the test to say that the
# daemon ran the gate (`.fake-committed-N`, then `.fake-gate-done-N`).
wait_for_gate() {
  printf '%s\n' '.fake-*' >> "$(git rev-parse --git-path info/exclude)"
  mkfifo ".fake-gate-done-$n"
  : > ".fake-committed-$n"
  read -r -t 30 _ < ".fake-gate-done-$n"
  rm -f ".fake-gate-done-$n"
}

# Waits until the other reviewer of this review pass (`n`-th run of the reviewer task, so `2 * n - 1` submitted verdicts in
# `state.json`) has had its verdict recorded by the daemon.
wait_for_peer_review() {
  local submitted fifo pid
  fifo="$home/.fake-peer-wait-$$"
  mkfifo "$fifo"
  tail -F -n +1 "$home/state.json" > "$fifo" 2>&1 &
  pid=$!
  while IFS= read -r _; do
    if [[ -f "$home/state.json" ]]; then
      # `grep -c` exits with 1 when no review is submitted yet.
      submitted=$(grep -c '"submitted"' "$home/state.json" || test $? -eq 1)
      (( submitted >= 2 * n - 1 )) && break
    fi
  done < "$fifo"
  kill "$pid" || test $? -eq 1
  wait "$pid" || test $? -gt 128
  rm -f "$fifo"
}

deaf=false
if [[ $action == deaf-* ]]; then
  deaf=true
  action=${action#deaf-}
fi
init
read_messages
case $action in
  held-review)
    # A test releases each report explicitly, without depending on which reviewer finishes first.
    mkfifo ".fake-review-result-$n"
    : > ".fake-review-ready-$n"
    IFS= read -r -t 30 report < ".fake-review-result-$n"
    rm ".fake-review-result-$n"
    result "$report\nSTATUS: DONE"
    ;;
  turn-limit-quota)
    printf '%s\n' 'quota exhausted' >&2
    printf '{"type":"result","subtype":"error_max_turns","is_error":true,"session_id":"%s"}\n' "$session"
    ;;
  turn-limit)
    printf '{"type":"result","subtype":"error_max_turns","is_error":true,"session_id":"%s"}\n' "$session"
    ;;
  done) result 'Finished the work.\nSTATUS: DONE' ;;
  review-commit:*|late-commit:*)
    # These new actions quote the commit actually checked out by the reviewer.
    if [[ $action == late-commit:* ]]; then
      wait_for_peer_review
    fi
    head=$(git rev-parse HEAD)
    result "VERDICT: ${action#*:}\nSUMMARY: commit \`$head\`\nSTATUS: DONE"
    ;;
  structured-commit:*|late-structured-commit:*)
    # Commit-link scenarios also answer the structured findings protocol.
    if [[ $action == late-structured-commit:* ]]; then
      wait_for_peer_review
    fi
    head=$(git rev-parse HEAD)
    verdict=${action#*:}
    report="VERDICT: $verdict\nSUMMARY: commit \`$head\`"
    verifying=false
    while IFS= read -r line; do
      if [[ $line == '# OPEN FINDINGS TO VERIFY' ]]; then verifying=true; fi
      if [[ $verifying == true && $line =~ ^([0-9]+)\.\ \[ ]]; then
        report+="\nFINDING ${BASH_REMATCH[1]}: RESOLVED"
      fi
    done <<< "$prompt"
    if [[ $verdict == REQUEST_CHANGES ]]; then
      report+='\nFINDINGS\n1. [MAJOR] TDD-2 work-1.txt:1 — the case is untested → add coverage\n   CHECK: read work-1.txt'
    fi
    result "$report"'\nSTATUS: DONE'
    ;;
  opinion-retry:*)
    if [[ $prompt == *'# SECOND OPINION REQUEST'* || $prompt == *'# CORRECT YOUR SECOND OPINION'* ]]; then
      if [[ $prompt == *'# SECOND OPINION REQUEST'* ]]; then
        number=$(awk '/^[0-9]+\. \[/{sub(/\./, "", $1); print $1; exit}' <<< "$prompt")
        printf '%s' "$number" > .opinion-number
      else
        number=$(cat .opinion-number)
      fi
      marker="$home/opinion-first-$number"
      if mkdir "$marker"; then printf '%s' "$PWD" > "$marker/owner"; fi
      owner=$(cat "$marker/owner")
      if [[ $owner == "$PWD" ]]; then
        case ${action#opinion-retry:} in
          RATE)
            printf '%s\n' 'Rate limit reached for requests' >&2
            printf '{"type":"result","subtype":"error","is_error":true,"session_id":"%s"}\n' "$session"
            exit 1
            ;;
          FAILED) result 'Cannot inspect commit.\nSTATUS: FAILED' ;;
          EMPTY) result 'Cannot inspect commit.' ;;
          INVALID) result 'No opinion line.\nSTATUS: DONE' ;;
        esac
      else
        result 'OPINION: NOT CONFIRMED\nRead café path; replacement established it is unreachable.\nSTATUS: DONE'
      fi
    else
      result 'VERDICT: REQUEST_CHANGES\nFINDINGS\n1. [BLOCKER] COR src/lib.rs:7 — café defect → fix it\n   CHECK: read src/lib.rs:7\nSTATUS: DONE'
    fi
    ;;
  opinion-failure:*)
    if [[ $prompt == *'# SECOND OPINION REQUEST'* || $prompt == *'# CORRECT YOUR SECOND OPINION'* ]]; then
      case ${action#opinion-failure:} in
        FAILED) result 'Cannot inspect commit.\nSTATUS: FAILED' ;;
        EMPTY) result 'Cannot inspect commit.' ;;
        INVALID) result 'No opinion line.\nSTATUS: DONE' ;;
        HOLD) while :; do sleep 0.02; done ;;
      esac
    else
      result 'VERDICT: REQUEST_CHANGES\nFINDINGS\n1. [BLOCKER] COR src/lib.rs:7 — café defect → fix it\n   CHECK: read src/lib.rs:7\nSTATUS: DONE'
    fi
    ;;
  opinion-fix-pushed)
    commit_work
    record_gate
    git "${GIT_FIXTURE[@]}" push -q origin HEAD
    result 'FINDING 1: FIXED\nFINDING 2: FIXED\nSTATUS: DONE'
    ;;
  opinion-release)
    if [[ $prompt == *'# SECOND OPINION REQUEST'* ]]; then
      while [[ ! -f $home/opinion-release ]]; do sleep 0.02; done
      result 'OPINION: NOT CONFIRMED\nRead src/lib.rs:7; café path is unreachable.\nSTATUS: DONE'
    else
      result 'VERDICT: REQUEST_CHANGES\nFINDINGS\n1. [BLOCKER] COR src/lib.rs:7 — café defect → fix it\n   CHECK: read src/lib.rs:7\nSTATUS: DONE'
    fi
    ;;
  opinion-probe:*)
    if [[ $prompt == *'# SECOND OPINION REQUEST'* || $prompt == *'# CORRECT YOUR SECOND OPINION'* ]]; then
      outcome=${action#opinion-probe:}
      if [[ $outcome == BAD ]]; then
        if [[ ! -f .opinion-corrected ]]; then
          touch .opinion-corrected
          result 'Read src/lib.rs:7; no defect.\nSTATUS: DONE'
        else
          result 'OPINION: NOT CONFIRMED\nRead src/lib.rs:7; café path is unreachable.\nSTATUS: DONE'
        fi
      else
        result "OPINION: $outcome\nRead src/lib.rs:7; café path examined.\nSTATUS: DONE"
      fi
    elif [[ $prompt == *'# OPEN FINDINGS TO VERIFY'* ]]; then
      answers=$(awk '/^# OPEN FINDINGS TO VERIFY/ {open=1} open && /^[0-9]+\. \[/{sub(/\./, "", $1); printf "FINDING %s: RESOLVED\\n", $1}' <<< "$prompt")
      result "VERDICT: APPROVE\n$answers\nSTATUS: DONE"
    else
      result 'VERDICT: REQUEST_CHANGES\nFINDINGS\n1. [BLOCKER] COR src/lib.rs:7 — café defect → fix it\n   CHECK: read src/lib.rs:7\nSTATUS: DONE'
    fi
    ;;
  opinion-probe-hold-verify:*)
    # Like opinion-probe, but the verification of the open findings waits until the test writes verify-release, so
    # the round stays open while the opinions are reported.
    if [[ $prompt == *'# SECOND OPINION REQUEST'* || $prompt == *'# CORRECT YOUR SECOND OPINION'* ]]; then
      outcome=${action#opinion-probe-hold-verify:}
      if [[ $outcome == BAD ]]; then
        if [[ ! -f .opinion-corrected ]]; then
          touch .opinion-corrected
          result 'Read src/lib.rs:7; no defect.\nSTATUS: DONE'
        else
          result 'OPINION: NOT CONFIRMED\nRead src/lib.rs:7; café path is unreachable.\nSTATUS: DONE'
        fi
      else
        result "OPINION: $outcome\nRead src/lib.rs:7; café path examined.\nSTATUS: DONE"
      fi
    elif [[ $prompt == *'# OPEN FINDINGS TO VERIFY'* ]]; then
      while [[ ! -f $home/verify-release ]]; do sleep 0.02; done
      answers=$(awk '/^# OPEN FINDINGS TO VERIFY/ {open=1} open && /^[0-9]+\. \[/{sub(/\./, "", $1); printf "FINDING %s: RESOLVED\\n", $1}' <<< "$prompt")
      result "VERDICT: APPROVE\n$answers\nSTATUS: DONE"
    else
      result 'VERDICT: REQUEST_CHANGES\nFINDINGS\n1. [BLOCKER] COR src/lib.rs:7 — café defect → fix it\n   CHECK: read src/lib.rs:7\nSTATUS: DONE'
    fi
    ;;
  opinion-mixed)
    result 'FINDING 1: FIXED\nFINDING 2: SECOND OPINION — café path is unreachable\nSTATUS: DONE'
    ;;
  opinion-ask|opinion-ask-changed)
    if [[ $action == opinion-ask-changed ]]; then commit_work; record_gate; fi
    answers=$(awk '/^### Review by / {exit} /^[0-9]+\. \[(BLOCKER|MAJOR)\]/ {sub(/\./, "", $1); printf "FINDING %s: SECOND OPINION — café path is unreachable\\n", $1}' <<< "$prompt")
    result "$answers"'STATUS: DONE'
    ;;
  rotating-verify)
    # Each verification resolves the supplied findings and adds a regression in pass two.
    answers=''
    verifying=false
    while IFS= read -r line; do
      if [[ $line == '# OPEN FINDINGS TO VERIFY' ]]; then verifying=true; fi
      if [[ $verifying == true && $line =~ ^([0-9]+)\.\ \[ ]]; then
        answers+="FINDING ${BASH_REMATCH[1]}: RESOLVED\n"
      fi
    done <<< "$prompt"
    if [[ $prompt == *'pass 3'* ]]; then
      result "VERDICT: APPROVE\n$answers\nSUMMARY: verified\nSTATUS: DONE"
    else
      result "VERDICT: REQUEST_CHANGES\n$answers\nFINDINGS\n1. [MAJOR] TDD-2 src/lib.rs:7 — missing regression → add it\n   CHECK: read src/lib.rs:7\nSTATUS: DONE"
    fi
    ;;
  verify:*|late-verify:*)
    # Structured reports answer the supplied permanent numbers before any new findings.
    if [[ $action == late-verify:* ]]; then
      wait_for_peer_review
    fi
    report=${action#*:}
    answers=''
    verifying=false
    while IFS= read -r line; do
      if [[ $line == '# OPEN FINDINGS TO VERIFY' ]]; then verifying=true; fi
      if [[ $verifying == true && $line =~ ^([0-9]+)\.\ \[ ]]; then
        if [[ $report == *REQUEST_CHANGES* ]]; then
          answers+="FINDING ${BASH_REMATCH[1]}: OPEN — still untested\n"
        else
          answers+="FINDING ${BASH_REMATCH[1]}: RESOLVED\n"
        fi
      fi
    done <<< "$prompt"
    if [[ -n $answers ]]; then
      first=${report%%\\n*}
      rest=${report#*\\n}
      report="$first\n$answers$rest"
    fi
    result "$report"'\nSTATUS: DONE'
    ;;
  dirty-done)
    hide_own_files
    printf 'uncommitted delivery work\n' > delivery-uncommitted.txt
    result 'Delivery still has local changes.\nSTATUS: DONE'
    ;;
  done:*) result "${action#done:}"'\nSTATUS: DONE' ;;
  target-done:*)
    # Leaves the build directory a build would leave in the worktree, then finishes with the text after the colon.
    mkdir -p target && printf 'build output\n' > target/output.bin
    result "${action#target-done:}"'\nSTATUS: DONE'
    ;;
  sleep) result 'Nothing to do now.\nSTATUS: SLEEPING' ;;
  fail) result 'I could not do it.\nSTATUS: FAILED' ;;
  nostatus) result 'I think that is all.' ;;
  nostatus-hold-2)
    if ((n == 2)); then
      while [[ ! -f .fake-continue ]]; do
        sleep 0.05
      done
    fi
    result 'I think that is all.'
    ;;
  fail-first:*)
    # Bash noclobber claims the marker with exclusive creation; some mkdir implementations report success to both racers.
    if (set -o noclobber; > "$home/.fake-first-failed") 2> /dev/null; then
      result 'I could not do it.\nSTATUS: FAILED'
    else
      result "${action#fail-first:}"'\nSTATUS: DONE'
    fi
    ;;
  questions) result 'I have to ask.\nSTATUS: QUESTIONS' ;;
  slowdone) sleep 1; result 'Finished the work.\nSTATUS: DONE' ;;
  slowfail) sleep 1; result 'I could not do it.\nSTATUS: FAILED' ;;
  review-reasons)
    if [[ $prompt == *'# OPEN FINDINGS TO VERIFY'* ]]; then
      answers=$(awk '/^# OPEN FINDINGS TO VERIFY/ {open=1} open && /^[0-9]+\. \[/{sub(/\./, "", $1); printf "FINDING %s: RESOLVED — café regression checked\\n", $1}' <<< "$prompt")
      result "VERDICT: APPROVE\n$answers\nSTATUS: DONE"
    else
      result 'VERDICT: REQUEST_CHANGES\nFINDINGS\n1. [MAJOR] TDD-2 src/lib.rs:7 — café regression missing → add it\n   CHECK: read src/lib.rs:7\n2. [MINOR] DOC-1 src/lib.rs:8 — wording → clarify it\n   CHECK: read src/lib.rs:8\nSTATUS: DONE'
    fi
    ;;
  fix-reasons-minor)
    commit_work
    record_gate
    result 'FINDING 1: FIXED — café regression added\nFINDING 2: FIXED - wording clarified\nFINDING 3: FIXED: café regression added\nFINDING 4: FIXED\nSTATUS: DONE'
    ;;
  fix-work|fix-work:*|fix-work-pushed|fix-commit|fix-done|fix-gated|fix-script:*)
    # New fix actions preserve the original actions and answer the ledger before the attached reports.
    case $action in
      fix-done) ;;
      fix-script:*) bash "$home/fake-scripts/${action#fix-script:}.sh"; record_gate ;;
      *) commit_work
         case $action in
           fix-gated) wait_for_gate ;;
           fix-commit) ;;
           *) record_gate ;;
         esac ;;
    esac
    if [[ $action == fix-work-pushed ]]; then
      git "${GIT_FIXTURE[@]}" push -q origin HEAD
      message='Finished the item and pushed it.'
    else
      message='Finished the item.'
    fi
    answers=$(awk '/^### Review by / {exit} /^[0-9]+\. \[(BLOCKER|MAJOR)\]/ {sub(/\./, "", $1); printf "FINDING %s: FIXED\\n", $1}' <<< "$prompt")
    if [[ $action == fix-work:* ]]; then
      answers="FINDING ${action#fix-work:}: FIXED"'\n'
    fi
    result "$message"'\n'"$answers"'STATUS: DONE'
    ;;
  fix-held:*)
    : > ".fake-started-$n"
    for _ in $(seq 1 600); do
      [[ -f ".fake-release-$n" ]] && break
      sleep 0.05
    done
    [[ -f ".fake-release-$n" ]] || { echo "the test never released run $n" >&2; exit 65; }
    commit_work
    record_gate
    result 'Finished the item.\nFINDING '"${action#fix-held:}"': FIXED\nSTATUS: DONE'
    ;;
  answer:*) result 'FINDING '"${action#answer:}"': FIXED\nSTATUS: DONE' ;;
  work)
    commit_work
    record_gate
    result 'Finished the item.\nSTATUS: DONE'
    ;;
  target-work)
    # Like `work`, and leaves the build directory a build would leave, excluded from git so the tree stays clean.
    printf 'target/\n' >> "$(git rev-parse --git-path info/exclude)"
    mkdir -p target && printf 'build output\n' > target/output.bin
    commit_work
    record_gate
    result 'Finished the item.\nSTATUS: DONE'
    ;;
  work-pushed)
    # Like `work`, and pushes the branch itself, as an agent that does not leave the push to the harness.
    commit_work
    record_gate
    git "${GIT_FIXTURE[@]}" push -q origin HEAD
    result 'Finished the item and pushed it.\nSTATUS: DONE'
    ;;
  commit)
    commit_work
    result 'Changed something.\nSTATUS: DONE'
    ;;
  commit-clean)
    hide_own_files
    commit_work
    result 'Changed something and left the tree clean.\nSTATUS: DONE'
    ;;
  break)
    hide_own_files
    echo "run $n" > gate-fails
    git add gate-fails
    git "${GIT_FIXTURE[@]}" commit -q -m "Break the gate in run $n of $name"
    result 'Changed something.\nSTATUS: DONE'
    ;;
  mend)
    hide_own_files
    git rm -q gate-fails
    git "${GIT_FIXTURE[@]}" commit -q -m "Mend the gate in run $n of $name"
    result 'Fixed what the gate found.\nSTATUS: DONE'
    ;;
  gated)
    # Commits like `work`, keeps its own files out of git's sight so that a gate sees a clean tree, says it has committed
    # (`.fake-committed-N`, N the number of the run) and waits for the test to ask the daemon for the gate and to say it ran
    # (`.fake-gate-done-N`).
    commit_work
    wait_for_gate
    result 'Committed, and the gate ran.\nSTATUS: DONE'
    ;;
  integrate-gated)
    # Integrates like `integrate`, and waits for the gate of the daemon like `gated`.
    current=$(git rev-parse --abbrev-ref HEAD)
    for other in $(git for-each-ref --format='%(refname:short)' 'refs/heads/agents/*/task-*-*.*'); do
      [[ $other == "$current" ]] || git "${GIT_FIXTURE[@]}" merge --no-edit "$other" > /dev/null
    done
    wait_for_gate
    result 'Integrated, and the gate ran.\nSTATUS: DONE'
    ;;
  lead-gated)
    # What a coordinator does: integrates what its members did, or, when the prompt is a fix pass of the reviewers, commits a
    # fix; then waits for the daemon's gate like `gated`.
    if grep -q 'THE REVIEWERS ASKED FOR CHANGES' <<< "$prompt"; then
      commit_work
    else
      current=$(git rev-parse --abbrev-ref HEAD)
      for other in $(git for-each-ref --format='%(refname:short)' 'refs/heads/agents/*/task-*-*.*'); do
        [[ $other == "$current" ]] || git "${GIT_FIXTURE[@]}" merge --no-edit "$other" > /dev/null
      done
    fi
    wait_for_gate
    result 'Led the round, and the gate ran.\nSTATUS: DONE'
    ;;
  late:*)
    # Ends after the other reviewer of the pass has had its verdict recorded in `state.json`, then prints the text and STATUS: DONE.
    wait_for_peer_review
    result "${action#late:}"'\nSTATUS: DONE'
    ;;
  script:*)
    bash "$home/fake-scripts/${action#script:}.sh"
    record_gate
    result 'Did the scripted work.\nSTATUS: DONE'
    ;;
  stream:*)
    cat "$home/fake-scripts/${action#stream:}.jsonl"
    result 'Streamed.\nSTATUS: DONE'
    ;;
  ask-here)
    file="$home/tasks/$(basename "$PWD")/questions-1.md"
    mkdir -p "$(dirname "$file")"
    printf '# Questions — fake\n\n## 1. Continue the part?\na) continue — finish the document\nb) stop — leave it open\n\n**Recommended:** a) — finish the independent part (fake)\n\n**Verdict:**\n' > "$file"
    result 'I have to ask.\nSTATUS: QUESTIONS'
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
