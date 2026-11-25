#!/usr/bin/env bash
# A stand-in for the `claude` CLI, run by the tests of the run engine and of the daemon: it ends the way the file
# `.fake-scenario` in its working directory says, speaking Claude Code's stream JSON. A committed script, never written
# by a test, so that no test races an `exec` of a file that is still open for writing.
#
#   done                  a short run that ends with STATUS: DONE
#   done-child            ends like `done` after starting a child whose PID is recorded in .fake-child
#   done-after-ok          run 1 ends like `done`; a later invocation waits for ok.txt to appear before it ends (count in .fake-runs)
#   fail                  ends with STATUS: FAILED
#   fail-tool             like `fail`, after streaming one tool call (a tool_use block): a failed run the harness counts as
#                         one that used a tool, so a task that fails this way is not retried for "no tool call"
#   nostatus              ends cleanly without a STATUS line
#   crash                 exits with status 3 and prints nothing
#   garbage               prints a line that is not JSON, then ends like `done`
#   flood                 prints three lines of 6 MB that are not JSON, then ends like `done`
#   questions PATH        writes a questions file to PATH, ends with STATUS: QUESTIONS
#   questions-nofile      ends with STATUS: QUESTIONS without writing the file
#   busy                  says something every 0.2 s until it is stopped: a run that is going and not hung
#   hang                  prints its first line and then nothing; leaves a child behind (pid in .fake-child)
#   hang-slow-stop        hangs; when it is told to stop it writes .fake-term and takes two seconds to end
#   hang-first            hangs when started, ends like `done` when resumed (--resume)
#   hang-then-step        run 1 hangs at once, run 2 says something and hangs, run 3 ends like `done` (count in .fake-runs)
#   quota                 the provider says the usage limit is reached and resets in 3 hours
#   ratelimit             the provider answers HTTP 429
#   env                   ends like `done` after writing PATH and FAKE_SECRET to .fake-env
#   env-git               ends like `done` after writing to .fake-git who a commit would be by and how git would sign it
#   env-home              ends like `done` after writing AGENTS_HOME to .fake-home
#   commit                commits work.txt in the repository it runs in, leaves left.txt uncommitted, ends like `done`
#   commit-frozen-test    rewrites tests/a.rs, tries to commit it with `git commit`, writes git's exit status to
#                         .fake-commit-code and what git said to .fake-commit-err, ends like `done`
#   commit-busy           commits work.txt, leaves left.txt, then says something every 0.2 s until it is stopped
#   context-from-file     reads the size N in `.fake-context`, reports one model call whose input is N tokens (usage with
#                         its ID, and a cost of 0.5 dollars in the result), ends like `done`
#   two-contexts          reports two model calls, the first with an input of 100 tokens and the second with 200, ends like
#                         `done`
#   serena-reads          reads forty-two code files of `src` with its `Read` tool, then ends like `done`
#   serena-calls          calls a tool of Serena, then reads code files as `serena-reads` does
#   serena-text           reads forty-two text files as `serena-reads` reads code ones
#   serena-writes         writes and edits files of code forty-two times, ends like `done`
#   serena-three          reads three code files, then ends like `done`
#   work                  prints a message every 0.2 s until it is stopped: a run that does work
#   hold                  prints its first line and then waits for .fake-continue to appear before it ends like `done`
#   dirty-hold            writes dirty.txt of 400 lines and leaves it uncommitted, says something every 0.2 s until
#                         .fake-continue exists, then ends like `done`
#   dirty-commit-hold     writes dirty.txt of 400 lines and commits it, then holds and ends as `dirty-hold` does
#   dirty-twice           writes dirty.txt of 400 lines, holds until .fake-commit exists, commits dirty.txt, writes
#                         second.txt of 400 lines, then holds and ends as `dirty-hold` does
#   dirty-late            holds until .fake-more exists, then writes dirty.txt of 400 lines, holds until .fake-continue
#                         exists and ends like `done`
#
# Every run records its arguments, one a line, in `.fake-args`.
set -eu

# Writes the values, one a line, into the file whole: staged in a same-directory temporary and renamed over the file,
# so a reader that looks at the record while the run is still writing it sees the whole record or no file, never a
# torn one.
whole() {
  local file temporary
  file=$1
  shift
  temporary=$(mktemp "$file.XXXXXXXXXX")
  printf '%s\n' "$@" > "$temporary"
  mv -- "$temporary" "$file"
}

scenario=$(cat .fake-scenario)
whole .fake-args "$@"

session=""
resumed=no
while (($#)); do
  case $1 in
    --session-id) session=$2; shift ;;
    --resume) session=$2; resumed=yes; shift ;;
  esac
  shift
done

init() {
  printf '{"type":"system","subtype":"init","session_id":"%s","model":"fake-model"}\n' "$session"
}

say() {
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"%s"}]},"session_id":"%s"}\n' "$1" "$session"
}

