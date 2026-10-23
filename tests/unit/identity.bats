#!/usr/bin/env bats
# "done" is a status name passed as an argument (wait_status "$t" done), not the end of a loop.
# shellcheck disable=SC1010
# The agents' commits are never the human's: not their e-mail (GitHub would attribute the commits to the human's
# account) and not their signature (README "Commit identity"). The human's git configuration lives in the test's
# temporary HOME.

load helpers

# last_commit format — the last commit of the solo task's branch, in a git log format
last_commit() {
  local t branch
  t=$(task_of solo)
  branch=$(sed -n 's/^BRANCH=//p' "$AGENTS_HOME/tasks/$t/meta.env")
  git -C "$HOME/repo" log -1 --format="$1" "$branch"
}

@test "an empty GIT_EMAIL is the agent's own .invalid address, never the human's e-mail from git config" {
  git config --global user.email "me@example.com"
  scenario '{"solo": {"new": "commit"}}'
  agent run solo "add a file"
  wait_status "$(task_of solo)" done
  [ "$(last_commit '%ae|%ce')" = "solo@team-agents.invalid|solo@team-agents.invalid" ]
}

@test "%a in GIT_EMAIL is the agent's name" {
  echo 'GIT_EMAIL="agents+%a@example.com"' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "commit"}}'
  agent run solo "add a file"
  wait_status "$(task_of solo)" done
  [ "$(last_commit '%ae')" = "agents+solo@example.com" ]
}

@test "without GIT_SIGNING_KEY the agents do not sign, even when the human signs every commit" {
  ssh-keygen -q -t ed25519 -N "" -f "$HOME/human-key" >/dev/null
  git config --global gpg.format ssh
  git config --global user.signingkey "$HOME/human-key"
  git config --global commit.gpgsign true
  scenario '{"solo": {"new": "commit"}}'
  agent run solo "add a file"
  wait_status "$(task_of solo)" done
  run git -C "$HOME/repo" cat-file commit "$(last_commit '%H')"
  [ "$status" -eq 0 ]
  [[ $output != *"gpgsig"* ]]
}
