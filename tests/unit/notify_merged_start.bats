#!/usr/bin/env bats
# The autopilot's merge was notified as "🔀 merged" in the file of the round's own notification, overwriting it, and
# a new task, team or round started with no notification. A merge is "🟣 PR merged", with who gave it "ready to
# merge" — the two reviewers, their CLI and model —, in a file of its own; a start is "🚀", once for a team.

load helpers

@test "a new solo task is announced as started" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task that starts"
  t=$(task_of solo)
  wait_status "$t" 'done'
  grep -q "🚀 started" "$HOME/notifications.log"
  grep -q "work starts" "$HOME/notifications.log"
}

@test "a new team is announced as started once, not once a member" {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "a team that starts" > /dev/null
  wait_status "$(task_of lead)" running
  wait_grep "🚀 started" "$HOME/notifications.log"
  [ "$(grep -c '🚀 started' "$HOME/notifications.log")" -eq 1 ]
  agent stop --running > /dev/null
}

@test "a merge is '🟣 PR merged', with its two reviewers and their models, in a file of its own" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # read by bin/agent
  INVESTIGATORS_TYPE["inv2"]=codex
  # shellcheck disable=SC2034 # read by bin/agent (its arrays, sourced here, are this function's)
  INVESTIGATORS_MODEL["inv"]=opus
  # shellcheck disable=SC2034
  INVESTIGATORS_MODEL["inv2"]=gpt-6-astra
  dir=$BATS_TEST_TMPDIR/pair
  mkdir -p "$dir"
  echo inv > "$dir/A.agent"
  echo inv2 > "$dir/B.agent"
  [ "$(pair_reviewers "$dir")" = "inv (claude · opus), inv2 (codex · gpt-6-astra)" ]
  mkdir -p "$TASKS/P"
  printf 'ID=P\nREPO_NAME=repo\nAGENT=lead\nTYPE=claude\nRUN=1\nKIND=worker\n' > "$TASKS/P/meta.env"
  notify P "done" "" "the round's own notification"
  notify P merged "" "merged PR #7 — ready to merge: $(pair_reviewers "$dir")"
  grep -q "the round's own notification" "$NOTIFY_DIR/P-r1-summary.md"   # not overwritten
  grep -q "🟣 PR merged" "$NOTIFY_DIR/P-r1-merged-summary.md"
  grep -q "inv2 (codex · gpt-6-astra)" "$NOTIFY_DIR/P-r1-merged-summary.md"
}
