#!/usr/bin/env bats
# An investigator's review on GitHub says who wrote it at its very top, not only in its last line: the agents' gh
# puts the byline first when the review body does not begin with it (AGENT_REVIEW_BYLINE, an investigation's own).

load helpers

BYLINE='Review by the team-agents investigator `inv-astra` (codex, model gpt-6-astra), request #66.'

setup_gh() {
  mkdir -p "$BATS_TEST_TMPDIR/ghw"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"   # agent run as gh, as the runner links it
  GH=$BATS_TEST_TMPDIR/ghw/gh
  export AGENT_REAL_GH=$REPO_ROOT/tests/unit/stubs/gh
  export AGENT_GITHUB_TRUSTED_RESOLVED="mateusz-szczyrzyca ms-ai-agents"
  mkdir -p "$AGENTS_HOME/logs"
}

# review_json body — a review.json with that body
review_json() { jq -n --arg b "$1" '{commit_id: "x", event: "COMMENT", body: $b, comments: []}' > "$BATS_TEST_TMPDIR/review.json"; }

@test "an investigator's review gets its byline at the top; the one at the end stays; none is doubled" {
  setup_gh
  export AGENT_REVIEW_BYLINE=$BYLINE
  review_json $'Overall: needs changes.\n\n— '"$BYLINE"
  "$GH" api repos/o/r/pulls/7/reviews --method POST --input "$BATS_TEST_TMPDIR/review.json" > /dev/null
  [ "$(jq -r .body "$HOME/gh-input.json" | head -n 1)" = "$BYLINE" ]
  [ "$(jq -r .body "$HOME/gh-input.json" | grep -c 'Review by the team-agents investigator')" -eq 2 ]
  review_json "$BYLINE"$'\n\nOverall: fine.'   # already at the top
  "$GH" api repos/o/r/pulls/7/reviews --method POST --input "$BATS_TEST_TMPDIR/review.json" > /dev/null
  [ "$(jq -r .body "$HOME/gh-input.json" | grep -c 'Review by the team-agents investigator')" -eq 1 ]
}

@test "a review posted by anyone but an investigator is left as it is" {
  setup_gh
  unset AGENT_REVIEW_BYLINE
  review_json "Overall: fine."
  "$GH" api repos/o/r/pulls/7/reviews --method POST --input "$BATS_TEST_TMPDIR/review.json" > /dev/null
  [ "$(jq -r .body "$HOME/gh-input.json")" = "Overall: fine." ]
}

@test "review_byline: the investigator, its CLI and model, the request" {
  mkdir -p "$AGENTS_HOME/tasks/X"
  printf 'ID=X\nAGENT=inv-astra\nTYPE=codex\nMODEL=gpt-6-astra\nKIND=investigation\nINV_ID=66\n' > "$AGENTS_HOME/tasks/X/meta.env"
  [ "$(source "$REPO_ROOT/bin/agent"; review_byline X)" = "$BYLINE" ]
}
