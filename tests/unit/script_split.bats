#!/usr/bin/env bats
# A coordinator worked alone through a fix pass, 200 steps, its board empty, while its members slept 30 minutes and no
# worker had anything: the split was left to a model. A fix pass's items are now on the new team's board before any of
# its runs starts — each item the reviews list under Recommendation, once, for a member in turn, never the coordinator
# — and a [minor] one is an easy item that goes to a worker at once, as its member's request.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

REVIEWS='## Recommendation
🔧 Needs changes
- [major] bin/agent: resume_run drops the PR note for a joiner
- [critical] agentbus/collab.go: boardAddFor wakes nobody
- [minor] README: the help line of round-check is not aligned
The overall assessment: close.

## Recommendation
🔧 Needs changes
1. [major] bin/agent: resume_run drops the PR note for a joiner
2. a finding with no severity'

@test "fix_items: the items a review lists under Recommendation, each once; [minor] ones easy, the others normal" {
  out=$(source "$REPO_ROOT/bin/agent"; fix_items "$REVIEWS")
  [ "$(wc -l <<< "$out")" -eq 4 ]
  [ "$(grep -c 'resume_run drops the PR note' <<< "$out")" -eq 1 ]
  grep -q $'^easy\t\\[minor\\] README' <<< "$out"
  grep -q $'^normal\t\\[critical\\] agentbus' <<< "$out"
  grep -q $'^normal\ta finding with no severity' <<< "$out"
  ! grep -q 'overall assessment' <<< "$out" || false
}

@test "a fix pass puts its items on the new team's board for the members in turn, not the coordinator; a minor one also goes to a worker" {
  scenario '{"lead": {"new": "done"}, "member": {"new": "done"}, "solo": {"new": "done"}}'
  agent run --team lead,member,solo "team task" > /dev/null
  t=$(task_of lead)
  wait_status "$t" 'done'
  wait_status "$(task_of member)" 'done'
  wait_status "$(task_of solo)" 'done'
  team=$(meta "$t" TEAM)
  scenario '{"lead": {"resume": "sleep:300"}, "member": {"resume": "sleep:300"}, "solo": {"resume": "sleep:300"}, "worker": {"new": "sleep:300"}, "worker2": {"new": "sleep:300"}}'
  (source "$REPO_ROOT/bin/agent"; handback "$t" "the fix pass" "AGENT_FIX_PASS=1" run --resume "$team" --same-round "The review:
$REVIEWS")
  new=$(meta "$(task_of lead)" TEAM)
  [ "$new" != "$team" ]
  board=$("$BUILD_DIR/agentbus" board-dump --dir "$AGENTS_HOME/bus" --team "$new")
  [ "$(jq '[.board | (if type == "object" then .[] else .[] end)] | length' <<< "$board")" -eq 4 ]
  [ "$(jq '[.board | (if type == "object" then .[] else .[] end) | select(.for == "lead")] | length' <<< "$board")" -eq 0 ]
  [ "$(jq '[.board | (if type == "object" then .[] else .[] end) | select(.for == "member")] | length' <<< "$board")" -eq 2 ]
  [ "$(jq '[.board | (if type == "object" then .[] else .[] end) | select(.for == "solo")] | length' <<< "$board")" -eq 2 ]
  [ "$(jq -r '.board | (if type == "object" then .[] else .[] end) | select(.title | startswith("[minor]")) | .level' <<< "$board")" = easy ]
  "$BUILD_DIR/agentbus" helper-list --dir "$AGENTS_HOME/bus" | grep -q "README: the help line"
  agent stop --running > /dev/null
}

@test "the review prompt asks for each item to fix with its severity; the fix pass tells the coordinator the items are on the board" {
  grep -q 'starting with its severity: `\[critical\]`, `\[major\]` or `\[minor\]`' "$REPO_ROOT/prompts/review.md"
  grep -q "already on the board" "$REPO_ROOT/bin/agent"
}
