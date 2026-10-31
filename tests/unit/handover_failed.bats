#!/usr/bin/env bats
# A resume of a team does not depend on whether the lead's own checkout is pushed: remote members coming back continue
# their round, and only a new team needs every member to start from one commit on origin. A team that cannot take
# its work back (a bounce or a fix pass whose resume is refused) is never silent: the human is told why.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# ahead_repo — a clone whose main has a commit origin does not have; prints its path
ahead_repo() {
  local o=$BATS_TEST_TMPDIR/origin.git r=$BATS_TEST_TMPDIR/clone
  git init -q --bare -b main "$o"
  git clone -q "$o" "$r" 2> /dev/null
  git -C "$r" -c user.name=t -c user.email=t@t commit -q --allow-empty -m first
  git -C "$r" push -q origin main
  git -C "$r" -c user.name=t -c user.email=t@t commit -q --allow-empty -m "applied, not pushed yet"
  echo "$r"
}

@test "remote_start_check: the lead's commit must be on origin for a new remote member" {
  source "$REPO_ROOT/bin/agent"
  r=$(ahead_repo)
  run ! remote_start_check "$r" main
  git -C "$r" push -q origin main
  remote_start_check "$r" main
}

@test "a remote member coming back on a resume does not need the lead's checkout pushed; a new one does" {
  r=$(ahead_repo)
  # shellcheck disable=SC2034  # LEAD_LISTEN is read by remote_member_task, sourced here
  out_new=$(source "$REPO_ROOT/bin/agent"; LEAD_LISTEN=127.0.0.1:1
    bus() { echo '[{"Node": "node2", "Offline": false, "FreeAgents": [{"Name": "g", "Tier": "normal"}]}]'; }
    remote_member_task "g@node2" "a task" "T-team" main "$r" 20261030-000000 2>&1) || true   # it dies: its output is the answer
  [[ $out_new == *"push main first"* ]]
  # shellcheck disable=SC2034  # as above
  out_resume=$(source "$REPO_ROOT/bin/agent"; LEAD_LISTEN=127.0.0.1:1
    bus() { echo '[{"Node": "node2", "Offline": false, "FreeAgents": [{"Name": "g", "Tier": "normal"}]}]'; }
    remote_member_task "g@node2" "a task" "T-team" main "$r" 20261030-000000 resume 2>&1) || true   # later steps need a lead: only this check is looked at
  [[ $out_resume != *"push main first"* ]]
}

@test "a team that cannot take its work back tells the human why: the stage, the PR, the notification" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  ( source "$REPO_ROOT/bin/agent"
    meta_set "$t" PR_URL "https://github.com/o/r/pull/30"
    handover_failed "$t" "the round" $'Resuming…\nagent: remote member g@node2 starts from origin: push main first (git push origin main)' )
  # shellcheck source=/dev/null
  stage=$(source "$AGENTS_HOME/tasks/$t/meta.env" && printf '%s' "$AUTO_STAGE")   # read as the script reads it
  [[ $stage == "waits for you: the round could not go back to the team — remote member g@node2 starts from origin"* ]]
  wait_grep "could not go back to the team: remote member g@node2 starts from origin" "$HOME/notifications.log"
  grep "pr comment https://github.com/o/r/pull/30" "$HOME/gh.log" | grep -q "could not go back to the team"
}
