#!/usr/bin/env bats
# agent run --resume <team> --team a,b,…: the members listed go on, a new one joins on its own branch from the
# coordinator's (the same round) and its task, the members not listed are left out; an unknown name is refused.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# stopped_team — a team of lead and member, stopped while it ran; prints the lead's task
stopped_team() {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  local old
  old=$(task_of lead)
  wait_status "$old" running
  agent stop --running > /dev/null
  echo "$old"
}

@test "agent run --resume --team: the listed go on, a new member joins from the coordinator's branch, the others are left out" {
  old=$(stopped_team)
  team=$(meta "$old" TEAM)
  scenario '{"lead": {"new": "sleep:300"}, "solo": {"new": "sleep:300"}}'
  run agent run --resume "$team" --team "lead, solo" --same-round "go on"
  [ "$status" -eq 0 ]
  new=$(task_of lead)
  joiner=$(task_of solo)
  newteam=$(meta "$new" TEAM)
  [ "$newteam" != "$team" ]
  [ "$(meta "$joiner" TEAM)" = "$newteam" ]
  [[ $(meta "$joiner" BRANCH) == agents/solo/* ]]
  root=$(meta "$new" REPO_ROOT)
  [ "$(git -C "$root" rev-parse "$(meta "$joiner" BRANCH)")" = "$(git -C "$root" rev-parse "$(meta "$new" BRANCH)")" ]
  [ ! -e "$AGENTS_HOME/tasks/${newteam%-team}-member" ]   # not listed: left out
  agent stop --running > /dev/null
}

@test "agent run --resume --team refuses a name that is not a configured agent" {
  old=$(stopped_team)
  run agent run --resume "$(meta "$old" TEAM)" --team "lead,nobody" --same-round "go on"
  [ "$status" -ne 0 ]
  [[ $output == *nobody* ]]
}