# result IS_ERROR TEXT — TEXT is JSON text: `\n` is a line break.
result() {
  printf '{"type":"result","subtype":"success","is_error":%s,"result":"%s","session_id":"%s"}\n' "$1" "$2" "$session"
}

finish_done() {
  init
  say "working"
  result false 'Finished the item.\nSTATUS: DONE'
}

# Commits a file and leaves another in the tree: what a run that did some work leaves.
leave_work() {
  echo "the work" > work.txt
  git add work.txt
  git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q -m "Work of the run"
  printf 'left over\nin two lines\n' > left.txt
}

# Commits what is staged, dated long before any clock of a test, so that the run has made no commit for a long time.
commit_long_ago() {
  GIT_AUTHOR_DATE="2020-01-01T00:00:00Z" GIT_COMMITTER_DATE="2020-01-01T00:00:00Z" \
    git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q -m "Work of the run"
}

# Writes the file of 400 lines the commit reminder counts.
write_400_lines() {
  seq 1 400 > "$1"
}

# Says something every 0.2 s until the file named exists, so that the watchdog never takes the run for hung; fails when it
# does not appear in a minute.
hold_until() {
  local file=$1 _
  for _ in $(seq 1 300); do
    [[ -f $file ]] && return 0
    say "working"
    sleep 0.2
  done
  echo "the test never created $file" >&2
  exit 65
}

# read_code FIRST N — tool_use blocks of the Read tool over the code files of the worktree, one a line, the first
# numbered FIRST.
read_code() {
  local first=$1 n=$2 i
  for ((i = first; i < first + n; i++)); do
    printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t%d","name":"Read","input":{"file_path":"%s"}}]}}\n' \
      "$i" "src/module-$((i % 3)).rs"
  done
}

# read_text FIRST N — tool_use blocks of the Read tool over text files, one a line, the first numbered FIRST.
read_text() {
  local first=$1 n=$2 i
  for ((i = first; i < first + n; i++)); do
    printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t%d","name":"Read","input":{"file_path":"%s"}}]}}\n' \
      "$i" "docs/note-$((i % 3)).md"
  done
}

