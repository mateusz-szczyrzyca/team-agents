#!/usr/bin/env bats
# A review whose model is overloaded at the provider ("Selected model is at capacity") paused until the overload
# passed, holding its pair back, while other investigators were free. Past AGENT_REVIEW_OVERLOAD_WAIT_MIN it now goes
# to another investigator at once, as on an exhausted quota — never to one of its pair's two, which has reviewed the
# PR already or is reviewing it.

load helpers

@test "review_hands_over: an exhausted quota, and an overload past AGENT_REVIEW_OVERLOAD_WAIT_MIN, hand a review over; a short overload or a rate limit do not" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  export AGENT_REVIEW_OVERLOAD_WAIT_MIN=5
  review_hands_over quota 60
  review_hands_over overloaded 600
  run review_hands_over overloaded 240
  [ "$status" -ne 0 ]
  run review_hands_over rate_limit 900
  [ "$status" -ne 0 ]
}

@test "a review done again goes to an investigator outside its pair, though the pair's other one is free" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  dir=$BATS_TEST_TMPDIR/pair
  mkdir -p "$dir"
  echo inv-a > "$dir/A.agent"   # overloaded: its review is done again
  echo inv-b > "$dir/B.agent"   # has reviewed the PR already: free
  automation_allowed() { return 0; }
  free_investigator_list() { printf '%s\n' inv-b inv-c; }
  start_review_slot() { echo "$4" > "$BATS_TEST_TMPDIR/picked"; }
  act() { :; }
  auto_stage() { :; }
  mkdir -p "$TASKS/P"
  printf 'ID=P\nSTATUS=running\n' > "$TASKS/P/meta.env"
  retry_review_slot P "$dir" A
  [ "$(cat "$BATS_TEST_TMPDIR/picked")" = inv-c ]
}
