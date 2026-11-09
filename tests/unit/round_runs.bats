#!/usr/bin/env bats
# Sub-rounds per run: the start line of every run in activity.jsonl carries the round and the PR the run went into
# ("▶ run 2 (resume) round R12/A pr #74", "pr -" before a PR); a worker or an investigation without a PR of its own
# goes into the PR of the task it was made for, recorded when it was made (REQUESTER_PR). agent tokens --round sums
# the usage per sub-round from these markers; a run without one counts under <round>/?.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
# starts task — the start lines of the task's runs
starts() { jq -r 'select(.k == "info") | .t' "$AGENTS_HOME/tasks/$1/activity.jsonl" | grep '^▶ run'; }
# task id meta — a task directory with that meta.env and an empty activity
task() { mkdir -p "$AGENTS_HOME/tasks/$1"; printf '%s' "$2" > "$AGENTS_HOME/tasks/$1/meta.env"; : > "$AGENTS_HOME/tasks/$1/activity.jsonl"; }
# act id text — an info line in the task's activity
act() { jq -nc --arg t "$2" '{ts: "2026-11-09 10:00:00", k: "info", t: $t}' >> "$AGENTS_HOME/tasks/$1/activity.jsonl"; }

@test "a run's start line carries the round and the PR, or pr - before the task has one" {
  scenario '{"solo": {"new": "quota", "resume": "done"}}'
  agent run solo $'Round: R12/A\nwork' > /dev/null
  t=$(task_of solo)
  wait_status "$t" paused
  [ "$(starts "$t")" = '▶ run 1 (new) round R12/A pr -' ]
  printf 'PR_NUMBER=74\n' >> "$AGENTS_HOME/tasks/$t/meta.env"
  sed -i 's/^WAKE_AT=.*/WAKE_AT=1/' "$AGENTS_HOME/tasks/$t/meta.env"
  agent _wake "$t"
  wait_status "$t" done
  [ "$(starts "$t" | sed -n 2p)" = '▶ run 2 (resume) round R12/A pr #74' ]
  # the parsers of the start line still read it: two runs, the second a resume (with usage: the fake CLI reports none)
  act "$t" 'usage: in 10 (cached 0), out 1'
  run agent tokens --runs "$t"
  [ "$status" -eq 0 ]
  [[ $(grep '^2 ' <<< "$output") == *resume ]]
}

@test "a task without a Round: line marks its runs with round ?" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo 'work' > /dev/null
  t=$(task_of solo)
  wait_status "$t" done
  [ "$(starts "$t")" = '▶ run 1 (new) round ? pr -' ]
}

@test "a worker made while its requester was on PR #74 counts under #74 after the requester moved to #75" {
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}, "worker": {"new": "done"}, "worker2": {"new": "done"}}'
  agent run --team lead,member $'team task\nRound: R12/A' > /dev/null
  m=$(task_of member); l=$(task_of lead); team=$(meta "$m" TEAM)
  wait_status "$m" sleeping
  wait_status "$l" sleeping
  printf 'PR_NUMBER=74\n' >> "$AGENTS_HOME/tasks/$l/meta.env"   # the team's PR, opened by its coordinator
  "$BUILD_DIR/agentbus" delegate --dir "$AGENTS_HOME/bus" --team "$team" --as member --task "$m" --repo repo \
    --branch "$(meta "$m" BRANCH)" --worktree "$(meta "$m" WORKTREE)" --agent-cmd agent --home "$AGENTS_HOME" \
    --request "count the files" --context "c" > /dev/null
  local w=""
  for _ in $(seq 1 100); do
    w=$(grep -lx "PARENT_TASK=$m" "$AGENTS_HOME"/tasks/*/meta.env | xargs -r grep -lx 'KIND=helper' | head -n 1)
    [ -n "$w" ] && break
    sleep 0.2
  done
  w=$(basename "$(dirname "$w")")
  wait_status "$w" 'done|failed'
  [ "$(meta "$w" REQUESTER_PR)" = 74 ]
  [ "$(starts "$w")" = '▶ run 1 (new) round R12/A pr #74' ]
  sed -i 's/^PR_NUMBER=.*/PR_NUMBER=75/' "$AGENTS_HOME/tasks/$l/meta.env"   # the round moved on to another PR
  act "$w" 'usage: in 500 (cached 0), out 5'   # its run's usage (the fake CLI reports none)
  run agent tokens --round R12/A
  [ "$status" -eq 0 ]
  [[ $(grep '^sub-round R12/A/#74 ' <<< "$output") == *" 500 "*" 5 "* ]]
  [[ $output != *"R12/A/#75"* ]]
  agent stop --running > /dev/null
}