case $scenario in
  done) finish_done ;;
  serena-reads)
    init
    read_code 0 42
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  serena-writes)
    init
    for i in $(seq 1 42); do
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"w%s","name":"Write","input":{"file_path":"src/written-%s.rs"}}]}}\n' "$i" "$i"
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"e%s","name":"Edit","input":{"file_path":"src/module-1.rs"}}]}}\n' "$i"
    done
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  serena-three)
    init
    read_code 0 3
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  serena-calls)
    init
    printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"s1","name":"mcp__serena__find_symbol","input":{}}]}}'
    read_code 0 42
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  serena-text)
    init
    read_text 0 42
    result false 'Finished the item.\nSTATUS: DONE'
    ;;

  done-child)
    sleep 600 &
    whole .fake-child "$!"
    finish_done
    ;;
  done-after-ok)
    # Run 1 ends like `done` at once, so the first mismatch sends the run back once and its test observes it; every
    # later invocation waits for the test's ok.txt before it ends, so a bounce-back run can never consume the bounce
    # budget while the file is still absent.
    runs=1
    if [[ -f .fake-runs ]]; then runs=$(( $(< .fake-runs) + 1 )); fi
    whole .fake-runs "$runs"
    if ((runs == 1)); then
      finish_done
    else
      init
      for _ in $(seq 1 600); do
        [[ -f ok.txt ]] && break
        sleep 0.05
      done
      [[ -f ok.txt ]] || { echo "the test never created ok.txt for run $runs" >&2; exit 65; }
      say "working"
      result false 'Finished the item.\nSTATUS: DONE'
    fi
    ;;
  fail) init; result false 'I could not do it.\nSTATUS: FAILED' ;;
  fail-tool)
    init
    printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"mcp__agentbus__read_messages","input":{}}]}}'
    result false 'I could not do it.\nSTATUS: FAILED'
    ;;
  sleep) init; result false 'Going to sleep.\nSTATUS: SLEEPING' ;;
  nostatus) init; result false 'I think that is all.' ;;
  crash) exit 3 ;;
  garbage) echo 'this line is not JSON'; finish_done ;;
  flood)
    for _ in 1 2 3; do head -c 6000000 /dev/zero | tr '\0' x; echo; done
    finish_done
    ;;
  questions\ *)
    init
    path=${scenario#questions }
    mkdir -p "$(dirname "$path")"
    printf '# Questions — fake\n\n## 1. Which store?\na) memory\nb) disk\n\n**Recommended:** a) — simpler (fake)\n\n**Verdict:**\n' > "$path"
    result false 'I need answers.\nSTATUS: QUESTIONS'
    ;;
  questions-nofile) init; result false 'I need answers.\nSTATUS: QUESTIONS' ;;
  busy)
    init
    while true; do
      say "working"
      sleep 0.2
    done
    ;;
  hang)
    init
    sleep 600 &
    whole .fake-child "$!"
    wait
    ;;
  hang-slow-stop)
    init
    trap 'touch .fake-term; sleep 2; exit 0' TERM
    sleep 600 &
    whole .fake-child "$!"
    while true; do wait; done
    ;;
  hang-first)
    if [[ $resumed == yes ]]; then
      finish_done
    else
      init
      sleep 600 &
      whole .fake-child "$!"
      wait
    fi
    ;;
  hang-then-step)
    runs=1
    if [[ -f .fake-runs ]]; then runs=$(( $(< .fake-runs) + 1 )); fi
    whole .fake-runs "$runs"
    if ((runs >= 3)); then
      finish_done
    else
      init
      if ((runs == 2)); then say "working"; fi
      sleep 600 &
      whole .fake-child "$!"
      wait
    fi
    ;;
  quota)
    init
    result true "You've hit your usage limit. Your limit resets in 3 hours."
    exit 1
    ;;
  ratelimit)
    init
    result true 'API Error: 429 rate limit exceeded'
    exit 1
    ;;
  env)
    printf 'PATH=%s\nFAKE_SECRET=%s\n' "$PATH" "${FAKE_SECRET:-}" > .fake-env
    finish_done
    ;;
  agent)
    agent > .fake-agent
    finish_done
    ;;
  env-git)
    {
      printf 'author=%s <%s>\n' "$(git var GIT_AUTHOR_IDENT | sed 's/ <.*//')" "${GIT_AUTHOR_EMAIL:-}"
      printf 'committer=%s <%s>\n' "$(git var GIT_COMMITTER_IDENT | sed 's/ <.*//')" "${GIT_COMMITTER_EMAIL:-}"
      printf 'gpgsign=%s\n' "$(git config --get commit.gpgsign || true)"
      printf 'format=%s\n' "$(git config --get gpg.format || true)"
      printf 'key=%s\n' "$(git config --get user.signingkey || true)"
    } > .fake-git
    finish_done
    ;;
  commit)
    init
    leave_work
    say "committed"
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  commit-frozen-test)
    init
    printf 'REWRITTEN\n' > tests/a.rs
    git add tests/a.rs
    code=0
    git commit -q -m "Format the test" 2> .fake-commit-err || code=$?
    printf '%s\n' "$code" > .fake-commit-code
    say "tried to commit"
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  commit-busy)
    init
    leave_work
    while true; do
      say "working"
      sleep 0.2
    done
    ;;
  context-from-file)
    context=$(< .fake-context)
    init
    printf '{"type":"assistant","message":{"id":"msg-%s","role":"assistant","content":[{"type":"text","text":"working"}],"usage":{"input_tokens":%s,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":100}},"session_id":"%s"}\n' "$context" "$context" "$session"
    printf '{"type":"result","subtype":"success","is_error":false,"result":"Finished the item.\\nSTATUS: DONE","session_id":"%s","total_cost_usd":0.5}\n' "$session"
    ;;
  two-contexts)
    init
    for context in 100 200; do
      printf '{"type":"assistant","message":{"id":"msg-%s","role":"assistant","content":[{"type":"text","text":"working"}],"usage":{"input_tokens":%s,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":10}},"session_id":"%s"}\n' "$context" "$context" "$session"
    done
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  env-incremental)
    printf '%s\n' "${CARGO_INCREMENTAL:-unset}" > .fake-incremental
    finish_done
    ;;
  env-home)
    printf '%s\n' "${AGENTS_HOME:-unset}" > .fake-home
    finish_done
    ;;
  done-on-resume-after\ *)
    path=${scenario#done-on-resume-after }
    init
    if [[ $resumed == yes ]]; then
      while [[ ! -f $path ]]; do
        say "working"
        sleep 0.02
      done
    fi
    say "working"
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  work)
    init
    while true; do
      say "working"
      sleep 0.2
    done
    ;;
  hold)
    init
    say "working"
    for _ in $(seq 1 600); do
      [[ -f .fake-continue ]] && break
      sleep 0.02
    done
    [[ -f .fake-continue ]] || { echo "the test never created .fake-continue" >&2; exit 65; }
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  dirty-hold)
    init
    write_400_lines dirty.txt
    hold_until .fake-continue
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  dirty-commit-hold)
    init
    write_400_lines dirty.txt
    git add dirty.txt
    commit_long_ago
    hold_until .fake-continue
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  dirty-twice)
    init
    write_400_lines dirty.txt
    hold_until .fake-commit
    git add dirty.txt
    commit_long_ago
    write_400_lines second.txt
    hold_until .fake-continue
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  dirty-late)
    init
    hold_until .fake-more
    write_400_lines dirty.txt
    hold_until .fake-continue
    result false 'Finished the item.\nSTATUS: DONE'
    ;;
  *) echo "unknown scenario: $scenario" >&2; exit 64 ;;
esac
