#!/usr/bin/env bats
# Texts on GitHub (PR descriptions, comments, reviews): code in blocks that name their language, commits and other
# PRs as links, RED and GREEN in bold red and green. A PR description that does not follow them goes back to the
# team once before its --autopilot review.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

# leads — the lead's tasks, oldest first
leads() { local d; for d in "$AGENTS_HOME"/tasks/*-lead/; do d=${d%/}; echo "${d##*/}"; done; }

@test "pr_format_issues: commits and PRs in code format without a link, a code block without its language, plain RED and GREEN" {
  source "$REPO_ROOT/bin/agent"
  good=$'Fixed in [`53b8659`](https://github.com/o/r/commit/53b8659f00d) after [#21](https://github.com/o/r/pull/21), `1727` lines.\n\n```go\nfunc f() {}\n```\n\n$\\color{red}{\\textbf{RED}}$, then $\\color{green}{\\textbf{GREEN}}$.'
  [ -z "$(pr_format_issues "$good")" ]
  out=$(pr_format_issues $'The commits `53b8659` and `b2bc702d5892b546ba0d013023543824b9898a62` fix it; see `#21`.')
  [[ $out == *53b8659* && $out == *b2bc702d5892b546* && $out == *"#21"* ]]
  [[ $(pr_format_issues $'Run:\n```\nmake gate\n```') == *language* ]]
  out=$(pr_format_issues $'RED: the test fails.\n**GREEN**: it passes.')
  [[ $out == *RED* && $out == *GREEN* ]]
  [ -z "$(pr_format_issues $'```bash\necho `53b8659` RED:\n```')" ]   # inside a code block nothing is checked
}

@test "--autopilot: a PR description with an unlinked commit goes back to the team once, with what to fix; then the review starts" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  printf 'The fix is `53b8659`.\n' > "$HOME/gh-pr-body.md"
  printf 'INVESTIGATORS_TYPE[inv2]=claude\nAGENT_REVIEW_GATE=true\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-done", "resume": "team-lead-done"}, "member": {"new": "team-sleep", "resume": "team-sleep"}, "inv": {"new": "review-ready"}, "inv2": {"new": "review-ready"}}'
  agent run --team lead,member --autopilot "team task"
  for _ in $(seq 1 600); do [ "$(leads | wc -l)" -ge 2 ] && break; sleep 0.2; done
  second=$(leads | sed -n 2p)
  [ -n "$second" ]
  for _ in $(seq 1 300); do grep -q '^STATUS=' "$AGENTS_HOME/tasks/$second/meta.env" 2> /dev/null && break; sleep 0.2; done   # not there yet: polled again
  grep -q '53b8659' "$AGENTS_HOME/tasks/$second/task.md"
  grep -q 'commit/' "$AGENTS_HOME/tasks/$second/task.md"   # how to link it
  # once: the next end goes to the review although the description has not changed
  for _ in $(seq 1 750); do grep -lq '^REVIEW=1$' "$AGENTS_HOME"/tasks/*/meta.env 2> /dev/null && break; sleep 0.2; done
  grep -lq '^REVIEW=1$' "$AGENTS_HOME"/tasks/*/meta.env
}


