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
