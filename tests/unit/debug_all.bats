#!/usr/bin/env bats
# agent debug is to show everything needed to diagnose a task or a team. It left out the most important: the prompt —
# the task's text, the request a worker got —, so whether a worker was right to ask could not be told. Now each task
# shows its own text (by default the task's part of task.md; with --full, or --all, the whole task.md and every run's
# prompt), its request on the bus in full (request, context, result, rating, links) and the requests it made, its
# questions and the human's answers, its checkpoint and its gate results, its other files, and the bus messages to
# and from its agent; and a task's debug takes in the tasks made for it — workers, investigations, reviews — at any
# depth.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# solo_waiting — a solo task with a text of its own, waiting on its questions; sets t
solo_waiting() {
  scenario '{"solo": {"new": "questions-file"}}'
  agent run solo "count the widgets in folder XYZ7781" > /dev/null
  t=$(task_of solo)
  wait_status "$t" waiting
}

# debug_of task [args] — agent debug of the task into D (the output file)
debug_of() {
  D="$BATS_TEST_TMPDIR/debug-$RANDOM.txt"
  agent debug "$@" --out "$D" > /dev/null
}

@test "the task's text by default; with --full, the whole task.md and every run's prompt" {
  solo_waiting
  debug_of "$t"
  grep -q "== task $t" "$D"
  grep -q "count the widgets in folder XYZ7781" "$D"
  ! grep -q "WORKING CONTEXT" "$D" || false   # the harness's own text only with --full
  grep -q -- "-- prompt of run 1: prompt-1.md, [0-9]* lines" "$D"
  debug_of "$t" --full
  grep -q "WORKING CONTEXT" "$D"
  grep -q -- "-- prompt of run 1: prompt-1.md" "$D"
  agent stop --running > /dev/null
}

@test "--all is --full" {
  solo_waiting
  debug_of "$t" --all
  grep -q "WORKING CONTEXT" "$D"
  agent stop --running > /dev/null
}

@test "a task's questions and the human's answers, its checkpoint and its gate results" {
  solo_waiting
  mkdir -p "$AGENTS_HOME/bus/checkpoints"
  printf 'CHECKPOINT-4411: half done\n' > "$AGENTS_HOME/bus/checkpoints/task-$t.md"
  echo "AGENT_REVIEW_GATE='true'" >> "$AGENTS_HOME/agents.conf"
  git -C "$(meta "$t" WORKTREE)" -c user.name=t -c user.email=t@t commit -q --allow-empty -m c
  agent gate "$t" > /dev/null
  debug_of "$t"
  grep -q -- "-- questions-1.md" "$D"
  grep -q "## 1. Which board?" "$D"
  grep -q "CHECKPOINT-4411: half done" "$D"
  grep -q -- "-- gate results" "$D"
  grep -qi "pass" <(sed -n '/-- gate results/,/^--/p' "$D")
  agent stop --running > /dev/null
}

# asked — a team whose member asks a worker, which answers with its questions; sets m and w (the worker's task)
asked() {
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}, "worker": {"new": "unclear"}, "worker2": {"new": "unclear"}}'
  agent run --team lead,member "team task" > /dev/null
  m=$(task_of member)
  wait_status "$m" sleeping
  "$BUILD_DIR/agentbus" delegate --dir "$AGENTS_HOME/bus" --team "$(meta "$m" TEAM)" --as member --task "$m" \
    --repo repo --branch "$(meta "$m" BRANCH)" --worktree "$(meta "$m" WORKTREE)" --agent-cmd agent \
    --home "$AGENTS_HOME" --request "count the files of PLK-3310" --context "c" > /dev/null
  for _ in $(seq 1 150); do
    w=$(grep -lx 'KIND=helper' "$AGENTS_HOME"/tasks/*/meta.env 2> /dev/null | head -n 1 | xargs -r dirname | xargs -r basename)
    [ -n "$w" ] && [ "$(meta "$w" STATUS)" = "done" ] && return 0
    sleep 0.2
  done
  return 1
}

@test "a worker's task shows its request on the bus in full: the request, its result and state" {
  asked
  debug_of "$w"
  grep -q -- "-- its request on the bus" "$D"
  grep -q "count the files of PLK-3310" "$D"
  grep -q "Which directory holds the files" "$D"   # the worker's result: its questions
  grep -q '"status": "unclear"' "$D"
  agent stop --running > /dev/null
}

@test "a requester's task shows the requests it made, and the bus messages to and from its agent" {
  asked
  wait_grep "RESULT H1" "$AGENTS_HOME/bus/messages.jsonl" 30
  debug_of "$m"
  grep -q -- "-- requests it made" "$D"
  grep -q "count the files of PLK-3310" "$D"
  grep -q -- "-- bus messages to and from member" "$D"
  grep -q "RESULT H1" <(sed -n '/-- bus messages to and from member/,/^--/p' "$D")
  agent stop --running > /dev/null
}

@test "a task's debug takes in the tasks made for it — workers, investigations, reviews — at any depth" {
  solo_waiting
  for kid in "$t:20261110-000001-inv" "20261110-000001-inv:20261110-000002-review"; do
    parent=${kid%%:*} id=${kid#*:}
    mkdir -p "$AGENTS_HOME/tasks/$id"
    printf 'ID=%s\nAGENT=inv\nKIND=investigation\nPARENT_TASK=%s\nSTATUS=done\n' "$id" "$parent" \
      > "$AGENTS_HOME/tasks/$id/meta.env"
    printf '# INVESTIGATION REQUEST #9\n\nwhy does %s fail\n' "$id" > "$AGENTS_HOME/tasks/$id/task.md"
  done
  debug_of "$t"
  grep -q "== task 20261110-000001-inv" "$D"
  grep -q "== task 20261110-000002-review" "$D"
  grep -q "why does 20261110-000002-review fail" "$D"
  agent stop --running > /dev/null
}