@test "model and investigator names use distinct consistent bold colors across rounds in attributions and review bylines" {
  cat > "$AGENTS_HOME/agents.conf" <<CONF
declare -A AGENT_TYPE=([gemini-1]=gemini [claude-haiku]=claude [codex-1]=codex)
declare -A AGENT_MODEL=([gemini-1]="gemini-3.8-flash-medium" [claude-haiku]="haiku" [codex-1]="gpt-6.1-sol")
NORMAL_AGENTS=(gemini-1 claude-haiku codex-1)
WORKER_AGENTS=()
declare -A INVESTIGATORS_TYPE=([inv-astra]=codex [inv-opus]=claude [inv-kimi]=opencode [inv-gemini-argon]=gemini)
declare -A INVESTIGATORS_MODEL=([inv-astra]="gpt-6-astra" [inv-opus]="opus" [inv-kimi]="openrouter/@preset/kimi-inv" [inv-gemini-argon]="gemini-4-argon-high")
CONF
  source "$REPO_ROOT/bin/agent"
  local g c x cg cc cx ca co ck cag b_astra
  g=$(agent_desc gemini-1)
  c=$(agent_desc claude-haiku)
  x=$(agent_desc codex-1)
  [[ $g == *'\textbf{gemini-1}'* && $g == *'(gemini, model `gemini-3.8-flash-medium`)'* ]]
  [[ $c == *'\textbf{claude-haiku}'* && $c == *'(claude, model `haiku`)'* ]]
  [[ $x == *'\textbf{codex-1}'* && $x == *'(codex, model `gpt-6.1-sol`)'* ]]
  cg=$(agent_color gemini-1)
  cc=$(agent_color claude-haiku)
  cx=$(agent_color codex-1)
  [ "$cg" != "$cc" ] && [ "$cg" != "$cx" ] && [ "$cc" != "$cx" ]
  [ "$(agent_color gemini-1)" = "$cg" ]
  [ "$(agent_desc inv-opus)" = "$(agent_color_name inv-opus) (claude, model \`opus\`)" ]

  ca=$(agent_color inv-astra)
  co=$(agent_color inv-opus)
  ck=$(agent_color inv-kimi)
  cag=$(agent_color inv-gemini-argon)
  [ "$ca" != "$co" ] && [ "$ca" != "$ck" ] && [ "$ca" != "$cag" ]
  [ "$co" != "$ck" ] && [ "$co" != "$cag" ] && [ "$ck" != "$cag" ]

  mkdir -p "$AGENTS_HOME/tasks/X"
  printf 'ID=X\nAGENT=inv-astra\nTYPE=codex\nMODEL=gpt-6-astra\nKIND=investigation\nINV_ID=66\n' > "$AGENTS_HOME/tasks/X/meta.env"
  b_astra=$(review_byline X color)
  [ "$b_astra" = "Review by the team-agents investigator $(agent_color_name inv-astra) (codex, model gpt-6-astra), request #66." ]
}

@test "agent-gh links repository filenames in PR descriptions, comments and reviews to their GitHub file contents" {
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$BATS_TEST_TMPDIR/ghw" "$AGENTS_HOME/logs"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"
  export AGENT_REAL_GH=$REPO_ROOT/tests/unit/stubs/gh
  export AGENT_GITHUB_TRUSTED_RESOLVED="mateusz-szczyrzyca ms-ai-agents"
  export AGENT_GH_REPO_URL="https://github.com/o/r"
  export AGENT_GH_REF="main"

  "$BATS_TEST_TMPDIR/ghw/gh" pr comment 7 --body "Local typed error core (agent/errors.go, agent/errors.json, agent/errors_gen.go generated via tools/errgen)."
  grep -qF '[`agent/errors.go`](https://github.com/o/r/blob/main/agent/errors.go)' "$STUB_HOME/gh.log"
  grep -qF '[`agent/errors.json`](https://github.com/o/r/blob/main/agent/errors.json)' "$STUB_HOME/gh.log"
  grep -qF '[`agent/errors_gen.go`](https://github.com/o/r/blob/main/agent/errors_gen.go)' "$STUB_HOME/gh.log"
  grep -qF '[`tools/errgen`](https://github.com/o/r/blob/main/tools/errgen)' "$STUB_HOME/gh.log"

  local issues
  issues=$(pr_format_issues 'Changed `agent/errors.go` and `tools/errgen`.' "https://github.com/o/r")
  [[ $issues == *'agent/errors.go'* ]]
}

