#!/usr/bin/env bats
# A round handed from one coordinator to another keeps one PR: the round's PR is carried on a same-round resume, and
# a new coordinator's PR closes the earlier one as superseded. Where the automation hands a decision to the human,
# it says so on the PR too (GitHub notifies the human there). AUTO_MERGE_HOLD_TEST_CHANGES=no lets --autopilot
# merge a round that changes existing tests.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
leads() { local d; for d in "$AGENTS_HOME"/tasks/*-lead/; do d=${d%/}; echo "${d##*/}"; done; }

@test "a same-round resume carries the round's PR to the new team" {
  echo '[{"number": 24, "url": "https://github.com/o/r/pull/24", "state": "OPEN", "isDraft": false}]' > "$HOME/gh-prs.json"
  scenario '{"lead": {"new": "team-lead-done", "resume": "team-lead-wait"}, "member": {"new": "team-sleep", "resume": "team-sleep"}, "inv": {"new": "review-ready"}}'
  agent run --team lead,member --verify "team task"
  for _ in $(seq 1 100); do [ -n "$(leads)" ] && break; sleep 0.1; done
  first=$(leads | head -n 1)
  for _ in $(seq 1 600); do [ "$(meta "$first" PR_NUMBER)" = 24 ] && break; sleep 0.2; done
  [ "$(meta "$first" PR_NUMBER)" = 24 ]
  team=$(meta "$first" TEAM)
  agent run --resume "$team" --same-round "go on"
  second=$(leads | sed -n 2p)
  [ -n "$second" ]
  [ "$(meta "$second" ROUND_PR)" = 24 ]
  agent team "$(meta "$second" TEAM)" --close > /dev/null
}

@test "a new coordinator's PR closes the round's earlier PR as superseded, and links back to it" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  ( source "$REPO_ROOT/bin/agent"; meta_set "$t" ROUND_PR 24; round_pr_supersede "$t" 29 "https://github.com/o/r/pull/29" )
  grep -q "pr close 24 --comment Superseded by #29" "$HOME/gh.log"
  grep -q "pr comment 29 --body Continues #24" "$HOME/gh.log"
  [ "$(meta "$t" ROUND_PR)" = 29 ]
  ( source "$REPO_ROOT/bin/agent"; round_pr_supersede "$t" 29 "https://github.com/o/r/pull/29" )   # the same PR: nothing to close
  [ "$(grep -c 'pr close' "$HOME/gh.log")" -eq 1 ]
}

@test "--autopilot hands a decision to the human on the PR too: a comment says what waits for them" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  printf 'INVESTIGATORS_TYPE[inv2]=claude\nAGENT_REVIEW_GATE=true\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-round:1/2"}, "member": {"new": "team-sleep", "resume": "team-sleep"}, "inv": {"new": "review-ready"}, "inv2": {"new": "review-ready"}}'
  out=$(agent run --team lead,member --autopilot "a task in two rounds")
  team=$(sed -n 's/^Started team \([^:]*\):.*/\1/p' <<< "$out")
  agent autopilot "$team" off
  wait_grep "autopilot is off" "$HOME/notifications.log" 150
  wait_grep "waits for you: the autopilot is off" "$HOME/gh.log" 20   # wait_grep looks for the text as it is
  grep "waits for you: the autopilot is off" "$HOME/gh.log" | grep -q "pr comment https://github.com/o/r/pull/7 --body"
}

@test "AUTO_MERGE_HOLD_TEST_CHANGES=no: changed existing tests no longer keep --autopilot from merging" {
  source "$REPO_ROOT/bin/agent"
  r=$BATS_TEST_TMPDIR/r
  git init -q -b main "$r"
  echo x > "$r/x.go"
  git -C "$r" add . && git -C "$r" -c user.name=t -c user.email=t@t commit -q -m x
  out=$(AUTO_MERGE_HOLD_TEST_CHANGES=yes merge_blockers "$r" main main "code=1 added=1 changed=1")
  [[ $out == *"changes existing tests"* ]]
  out=$(AUTO_MERGE_HOLD_TEST_CHANGES=no merge_blockers "$r" main main "code=1 added=1 changed=1")
  [[ $out != *"changes existing tests"* ]]
}
