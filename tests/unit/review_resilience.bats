#!/usr/bin/env bats
# --verify and --autopilot keep going when a part of the review fails: a team closed because all its members
# finished still goes to review; a review run that ends without a verdict does not stop its pair; a CLI that kills
# its own process group does not take its runner with it; a round's notification says what comes by itself.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

# leads — the lead's tasks, oldest first; reviews — the review tasks
leads() { local d; for d in "$AGENTS_HOME"/tasks/*-lead/; do d=${d%/}; echo "${d##*/}"; done; }
reviews() { grep -l '^REVIEW=1$' "$AGENTS_HOME"/tasks/*/meta.env 2> /dev/null || true; }   # none yet is an answer
# review_by agent — exit 0 when one of the review tasks is that investigator's
review_by() { reviews | xargs -r grep -l "^AGENT=$1\$" | grep -q .; }

@test "--verify: a team closed because all its members finished (its coordinator out) still goes to review" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN", "isDraft": false}]' > "$HOME/gh-prs.json"
  scenario '{"lead": {"new": "team-lead-crash"}, "member": {"new": "commit"}, "inv": {"new": "review-ready"}}'
  agent run --team lead,member --verify "team task"
  for _ in $(seq 1 600); do [ -n "$(reviews)" ] && break; sleep 0.2; done
  [ -n "$(reviews)" ]
}

@test "--autopilot: a review that ends without a verdict does not stop the pair: the other's needs-fixes starts the fix pass" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN", "isDraft": false}]' > "$HOME/gh-prs.json"
  printf 'INVESTIGATORS_TYPE[inv2]=claude\nAGENT_REVIEW_GATE=true\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-done", "resume": "team-sleep"}, "member": {"new": "team-sleep", "resume": "team-sleep"}, "inv": {"new": "review-fixes"}, "inv2": {"new": "cli-error"}}'
  agent run --team lead,member --autopilot "team task"
  for _ in $(seq 1 750); do [ "$(leads | wc -l)" -ge 2 ] && break; sleep 0.2; done
  [ "$(leads | wc -l)" -ge 2 ]
}

@test "--autopilot: a review that ends without a verdict while the other is ready to merge is done again by another investigator" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN", "isDraft": false}]' > "$HOME/gh-prs.json"
  printf 'INVESTIGATORS_TYPE[inv2]=claude\nINVESTIGATORS_TYPE[inv3]=claude\nAGENT_REVIEW_GATE=true\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-done"}, "member": {"new": "team-sleep"}, "inv": {"new": "review-ready"}, "inv2": {"new": "cli-error"}, "inv3": {"new": "review-ready"}}'
  agent run --team lead,member --autopilot "team task"
  for _ in $(seq 1 750); do review_by inv2 && break; sleep 0.2; done
  review_by inv2   # the pair had the failing investigator
  for _ in $(seq 1 750); do review_by inv3 && break; sleep 0.2; done
  review_by inv3   # its review is done again by the third
}

@test "a CLI that kills its own process group does not take its runner with it: the run fails with the CLI's exit code" {
  scenario '{"solo": {"new": "kill-group"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" failed
  wait_grep "\"event\":\"run_finished\"" "$AGENTS_HOME/logs/agent.jsonl"
  run ! grep -q "\"event\":\"run_interrupted\".*\"task\":\"$t\"" "$AGENTS_HOME/logs/agent.jsonl"
}

@test "--autopilot: a round's notification says what comes by itself, not to merge the PR and resume" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN", "isDraft": false}]' > "$HOME/gh-prs.json"
  printf 'AGENT_REVIEW_GATE=true\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-round:1/2"}, "member": {"new": "team-sleep"}, "inv": {"new": "team-sleep"}}'
  agent run --team lead,member --autopilot "Round 1 of 2: a part"
  wait_grep "round 1/2 done" "$HOME/notifications.log"
  wait_grep "merges it and starts round 2 by itself" "$HOME/notifications.log"
  run ! grep -q "Next: merge the PR of round" "$HOME/notifications.log"
}
