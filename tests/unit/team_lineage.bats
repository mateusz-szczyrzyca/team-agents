#!/usr/bin/env bats
# Every resume makes a new team of new tasks, whose RESUMED_FROM names the old ones. Two things a team was given stayed
# with the old team's ID. The human's allowance (agent allow): after each resume the precheck refused the same approved
# change again, and an --autopilot coordinator, sent its question back, allowed the change itself. And --verify's
# "sent back once": member commits explained after the first bounce sent the round back after every resume, each time
# for one more report-only commit and one more gate. Both now follow the team's resume line.

load helpers

# task id team [resumed_from] — a coordinator's task of a team, resumed from another task
task() {
  mkdir -p "$TASKS/$1"
  printf 'ID=%s\nAGENT=lead\nKIND=worker\nTEAM=%s\nRESUMED_FROM=%q\n' "$1" "$2" "${3:-}" > "$TASKS/$1/meta.env"
}

@test "team_lineage: a team and the teams it was resumed from, newest first; a member added on resume does not end it" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  task A1 T1
  task B1 T2 A1
  task C0 T3          # a member the resume added: no task it was resumed from
  task C1 T3 B1
  [ "$(team_lineage T3 | paste -sd ' ')" = "T3 T2 T1" ]
  [ "$(team_lineage T1)" = T1 ]
  task X1 T9 Y1       # a loop in broken metadata ends
  task Y1 T8 X1
  [ "$(team_lineage T9 | paste -sd ' ')" = "T9 T8" ]
}

@test "an allowance the human gave a team holds for the teams resumed from it, also one given after the resume" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  task A1 T1
  task B1 T2 A1
  task C1 T3 B1
  mkdir -p "$AGENTS_HOME/allowed"
  printf 'agent/f_test.go\n' > "$AGENTS_HOME/allowed/T1"
  printf 'tests/unit/a.bats\n' > "$AGENTS_HOME/allowed/T3"
  run team_allowed_paths T3
  [[ $output == *agent/f_test.go* && $output == *tests/unit/a.bats* ]]
  run team_allowed_paths T1
  [ "$output" = agent/f_test.go ]   # an older team gets nothing of the newer one's
  task D1 T4
  [ -z "$(team_allowed_paths T4)" ]
}

@test "--verify: unmerged member commits a coordinator before the resume was sent back for do not send it back again" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  task A1 T1
  task B1 T2 A1
  member="- member m's branch (agents/m/1) has unmerged changes (4 commits) relative to agents/lead/1 — merge them"
  draft="- PR #78 is still a draft — when the work is complete: gh pr ready"
  verify_gaps() { printf '%s\n%s\n' "$member" "$draft"; }
  [ "$(verify_bounce_gaps A1)" = "$(printf '%s\n%s' "$member" "$draft")" ]   # the first: both, and recorded
  [ "$(verify_bounce_gaps B1)" = "$draft" ]                                 # resumed: only what is not settled
  verify_gaps() { printf '%s\n' "${member/4 commits/5 commits}"; }          # the member added a commit since
  [ "$(verify_bounce_gaps B1)" = "${member/4 commits/5 commits}" ]
}
