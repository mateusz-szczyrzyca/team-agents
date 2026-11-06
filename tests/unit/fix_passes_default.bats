#!/usr/bin/env bats
# --autopilot's cycle by default: a review, a fix pass, five times; the sixth review that still finds problems calls
# the human (VERIFY_MAX_FIX_PASSES=5; it was 2).

load helpers

@test "by default --autopilot has five fix passes: the sixth failed review calls the human" {
  unset VERIFY_MAX_FIX_PASSES
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  [ "$VERIFY_MAX_FIX_PASSES" = 5 ]
}
