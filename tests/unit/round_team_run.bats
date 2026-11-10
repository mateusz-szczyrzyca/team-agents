#!/usr/bin/env bats
# Round accounting on tasks the script makes itself (fake CLIs, no artificial metadata): an investigation team of
# round R20/inv, then a team of round R20/A whose member asks a worker and whose coordinator asks a review of its PR.
# agent tokens <team>, agent tokens <investigation team> and agent tokens --round R20 list every task with its role
# and round, and their sums match the usage of the runs.

load helpers
load investigation_pipeline_fixture

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
# usage task n — a usage line of n input tokens (n/10 cached, n/100 out) in the task's activity: the fake CLIs report
# none. It follows the task's last run line, so it counts under that run's marker.
usage() {
  jq -nc --arg t "usage: in $2 (cached $(($2 / 10))), out $(($2 / 100))" '{ts: "2026-11-10 10:00:00", k: "info", t: $t}' \
    >> "$AGENTS_HOME/tasks/$1/activity.jsonl"
}
# row task — the role and the round of the task's row in the last report (its last two columns); "" no row
row() { awk -v t="$1" '$1 == t {print $(NF - 1), $NF}' <<< "$output"; }
# sums label — the "<label> … in cached out" line of the last report, its numbers only
sums() { grep "^$1 " <<< "$output" | grep -oE '[0-9]+ +[0-9]+ +[0-9]+ +[0-9.]+$' | awk '{print $1, $2, $3}'; }
# child parent line — the first task made for parent whose meta.env has that line; "" none yet
child() {
  local f
  f=$(grep -lx "PARENT_TASK=$1" "$AGENTS_HOME"/tasks/*/meta.env | xargs -r grep -lx -- "$2" | head -n 1)
  [[ -z $f ]] || basename "$(dirname "$f")"
}

@test "a team run and an investigation team of round R20: every task with its role, its round and the right sums" {
  # the investigation team first: the pipeline's CLIs stand in for claude while it runs
  single_pipeline_setup
  run agent run --investigation --team inv $'Investigate a temporary fixture\nRound: R20/inv'
  [ "$status" -eq 0 ]
  local ip; ip=$(task_of investigation-team)
  wait_status "$ip" '^(done|failed)$' 45
  local -a phases=()
  mapfile -t phases < <(grep -lx "INVESTIGATION_TEAM=$ip" "$AGENTS_HOME"/tasks/*/meta.env | xargs -n1 dirname | xargs -n1 basename)
  ((${#phases[@]} >= 3))   # phase 1, the merger, the reviewer

  # then the team, on the ordinary fake CLI
  cp "$BATS_TEST_DIRNAME/stubs/fake-claude" "$HOME/.local/bin/claude"
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}, "worker": {"new": "done"}, "worker2": {"new": "done"}, "inv": {"new": "review-ready"}}'
  agent run --team lead,member $'Build it.\nRound: R20/A' > /dev/null
  local l m team
  l=$(task_of lead); m=$(task_of member); team=$(meta "$m" TEAM)
  wait_status "$l" sleeping
  wait_status "$m" sleeping
  printf 'PR_NUMBER=74\n' >> "$AGENTS_HOME/tasks/$l/meta.env"   # the team's PR, opened by its coordinator
  "$BUILD_DIR/agentbus" delegate --dir "$AGENTS_HOME/bus" --team "$team" --as member --task "$m" --repo repo \
    --branch "$(meta "$m" BRANCH)" --worktree "$(meta "$m" WORKTREE)" --agent-cmd agent --home "$AGENTS_HOME" \
    --request "count the files" --context "c" > /dev/null
  "$BUILD_DIR/agentbus" inv-add --dir "$AGENTS_HOME/bus" --kind review --from-agent lead --from-task "$l" --repo repo \
    --branch "$(meta "$l" BRANCH)" --tasks-dir "$AGENTS_HOME/tasks" --question "review it" \
    --pr-url https://github.com/example/repo/pull/74 --pr-number 74 > /dev/null
  agent dispatch > /dev/null
  local w="" r=""
  for _ in $(seq 1 150); do
    w=$(child "$m" KIND=helper); r=$(child "$l" REVIEW=1)
    [[ -n $w && -n $r ]] && break
    sleep 0.2
  done
  [[ -n $w && -n $r ]] || { ls "$AGENTS_HOME/tasks"; false; }
  wait_status "$w" '^(done|failed)$'
  wait_status "$r" '^(done|failed)$'
  [ "$(meta "$r" REVIEW)" = 1 ]

  # the usage of every run (each task's own amount, so a sum shows which tasks it took)
  local -A in=([$l]=100000 [$m]=20000 [$w]=3000 [$r]=400 [$ip]=0)
  local t n=50
  for t in "${phases[@]}"; do in[$t]=$n; n=$((n + 10)); done
  for t in "${!in[@]}"; do ((in[$t] == 0)) || usage "$t" "${in[$t]}"; done

  # the team: its four tasks, never the investigation team's
  run agent tokens "$team"
  [ "$status" -eq 0 ]
  echo "$output" >&3
  [ "$(row "$l")" = "coordinator R20/A" ]
  [ "$(row "$m")" = "member R20/A" ]
  [ "$(row "$w")" = "worker R20/A" ]
  [ "$(row "$r")" = "review R20/A" ]
  [[ -z $(row "$ip") ]]
  [ "$(sums total)" = "123400 12340 1234" ]

  # the investigation team: its parent and every phase — the tasks agent tokens <team> used to leave out
  run agent tokens "$ip"
  [ "$status" -eq 0 ]
  echo "$output" >&3
  [ "$(row "$ip")" = "investigation R20/inv" ]
  local inv_in=0 inv_cached=0 inv_out=0
  for t in "${phases[@]}"; do
    local want=investigation
    [[ $(meta "$t" REVIEW) != 1 ]] || want=review   # the investigation team's reviewer
    [ "$(row "$t")" = "$want R20/inv" ] || { echo "phase $t: '$(row "$t")', not '$want R20/inv'"; false; }
    inv_in=$((inv_in + in[$t])); inv_cached=$((inv_cached + in[$t] / 10)); inv_out=$((inv_out + in[$t] / 100))
  done
  [ "$(sums total)" = "$inv_in $inv_cached $inv_out" ]

  # the round: both, each task once; per role, per model, per sub-round and in all the same tokens
  run agent tokens --round R20
  [ "$status" -eq 0 ]
  echo "$output" >&3
  for t in "${!in[@]}"; do [ "$(grep -c "^$t " <<< "$output")" -eq 1 ] || { echo "rows of $t"; false; }; done
  local all_in=$((123400 + inv_in)) all_cached=$((12340 + inv_cached)) all_out=$((1234 + inv_out))
  [ "$(sums total)" = "$all_in $all_cached $all_out" ]
  [ "$(sums 'role coordinator')" = "100000 10000 1000" ]
  [ "$(sums 'role member')" = "20000 2000 200" ]
  [ "$(sums 'role worker')" = "3000 300 30" ]
  # the sums of each kind add up to the total
  local kind
  for kind in role model sub-round; do
    [ "$(grep "^$kind " <<< "$output" | grep -oE '[0-9]+ +[0-9]+ +[0-9]+ +[0-9.]+$' \
      | awk '{i += $1; c += $2; o += $3} END {print i, c, o}')" = "$all_in $all_cached $all_out" ] \
      || { echo "the $kind sums do not add up to the total"; false; }
  done
  # the sub-rounds: the coordinator's and the member's runs started before the PR, the worker's and the review's in it
  [ "$(sums 'sub-round R20/A/(no PR)')" = "120000 12000 1200" ]
  [ "$(sums 'sub-round R20/A/#74')" = "3400 340 34" ]
  agent stop --running > /dev/null
}
