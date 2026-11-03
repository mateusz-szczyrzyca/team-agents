#!/usr/bin/env bats
# The agents' names on GitHub — in a PR's attribution ("coordinator: …", "member: …") and an investigator's review
# byline — are bold, each in its own colour, the same in every round: the agents' gh colours them on the way out
# (AGENT_GITHUB_NAME_COLORS, set by the runner from the configuration's order of the agents).

load helpers

# setup_gh — agent run as gh in front of a recording real gh: its arguments one a line in $HOME/real-args, the file
# given with --body-file or --input copied to $HOME/real-body
setup_gh() {
  mkdir -p "$BATS_TEST_TMPDIR/ghw" "$AGENTS_HOME/logs"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"
  GH=$BATS_TEST_TMPDIR/ghw/gh
  cat > "$BATS_TEST_TMPDIR/real-gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$HOME/real-args"
prev=""
for a in "$@"; do
  if [[ $prev == --body-file || $prev == -F || $prev == --input ]]; then cp "$a" "$HOME/real-body"; fi
  if [[ $prev == --body || $prev == -b ]]; then printf '%s' "$a" > "$HOME/real-body"; fi
  prev=$a
done
EOF
  chmod +x "$BATS_TEST_TMPDIR/real-gh"
  export AGENT_REAL_GH=$BATS_TEST_TMPDIR/real-gh
  export AGENT_GITHUB_TRUSTED_RESOLVED="mateusz-szczyrzyca ms-ai-agents"
  export AGENT_GITHUB_COLORS="#0969DA #BC4C00 #8250DF #1B7C83"
  export AGENT_GITHUB_NAME_COLORS="claude-haiku=#0969DA codex-1=#BC4C00 gemini-1=#8250DF inv-astra=#0969DA inv-opus=#BC4C00"
}

BLOCK='Opened by team-agents:
- team: `T1`
- coordinator: `gemini-1` (gemini, model `gemini-3.8-flash-medium`)
- member: `claude-haiku` (claude, model `haiku`)
- member: `codex-1` (codex, model `gpt-6.1-sol`)'

@test "gh pr create --body: the coordinator and the members are bold, each in its own colour" {
  setup_gh
  "$GH" pr create --title t --body "Fixes the cache."$'\n\n'"$BLOCK"
  body=$(cat "$HOME/real-body")
  [[ $body == *'- coordinator: $\color{#8250DF}{\textbf{gemini-1}}$ (gemini, model `gemini-3.8-flash-medium`)'* ]]
  [[ $body == *'- member: $\color{#0969DA}{\textbf{claude-haiku}}$ (claude, model `haiku`)'* ]]
  [[ $body == *'- member: $\color{#BC4C00}{\textbf{codex-1}}$ (codex, model `gpt-6.1-sol`)'* ]]
  [[ $body == "Fixes the cache."* ]]
  [[ $body == *'- team: `T1`'* ]]
}

@test "gh pr edit --body-file: the same colours as in the PR's first round" {
  setup_gh
  printf '%s\n' "$BLOCK" > "$BATS_TEST_TMPDIR/body.md"
  "$GH" pr edit 7 --body-file "$BATS_TEST_TMPDIR/body.md"
  grep -qF -- '- coordinator: $\color{#8250DF}{\textbf{gemini-1}}$ (gemini' "$HOME/real-body"
  grep -qF -- '- member: $\color{#0969DA}{\textbf{claude-haiku}}$ (claude' "$HOME/real-body"
  grep -qx -- '- coordinator: `gemini-1` (gemini, model `gemini-3.8-flash-medium`)' "$BATS_TEST_TMPDIR/body.md"   # the agent's file stays
}

@test "an investigator's review: its byline at the top and at the end, bold in the investigator's colour" {
  setup_gh
  export AGENT_REVIEW_BYLINE='Review by the team-agents investigator `inv-astra` (codex, model gpt-6-astra), request #66.'
  jq -n --arg b $'Overall: fine.\n\n— '"$AGENT_REVIEW_BYLINE" '{commit_id: "x", event: "COMMENT", body: $b, comments: []}' \
    > "$BATS_TEST_TMPDIR/review.json"
  "$GH" api repos/o/r/pulls/7/reviews --method POST --input "$BATS_TEST_TMPDIR/review.json"
  local want='Review by the team-agents investigator $\color{#0969DA}{\textbf{inv-astra}}$ (codex, model gpt-6-astra), request #66.'
  [ "$(jq -r .body "$HOME/real-body" | head -n 1)" = "$want" ]
  [ "$(jq -r .body "$HOME/real-body" | tail -n 1)" = "— $want" ]
  [ "$(jq -r .event "$HOME/real-body")" = COMMENT ]
}

@test "two names of one text with the same colour: the second gets the next colour nobody in it has" {
  setup_gh
  export AGENT_GITHUB_NAME_COLORS="gemini-1=#0969DA claude-haiku=#0969DA"
  "$GH" pr create --title t --body "$BLOCK"
  grep -qF -- '$\color{#0969DA}{\textbf{gemini-1}}$' "$HOME/real-body"
  grep -qF -- '$\color{#BC4C00}{\textbf{claude-haiku}}$' "$HOME/real-body"
}

@test "a name the map does not know gets a colour of the palette from its name, the same every time" {
  setup_gh
  "$GH" pr create --title t --body '- member: `remote-7@node2` (claude, model `sonnet`)'
  first=$(cat "$HOME/real-body")
  [[ $first =~ ^'- member: $\color{#'[0-9A-F]{6}'}{\textbf{remote-7@node2}}$ (claude' ]]
  "$GH" pr create --title t --body '- member: `remote-7@node2` (claude, model `sonnet`)'
  [ "$(cat "$HOME/real-body")" = "$first" ]
}

@test "with no colours known the text goes out as it is" {
  setup_gh
  unset AGENT_GITHUB_NAME_COLORS AGENT_GITHUB_COLORS
  "$GH" pr create --title t --body "$BLOCK"
  [ "$(cat "$HOME/real-body")" = "$BLOCK" ]
}

@test "agent_github_colors: the agents in the configuration's order, each its own colour; investigators apart" {
  cat >> "$AGENTS_HOME/agents.conf" <<'CONF'
NORMAL_AGENTS=(claude-haiku codex-1 gemini-1)
WORKER_AGENTS=(worker worker2)
declare -A INVESTIGATORS_TYPE=([inv-opus]=claude [inv-astra]=codex [inv-kimi]=opencode)
AGENT_GITHUB_COLORS="#000001 #000002 #000003 #000004 #000005 #000006"
CONF
  out=$(source "$REPO_ROOT/bin/agent"; agent_github_colors)
  [[ " $out " == *" claude-haiku=#000001 codex-1=#000002 gemini-1=#000003 worker=#000004 worker2=#000005 "* ]]
  [[ " $out " == *" inv-astra=#000001 inv-kimi=#000002 inv-opus=#000003 "* ]]
}

@test "the runner gives the agents' gh the colours of every agent" {
  out=$( source "$REPO_ROOT/bin/agent"; RUNNER_ID="" REPO_ROOT=$HOME/repo AGENT_GITHUB_TRUSTED=o
         agents_gh_env; printf '%s|%s' "$AGENT_GITHUB_NAME_COLORS" "$AGENT_GITHUB_COLORS" )
  [[ $out == *"lead=#"* && $out == *"inv=#"* ]]
  [[ ${out#*|} == "#"* ]]
}
