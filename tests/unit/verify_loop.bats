#!/usr/bin/env bats
# "done" is a status name passed as an argument (wait_status "$t" done), not the end of a loop.
# shellcheck disable=SC1010
# --last-team, --same-round without rounds, and the --verify fix loop: a review with critical or major findings
# wakes the team with them (at most VERIFY_MAX_FIX_PASSES times); then the human is called on the PR and notified.

load helpers

# meta id key — a value from a task's meta.env
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# leads — the lead's tasks, oldest first (task IDs start with their time; a glob is sorted)
leads() {
  local d
  for d in "$AGENTS_HOME"/tasks/*-lead; do [[ -d $d ]] && echo "${d##*/}"; done
  return 0
}

# teams_of agent — the distinct teams the agent's tasks belonged to, one per line
teams_of() { grep -h '^TEAM=' "$AGENTS_HOME"/tasks/*-"$1"/meta.env 2> /dev/null | sort -u; }

@test "--last-team resumes the newest team of the repository" {
  scenario '{"lead": {"new": "done", "resume": "done"}, "member": {"new": "done", "resume": "done"}}'
  agent run --team lead,member "first team"
  wait_status "$(task_of lead)" done
  first=$(meta "$(task_of lead)" TEAM)
  sleep 1.1
  agent run --team lead,member "second team"
  t2=$(leads | tail -1)
  wait_status "$t2" done
  second=$(meta "$t2" TEAM)
  [ "$first" != "$second" ]
  cd "$HOME/repo"
  run agent run --last-team "go on" < /dev/null
  [ "$status" -eq 0 ]
  [[ $output == *"$second"* && $output != *"$first"* ]]
}

@test "--same-round on a team that did not end a round continues on the same branches" {
  scenario '{"lead": {"new": "done", "resume": "done"}, "member": {"new": "done", "resume": "done"}}'
  agent run --team lead,member "team task"
  t=$(task_of lead)
  wait_status "$t" done
  cd "$HOME/repo"
  agent run --last-team --same-round "fix the review findings" < /dev/null
  t2=$(leads | tail -1)
  [ "$t2" != "$t" ]
  [ "$(meta "$t2" BRANCH)" = "$(meta "$t" BRANCH)" ]
}

@test "a review with major findings wakes the team with them, twice at most; then the human is called on the PR" {
  printf 'AGENT_HUMAN_GITHUB=boss\n' >> "$AGENTS_HOME/agents.conf"
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  scenario '{"lead": {"new": "done", "resume": "done"}, "member": {"new": "done", "resume": "done"}, "inv": {"new": "review-majors"}}'
  agent run --team lead,member --verify "team task"
  wait_grep "pr comment" "$HOME/gh.log" 150
  grep -q "@boss" "$HOME/gh.log"
  wait_grep "still" "$HOME/notifications.log"
  [ "$(teams_of lead | wc -l)" -eq 3 ]
  passes=$(for t in $(leads); do meta "$t" FIX_PASS; done | tr '\n' ' ')
  [ "$passes" = "0 1 2 " ]
  second=$(leads | sed -n 2p)
  grep -q "Fix pass 1 of 2" "$AGENTS_HOME/tasks/$second/task.md"
  grep -q "the parser drops the last line" "$AGENTS_HOME/tasks/$second/task.md"
  sleep 3
  [ "$(teams_of lead | wc -l)" -eq 3 ]
}