@test "investigator review puts Recommendation at the top with bold green Ready to merge and bold red Needs changes icons" {
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$BATS_TEST_TMPDIR/ghw" "$AGENTS_HOME/logs"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"
  export AGENT_REAL_GH=$REPO_ROOT/tests/unit/stubs/gh
  export AGENT_GITHUB_TRUSTED_RESOLVED="mateusz-szczyrzyca ms-ai-agents"
  export AGENT_GH_REPO_URL="https://github.com/o/r"
  export AGENT_GH_REF="main"
  AGENT_REVIEW_BYLINE="Review by the team-agents investigator $(agent_color_name inv-astra) (codex, model gpt-6-astra), request #66."
  export AGENT_REVIEW_BYLINE

  jq -n --arg b $'### Summary\nGood change in `agent/errors.go`.\n\n### What I verified\nRan `make gate`.\n\n### Recommendation\n**Ready to merge**\n\n— Review by the team-agents investigator `inv-astra` (codex, model gpt-6-astra), request #66.' \
    '{commit_id:"x", event:"COMMENT", body:$b, comments:[]}' > "$BATS_TEST_TMPDIR/r1.json"
  "$BATS_TEST_TMPDIR/ghw/gh" api repos/o/r/pulls/7/reviews --method POST --input "$BATS_TEST_TMPDIR/r1.json" > /dev/null
  local body1 rec_line sum_line
  body1=$(jq -r .body "$STUB_HOME/gh-input.json")
  [[ $body1 == *'✅ $\color{green}{\textbf{Ready to merge}}$'* ]]
  [[ $body1 == *'[`agent/errors.go`](https://github.com/o/r/blob/main/agent/errors.go)'* ]]
  rec_line=$(grep -n '^### Recommendation' <<< "$body1" | cut -d: -f1)
  sum_line=$(grep -n '^### Summary' <<< "$body1" | cut -d: -f1)
  [ -n "$rec_line" ] && [ -n "$sum_line" ] && [ "$rec_line" -lt "$sum_line" ]

  jq -n --arg b $'### Summary\nFound 1 major bug.\n\n### Recommendation\n**Needs changes.** Fix the nil check.\n\n— Review by the team-agents investigator `inv-astra` (codex, model gpt-6-astra), request #66.' \
    '{commit_id:"x", event:"COMMENT", body:$b, comments:[]}' > "$BATS_TEST_TMPDIR/r2.json"
  "$BATS_TEST_TMPDIR/ghw/gh" api repos/o/r/pulls/7/reviews --method POST --input "$BATS_TEST_TMPDIR/r2.json" > /dev/null
  local body2
  body2=$(jq -r .body "$STUB_HOME/gh-input.json")
  [[ $body2 == *'🔧 $\color{red}{\textbf{Needs changes}}$'* ]]
  rec_line=$(grep -n '^### Recommendation' <<< "$body2" | cut -d: -f1)
  sum_line=$(grep -n '^### Summary' <<< "$body2" | cut -d: -f1)
  [ -n "$rec_line" ] && [ -n "$sum_line" ] && [ "$rec_line" -lt "$sum_line" ]
}

@test "Claude Code attribution clause is disabled in run_claude and stripped by agent-gh and flagged by pr_format_issues" {
  source "$REPO_ROOT/bin/agent"
  grep -q -- '--settings' "$REPO_ROOT/bin/agent"
  grep -q 'includeCoAuthoredBy' "$REPO_ROOT/bin/agent"

  mkdir -p "$BATS_TEST_TMPDIR/ghw" "$AGENTS_HOME/logs"
  ln -sf "$REPO_ROOT/bin/agent" "$BATS_TEST_TMPDIR/ghw/gh"
  export AGENT_REAL_GH=$REPO_ROOT/tests/unit/stubs/gh
  export AGENT_GITHUB_TRUSTED_RESOLVED="mateusz-szczyrzyca ms-ai-agents"
  export AGENT_GH_REPO_URL="https://github.com/o/r"
  export AGENT_GH_REF="main"

  : > "$STUB_HOME/gh.log"
  "$BATS_TEST_TMPDIR/ghw/gh" pr create --title "Fix" --body $'Summary of fix.\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\nCo-Authored-By: Claude <noreply@anthropic.com>'
  run ! grep -q 'Generated with' "$STUB_HOME/gh.log"
  run ! grep -q 'Co-Authored-By: Claude' "$STUB_HOME/gh.log"

  local issues
  issues=$(pr_format_issues $'Summary.\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)' "https://github.com/o/r")
  [[ $issues == *'Claude Code'* ]]
}
