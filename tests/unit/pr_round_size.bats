#!/usr/bin/env bats
# Teams kept line counts in their PR descriptions by their own rule (reports included, where the round limit leaves
# them out) and sent workers to recount them (request H1091: "handwritten diff totals INCLUDING reports"). The
# precheck now writes the round's size into the PR description itself, counted as the round limit counts it, in a
# block of its own that it rewrites at each check; the prompt tells the team not to count lines for the PR.

load helpers

@test "the size block is added to a description without one, replaced in one with it, and the rest kept" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  block=$(printf '%s\n%s\n%s' "$PR_SIZE_START" "Round size: 10 lines" "$PR_SIZE_END")
  body=$(pr_body_with_round_size "## Summary
Text." "$block")
  [[ $body == "## Summary"$'\n'"Text."*"Round size: 10 lines"* ]]
  newer=$(printf '%s\n%s\n%s' "$PR_SIZE_START" "Round size: 25 lines" "$PR_SIZE_END")
  body2=$(pr_body_with_round_size "$body
## After
More." "$newer")
  [[ $body2 == *"Round size: 25 lines"* ]]
  [[ $body2 != *"Round size: 10 lines"* ]]
  [[ $body2 == "## Summary"$'\n'"Text."* && $body2 == *"## After"$'\n'"More." ]]
  [ "$(grep -c "$PR_SIZE_START" <<< "$body2")" -eq 1 ]
  [ "$(pr_body_with_round_size "$body2" "$newer")" = "$body2" ]   # the same size again: nothing to change
}

@test "--autopilot: the precheck writes the round's size into its PR description, the description's text kept" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  printf '## Summary\nThe round.\n' > "$HOME/gh-pr-body.md"
  echo "AGENT_REVIEW_GATE='true'" >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-done", "resume": "team-lead-done"}, "member": {"new": "team-sleep", "resume": "done"}, "inv": {"new": "review-ready"}}'
  agent run --team lead,member --autopilot "team task"
  wait_grep "gh pr edit https://github.com/o/r/pull/7" "$HOME/gh.log" 150
  wait_grep "Round size" "$HOME/gh.log" 30
  grep -q "The round." "$HOME/gh.log"
  agent stop --running > /dev/null
}

@test "the rounds prompt tells the team the script writes the round's size into the PR: no counting for it" {
  grep -q "writes the round's size into the PR description" "$REPO_ROOT/prompts/tdd-rounds.md"
  grep -q "nor ask a worker" "$REPO_ROOT/prompts/tdd-rounds.md"
}
