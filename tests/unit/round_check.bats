#!/usr/bin/env bats
# Was a round's work split as planned, done once, merged? Two members' work — 25 commits — was found thrown away only
# by reading the coordinator's report. agent round-check <team> shows the round's card: its board (items, owners,
# done), each member's work not in the round's PR, the files more than one branch changed, the workers' items, each
# member's tokens; the coordinator's notification of a round carries its summary. It reads the tasks and branches a
# finished round left, so earlier rounds can be checked too.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
set_meta() { (source "$REPO_ROOT/bin/agent"; meta_set "$1" "$2" "$3"); }

# round_with_lost_work — a team whose coordinator's branch holds the round's PR, and whose member has two commits that
# are not in it, on a file the coordinator changed too
round_with_lost_work() {
  scenario '{"lead": {"new": "done"}, "member": {"new": "done"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member)
  wait_status "$t" 'done'
  wait_status "$m" 'done'
  team=$(meta "$t" TEAM)
  local lw mw
  lw=$(meta "$t" WORKTREE); mw=$(meta "$m" WORKTREE)
  printf 'a\n' > "$lw/shared.txt" && git -C "$lw" add shared.txt && git -C "$lw" -c user.name=t -c user.email=t@t commit -q -m "the coordinator's"
  printf 'b\n' > "$mw/shared.txt" && git -C "$mw" add shared.txt && git -C "$mw" -c user.name=t -c user.email=t@t commit -q -m "member one"
  printf 'c\nd\n' > "$mw/other.txt" && git -C "$mw" add other.txt && git -C "$mw" -c user.name=t -c user.email=t@t commit -q -m "member two"
  set_meta "$t" PR_NUMBER 7
}

@test "agent round-check shows a member's work not in the round's PR, the files two branches changed, the workers" {
  round_with_lost_work
  run agent round-check "$team"
  [ "$status" -eq 0 ]
  [[ $output == *"member"*"2 commits"* ]]
  [[ $output == *"shared.txt"* ]]
  [[ $output == *"workers: 0"* ]]
  [[ $output == *"board:"* ]]
}

@test "round_check_line sums the card up in one line" {
  round_with_lost_work
  out=$(source "$REPO_ROOT/bin/agent"; round_check_line "$team")
  [[ $out == *"not in the PR: member 2 commits"* ]]
  [[ $out == *"agent round-check $team"* ]]
}

@test "the coordinator's notification of a round carries the round's card in one line" {
  round_with_lost_work
  set_meta "$t" WORK_ROUND 1
  set_meta "$t" WORK_ROUNDS 2
  (source "$REPO_ROOT/bin/agent"; notify "$t" "done" "" "the round is done")
  grep -q "round-check: " "$AGENTS_HOME/notifications/$t-r$(meta "$t" RUN)-summary.md"
}
