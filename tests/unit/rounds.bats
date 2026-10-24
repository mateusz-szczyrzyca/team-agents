#!/usr/bin/env bats
# "done" is a status name passed as an argument (wait_status "$t" done), not the end of a loop.
# shellcheck disable=SC1010
# Rounds (README "Rounds"): a task's PR stays under the round limit; ROUND_DONE ends a round; --resume continues
# only after the round's PR is merged, on fresh branches — or with --same-round on the same ones.

load helpers

# meta id key — a value from a task's meta.env
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# latest agent — the newest task of an agent (task IDs start with their time, so the last one in order)
latest() {
  local d last=""
  for d in "$AGENTS_HOME"/tasks/*-"$1"; do [[ -d $d ]] && last=${d##*/}; done
  printf '%s' "$last"
}

@test "a change over the round limit goes back to be split; after the allowed corrections it passes with a warning" {
  printf 'ROUND_MAX_LINES=10\nROUND_SOFT_LINES=5\nROUND_BOUNCES=1\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "commit-lines:30", "resume": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  [ "$(meta "$t" RUN)" = 2 ]
  grep -q "30 lines" "$HOME/prompt-solo-resume.txt"
  grep -q "over the round limit" "$HOME/prompt-solo-resume.txt"
  wait_grep "over the round limit" "$HOME/notifications.log"
}

@test "a repository can have no round limit" {
  printf 'ROUND_MAX_LINES=10\ndeclare -A ROUND_MAX_LINES_REPO=([repo]=0)\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "commit-lines:30"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  [ "$(meta "$t" RUN)" = 1 ]
}

@test "agent round-size shows the size of a task's change and its limit" {
  printf 'ROUND_MAX_LINES=100\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "commit-lines:30"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  run agent round-size "$t"
  [ "$status" -eq 0 ]
  [[ $output == *"round size: 30 lines"* && $output == *"hard limit 100"* ]]
}

@test "ROUND_DONE N/M finishes a round and tells the human how to continue" {
  scenario '{"solo": {"new": "round-done:1/3"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  [ "$(meta "$t" WORK_ROUND)" = 1 ]
  [ "$(meta "$t" WORK_ROUNDS)" = 3 ]
  wait_grep "round 1/3" "$HOME/notifications.log"
  grep -q -- "--same-round" "$HOME/notifications.log"
}

@test "a skip added in a round goes back while the PR does not list it — whatever round its marker names" {
  printf 'ROUND_BOUNCES=1\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "pending:1:1/2", "resume": "round-done:1/2"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  [ "$(meta "$t" RUN)" = 2 ]
  grep -q "pending: parser, round 1" "$HOME/prompt-solo-resume.txt"
}

@test "resuming a round whose PR is not merged stops; --same-round continues on the same branch" {
  scenario '{"solo": {"new": "round-done:1/2"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  echo '[{"number": 5, "url": "https://github.com/o/r/pull/5", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  run agent run --resume "$t" < /dev/null   # no addition: --resume would read one from stdin
  [ "$status" -ne 0 ]
  [[ $output == *"not merged"* && $output == *"--same-round"* ]]

  scenario '{"solo": {"new": "done"}}'
  agent run --resume "$t" --same-round "fix the review findings"
  t2=$(latest solo)
  [ "$t2" != "$t" ]
  [ "$(meta "$t2" BRANCH)" = "$(meta "$t" BRANCH)" ]
  wait_status "$t2" done
}

@test "after a merged round the next one starts on a fresh branch from the updated base; after the last one, no more" {
  scenario '{"solo": {"new": "round-done:1/2"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  # the human merges round 1: the base moves on
  cd "$HOME/repo" && git -c user.name=t -c user.email=t@t commit -q --allow-empty -m "Merge round 1" && git push -q origin main
  echo '[{"number": 5, "url": "https://github.com/o/r/pull/5", "state": "MERGED"}]' > "$HOME/gh-prs.json"
  scenario '{"solo": {"new": "round-done:2/2"}}'
  agent run --resume "$t" < /dev/null
  t2=$(latest solo)
  [ "$t2" != "$t" ]
  [ "$(meta "$t2" BRANCH)" != "$(meta "$t" BRANCH)" ]
  [ "$(meta "$t2" PREV_BRANCH)" = "$(meta "$t" BRANCH)" ]
  [ "$(git -C "$HOME/repo" rev-parse "$(meta "$t2" BRANCH)")" = "$(git -C "$HOME/repo" rev-parse origin/main)" ]
  grep -q "Round 2 of 2" "$AGENTS_HOME/tasks/$t2/task.md"
  wait_status "$t2" done

  run agent run --resume "$t2" < /dev/null
  [ "$status" -ne 0 ]
  [[ $output == *"all 2 rounds are done"* ]]
}