# member_fixture — a member of R12/A whose runs went into PR #74, then #75; a task whose runs have no marker (made
# before), one with usage before its first run line, and one whose run came before its PR
member_fixture() {
  task M $'ID=M\nAGENT=member\nKIND=worker\nTEAM=R-team\nROUND_ID=R12/A\n'
  act M '▶ run 1 (new) round R12/A pr #74'
  act M 'usage: in 1000 (cached 100), out 10, cost $0.1000'
  act M '▶ run 2 (resume) round R12/A pr #74'
  act M 'usage: in 2000 (cached 200), out 20, cost $0.2000'
  act M '▶ run 3 (fresh) round R12/A pr #75'
  act M 'usage: in 4000 (cached 400), out 40, cost $0.4000'
  task O $'ID=O\nAGENT=member\nKIND=worker\nROUND_ID=R12/A\n'
  act O '▶ run 1 (new)'
  act O 'usage: in 300 (cached 30), out 3'
  act O 'usage: in 7 (cached 0), out 1'
  task P $'ID=P\nAGENT=member\nKIND=worker\nROUND_ID=R12/A\n'
  act P 'usage: in 60 (cached 6), out 6'
  task Q $'ID=Q\nAGENT=lead\nKIND=worker\nROUND_ID=R12/A\n'
  act Q '▶ run 1 (new) round R12/A pr -'
  act Q 'usage: in 80000 (cached 8), out 8'
}

@test "agent tokens --round: a member's usage split by its runs' PRs; its row keeps the task's total" {
  member_fixture
  run agent tokens --round R12/A
  [ "$status" -eq 0 ]
  echo "$output" >&3
  [[ $(grep '^M ' <<< "$output") == *" 7000 "*" 700 "*" 70 "*"0.7000"* ]]
  [ "$(grep -c '^M ' <<< "$output")" -eq 1 ]   # one row per task
  [[ $(grep '^sub-round R12/A/#74 ' <<< "$output") == *" 3000 "*" 300 "*" 30 "*"0.3000"* ]]
  [[ $(grep '^sub-round R12/A/#75 ' <<< "$output") == *" 4000 "*" 400 "*" 40 "*"0.4000"* ]]
  [[ $(grep '^sub-round R12/A/(no PR) ' <<< "$output") == *" 80000 "* ]]
}

@test "agent tokens --round: runs without a marker count under <round>/?, never dropped; the total unchanged" {
  member_fixture
  run agent tokens --round R12/A
  [ "$status" -eq 0 ]
  [[ $(grep '^sub-round R12/A/? ' <<< "$output") == *" 367 "*" 36 "*" 10 "* ]]
  [[ $(grep '^total ' <<< "$output") == *" 87367 "*" 744 "*" 88 "* ]]
}

@test "docs/node-improvements.md lists the nodes' future work, first the usage of remote members; multi-instance links it" {
  local f=$REPO_ROOT/docs/node-improvements.md
  [ -f "$f" ]
  grep -q 'run marker' "$f"
  grep -q 'synchronis' "$f"
  grep -q 'node-improvements.md' "$REPO_ROOT/docs/multi-instance.md"
}
