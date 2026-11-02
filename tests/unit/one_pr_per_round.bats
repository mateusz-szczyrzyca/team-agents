#!/usr/bin/env bats
# A round has one PR. Round 3 of a task was merged from one member's branch while another member's PR of the same
# round stayed open, and round 4 started all the same: the next round now starts only when no PR of the round is open;
# the checks send a round with more than one open PR back to its team; and a round resumed says which is its PR.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "the next round does not start while another PR of the round is open, though one of its PRs is merged" {
  scenario '{"lead": {"new": "round-done:1/2"}, "member": {"new": "done"}}'
  agent run --team lead,member "task"
  t=$(task_of lead); m=$(task_of member)
  wait_status "$t" 'done'
  wait_status "$m" 'done'
  printf 'WORK_ROUND=1\nWORK_ROUNDS=2\n' >> "$AGENTS_HOME/tasks/$m/meta.env"
  cd "$HOME/repo" && git -c user.name=t -c user.email=t@t commit -q --allow-empty -m "Merge round 1" && git push -q origin main
  printf '[{"number": 5, "url": "https://github.com/o/r/pull/5", "state": "MERGED", "headRefName": "%s"},
           {"number": 6, "url": "https://github.com/o/r/pull/6", "state": "OPEN", "headRefName": "%s"}]\n' \
    "$(meta "$t" BRANCH)" "$(meta "$m" BRANCH)" > "$HOME/gh-prs.json"
  run agent run --resume "$(meta "$t" TEAM)" < /dev/null
  [ "$status" -ne 0 ]
  [[ $output == *"pull/6"* && $output == *"--same-round"* ]]
}

@test "round_open_prs lists the open PRs of the team's branches, and only those" {
  for x in "A1 T-team agents/a/1" "A2 T-team agents/b/1" "A3 U-team agents/c/1"; do
    read -r id team branch <<< "$x"
    mkdir -p "$AGENTS_HOME/tasks/$id"
    printf 'ID=%s\nTEAM=%s\nBRANCH=%s\nREPO_ROOT=%s\n' "$id" "$team" "$branch" "$HOME/repo" > "$AGENTS_HOME/tasks/$id/meta.env"
  done
  printf '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN", "headRefName": "agents/a/1"},
           {"number": 8, "url": "https://github.com/o/r/pull/8", "state": "OPEN", "headRefName": "agents/b/1"},
           {"number": 9, "url": "https://github.com/o/r/pull/9", "state": "MERGED", "headRefName": "agents/a/1"},
           {"number": 10, "url": "https://github.com/o/r/pull/10", "state": "OPEN", "headRefName": "agents/c/1"}]\n' > "$HOME/gh-prs.json"
  out=$(source "$REPO_ROOT/bin/agent"; round_open_prs A1)
  [[ $out == *"#7"* && $out == *"#8"* ]]
  [[ $out != *"#9"* && $out != *"#10"* ]]
}

@test "a round resumed says which is its PR, and that every commit of the round goes there" {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead)
  wait_status "$t" running
  agent stop --running > /dev/null
  printf 'PR_NUMBER=7\nPR_URL=https://github.com/o/r/pull/7\n' >> "$AGENTS_HOME/tasks/$t/meta.env"
  agent run --resume "$(meta "$t" TEAM)" --same-round "go on" > /dev/null
  t2=$(task_of lead)
  grep -q "This round's PR is #7" "$AGENTS_HOME/tasks/$t2/task.md"
  grep -q "$(meta "$t" BRANCH)" "$AGENTS_HOME/tasks/$t2/task.md"
  agent stop --running > /dev/null
}
