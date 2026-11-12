#!/usr/bin/env bash
# A stand-in for the `claude` CLI, run by the tests of the run engine and of the daemon: it ends the way the file
# `.fake-scenario` in its working directory says, speaking Claude Code's stream JSON. A committed script, never written
# by a test, so that no test races an `exec` of a file that is still open for writing.
#
#   done                  a short run that ends with STATUS: DONE
#   done-child            ends like `done`, leaving a child behind (pid in .fake-child)
#   fail                  ends with STATUS: FAILED
#   nostatus              ends cleanly without a STATUS line
#   crash                 exits with status 3 and prints nothing
#   garbage               prints a line that is not JSON, then ends like `done`
#   questions PATH        writes a questions file to PATH, ends with STATUS: QUESTIONS
#   questions-nofile      ends with STATUS: QUESTIONS without writing the file
#   hang                  prints its first line and then nothing; leaves a child behind (pid in .fake-child)
#   hang-first            hangs when started, ends like `done` when resumed (--resume)
#   quota                 the provider says the usage limit is reached and resets in 3 hours
#   ratelimit             the provider answers HTTP 429
#   env                   ends like `done` after writing PATH and FAKE_SECRET to .fake-env
#
# Every run records its arguments, one a line, in `.fake-args`.
set -eu

scenario=$(cat .fake-scenario)
printf '%s\n' "$@" > .fake-args

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

case $scenario in
  done) finish_done ;;
  done-child)
    sleep 600 &
    echo $! > .fake-child
    finish_done
    ;;
  fail) init; result false 'I could not do it.\nSTATUS: FAILED' ;;
  nostatus) init; result false 'I think that is all.' ;;
  crash) exit 3 ;;
  garbage) echo 'this line is not JSON'; finish_done ;;
  questions\ *)
    init
    path=${scenario#questions }
    mkdir -p "$(dirname "$path")"
    printf '# Questions — fake\n\n## 1. Which store?\na) memory\nb) disk\n\n**Recommended:** a) — simpler (fake)\n\n**Verdict:**\n' > "$path"
    result false 'I need answers.\nSTATUS: QUESTIONS'
    ;;
  questions-nofile) init; result false 'I need answers.\nSTATUS: QUESTIONS' ;;
  hang)
    init
    sleep 600 &
    echo $! > .fake-child
    wait
    ;;
  hang-first)
    if [[ $resumed == yes ]]; then
      finish_done
    else
      init
      sleep 600 &
      echo $! > .fake-child
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
  *) echo "unknown scenario: $scenario" >&2; exit 64 ;;
esac
