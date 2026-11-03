#!/usr/bin/env bats
# An investigator's review on GitHub: its recommendation is bold, in colour, with the notification's icon — ✅ green
# "Ready to merge", 🔧 red "Needs changes" (🧩 when incomplete) — whatever words the model wrote it in. The agents' gh
# rewrites the recommendation's line of a review as it publishes it; the rest of the text and other texts stay.

load helpers

# setup_gh — agent run as gh in front of a recording real gh: the file given with --input copied to $HOME/real-body
setup_gh() {
  mkdir -p "$BATS_TEST_TMPDIR/ghw" "$AGENTS_HOME/logs"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"
  GH=$BATS_TEST_TMPDIR/ghw/gh
  cat > "$BATS_TEST_TMPDIR/real-gh" <<'EOF'
#!/usr/bin/env bash
prev=""
for a in "$@"; do
  if [[ $prev == --input ]]; then cp "$a" "$HOME/real-body"; fi
  prev=$a
done
EOF
  chmod +x "$BATS_TEST_TMPDIR/real-gh"
  export AGENT_REAL_GH=$BATS_TEST_TMPDIR/real-gh
  export AGENT_GITHUB_TRUSTED_RESOLVED="mateusz-szczyrzyca"
  export AGENT_REVIEW_BYLINE='Review by the team-agents investigator `inv-astra` (codex, model gpt-6-astra), request #125.'
}

# publish body [path] — the body as GitHub gets it from a review posted with --input
publish() {
  jq -n --arg b "$1" '{commit_id: "x", event: "COMMENT", body: $b, comments: []}' > "$BATS_TEST_TMPDIR/review.json"
  "$GH" api "${2:-repos/o/r/pulls/50/reviews}" --method POST --input "$BATS_TEST_TMPDIR/review.json"
  jq -r .body "$HOME/real-body"
}

GREEN='✅ $\color{green}{\textbf{Ready to merge}}$'
RED='🔧 $\color{red}{\textbf{Needs changes}}$'

@test "a Recommendation heading's line: needs changes in red, bold, with 🔧" {
  setup_gh
  out=$(publish $'## Recommendation\n\nNeeds changes — fix M4 first.\n\n## Findings\n\nThe code needs changes in two places.')
  grep -qxF -- "$RED — fix M4 first." <<< "$out"
  grep -qxF 'The code needs changes in two places.' <<< "$out"
}

@test "an inline recommendation as inv-astra wrote it on PR #50: the words become the coloured ones" {
  setup_gh
  out=$(publish 'Recommendation: **needs changes**. All three previous major findings are fixed.')
  grep -qxF -- "Recommendation: $RED. All three previous major findings are fixed." <<< "$out"
}

@test "ready to merge in green, bold, with ✅; an icon already there is not doubled" {
  setup_gh
  out=$(publish $'## Recommendation\n\n✅ **Ready to merge.**')
  grep -qxF -- "$GREEN." <<< "$out"
  out=$(publish '## Recommendation: ready to merge')
  grep -qxF -- "## Recommendation: $GREEN" <<< "$out"
}

@test "incomplete: red with 🧩" {
  setup_gh
  out=$(publish $'## Recommendation\n\nNeeds changes — incomplete: the tests did not run.')
  grep -qxF -- '🧩 $\color{red}{\textbf{Needs changes}}$ — incomplete: the tests did not run.' <<< "$out"
}

@test "a recommendation already in colour goes out as it is" {
  setup_gh
  out=$(publish $'## Recommendation\n\n'"$RED"$'\n\n- fix M1')
  [ "$(grep -c 'Needs changes' <<< "$out")" -eq 1 ]
  grep -qxF -- "$RED" <<< "$out"
}

@test "a PR comment is not a review: its words stay" {
  setup_gh
  jq -n '{body: "Recommendation: needs changes"}' > "$BATS_TEST_TMPDIR/c.json"
  "$GH" api repos/o/r/issues/50/comments --method POST --input "$BATS_TEST_TMPDIR/c.json"
  [ "$(jq -r .body "$HOME/real-body")" = 'Recommendation: needs changes' ]
}
