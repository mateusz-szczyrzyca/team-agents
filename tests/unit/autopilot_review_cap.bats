#!/usr/bin/env bats
# --autopilot stops after three reviews that still find problems (work → review → fix → review → fix → review): with
# VERIFY_MAX_FIX_PASSES (2) fix passes spent the team is not woken again; the human is mentioned on the PR and notified.

load helpers

# stage task — a task's AUTO_STAGE, as the script reads it (meta.env keeps it shell-quoted)
stage() { ( eval "$(grep '^AUTO_STAGE=' "$AGENTS_HOME/tasks/$1/meta.env")"; printf '%s' "$AUTO_STAGE" ); }
# leads — the lead's tasks, oldest first
leads() { local d; for d in "$AGENTS_HOME"/tasks/*-lead/; do d=${d%/}; echo "${d##*/}"; done; }
# reviews — the review investigations' tasks, one per line
reviews() { grep -l '^REVIEW=1$' "$AGENTS_HOME"/tasks/*/meta.env 2> /dev/null || (($? == 1)); }   # none yet is an answer

@test "--autopilot: after three pairs of reviews with findings the autopilot stops and waits for the human" {
  printf 'AGENT_HUMAN_GITHUB=boss\nINVESTIGATORS_TYPE[inv2]=claude\nAGENT_REVIEW_GATE=true\n' >> "$AGENTS_HOME/agents.conf"
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  scenario '{"lead": {"new": "team-lead-done", "resume": "team-lead-done"}, "member": {"new": "team-sleep", "resume": "done"}, "inv": {"new": "review-fixes"}, "inv2": {"new": "review-fixes"}}'
  agent run --team lead,member --autopilot "team task"
  wait_grep "after 2 fix passes" "$HOME/notifications.log" 300
  grep -q "@boss" "$HOME/gh.log"
  [ "$(reviews | wc -l)" -eq 6 ]   # three passes, two reviews each
  [ "$(leads | wc -l)" -eq 3 ]
  [ "$(stage "$(leads | tail -1)")" = "waits for you: the reviews still find problems after 2 fix passes" ]
  sleep 3
  [ "$(reviews | wc -l)" -eq 6 ]   # nothing more by itself
  [ "$(leads | wc -l)" -eq 3 ]
}
