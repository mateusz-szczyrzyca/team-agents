#!/usr/bin/env bats
# Reviews before a merge (--verify, --autopilot): a round goes to review only when its head passes the gate and a
# change of code adds tests; a review ends with a verdict; --autopilot asks two investigators at once, each on its
# own, and a PR is ready only when both say ready-to-merge; a review's notification shows its verdict.

load helpers

# meta task key — a field of a task (as in verify_loop.bats)
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
# leads — the lead's tasks, oldest first
leads() { local d; for d in "$AGENTS_HOME"/tasks/*-lead/; do d=${d%/}; echo "${d##*/}"; done; }
# reviews — the review investigations' tasks: "<investigator> <task>"
reviews() { grep -l '^REVIEW=1$' "$AGENTS_HOME"/tasks/*/meta.env 2> /dev/null | while read -r m; do echo "$(sed -n 's/^AGENT=//p' "$m") $(basename "$(dirname "$m")")"; done; }

@test "review_verdict reads a review's verdict in its usual forms; the last one counts" {
  source "$REPO_ROOT/bin/agent"
  [ "$(review_verdict $'The PR.\nVERDICT: ready-to-merge')" = ready-to-merge ]
  [ "$(review_verdict $'**Verdict:** Needs-Fixes\nmore text')" = needs-fixes ]
  [ "$(review_verdict 'end: `VERDICT: incomplete`')" = incomplete ]
  [ "$(review_verdict 'no verdict here, ready-to-merge')" = "" ]
  [ "$(review_verdict $'VERDICT: ready-to-merge\nafter the re-check: VERDICT: needs-fixes')" = needs-fixes ]
}

@test "round_test_changes: code without new tests, documentation only, a new test, a changed existing test" {
  source "$REPO_ROOT/bin/agent"
  r=$BATS_TEST_TMPDIR/r
  git init -q "$r"
  printf 'package main\n\nfunc f() int { return 1 }\n' > "$r/main.go"
  printf 'package main\n\nimport "testing"\n\nfunc TestF(t *testing.T) { _ = f() }\n' > "$r/main_test.go"
  mkdir -p "$r/tests"; printf '@test "a" {\n  true\n}\n' > "$r/tests/a.bats"
  echo "# r" > "$r/README.md"
  git -C "$r" add -A; git -C "$r" -c user.name=t -c user.email=t@t commit -qm base
  base=$(git -C "$r" rev-parse HEAD)
  ci() { git -C "$r" add -A; git -C "$r" -c user.name=t -c user.email=t@t commit -qm "$1"; git -C "$r" rev-parse HEAD; }
  printf 'func g() int { return 2 }\n' >> "$r/main.go"; code=$(ci code)
  [ "$(round_test_changes "$r" "$base" "$code")" = "code=1 added=0 changed=0" ]
  git -C "$r" reset -q --hard "$base"; echo more >> "$r/README.md"; docs=$(ci docs)
  [ "$(round_test_changes "$r" "$base" "$docs")" = "code=0 added=0 changed=0" ]
  git -C "$r" reset -q --hard "$base"; printf 'func g() int { return 2 }\n' >> "$r/main.go"
  printf '\nfunc TestG(t *testing.T) { _ = g() }\n' >> "$r/main_test.go"; printf '\n@test "b" {\n  true\n}\n' >> "$r/tests/a.bats"
  added=$(ci added)
  [ "$(round_test_changes "$r" "$base" "$added")" = "code=1 added=2 changed=0" ]
  git -C "$r" reset -q --hard "$base"; sed -i 's/_ = f()/if f() != 1 { t.Fail() }/' "$r/main_test.go"; changed=$(ci changed)
  [ "$(round_test_changes "$r" "$base" "$changed")" = "code=1 added=0 changed=1" ]
}

@test "--autopilot: a round whose head fails the gate goes back to the team with the gate's output; no review starts" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  echo "AGENT_REVIEW_GATE='echo \"FAIL: 3 tests\"; exit 1'" >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-done", "resume": "team-lead-done"}, "member": {"new": "team-sleep", "resume": "done"}, "inv": {"new": "review-ready"}}'
  agent run --team lead,member --autopilot "team task"
  wait_grep "gate" "$HOME/notifications.log" 150
  [ "$(leads | wc -l)" -ge 2 ]
  grep -q "FAIL: 3 tests" "$AGENTS_HOME/tasks/$(leads | sed -n 2p)/task.md"
  [ -z "$(reviews)" ]
}

@test "--autopilot: two reviews start together on two investigators; both ready-to-merge — the PR is ready" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  printf 'INVESTIGATORS_TYPE[inv2]=claude\nAGENT_REVIEW_GATE=true\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-done"}, "member": {"new": "team-sleep", "resume": "done"}, "inv": {"new": "review-ready"}, "inv2": {"new": "review-ready"}}'
  agent run --team lead,member --autopilot "team task"
  wait_grep "both reviews: ready to merge" "$HOME/notifications.log" 150
  [ "$(reviews | wc -l)" -eq 2 ]
  [ "$(reviews | cut -d' ' -f1 | sort -u | wc -l)" -eq 2 ]   # two different investigators
  [ "$(grep -c '✅ ready to merge' "$HOME/notifications.log")" -eq 2 ]
  [ "$(meta "$(leads | head -1)" REVIEW_RESULT)" = approved ]
}

@test "--autopilot: one review needs fixes — the team goes on with that review's findings" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  printf 'INVESTIGATORS_TYPE[inv2]=claude\nAGENT_REVIEW_GATE=true\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-done", "resume": "team-lead-wait"}, "member": {"new": "team-sleep", "resume": "done"}, "inv": {"new": "review-ready"}, "inv2": {"new": "review-fixes"}}'
  agent run --team lead,member --autopilot "team task"
  for _ in $(seq 1 750); do [ "$(leads | wc -l)" -ge 2 ] && break; sleep 0.2; done
  second=$(leads | sed -n 2p)
  [ -n "$second" ]
  # a resume makes the task's directory and its text before its meta.env: wait for that, then for its start
  for _ in $(seq 1 300); do grep -q '^STATUS=' "$AGENTS_HOME/tasks/$second/meta.env" 2> /dev/null && break; sleep 0.2; done   # not there yet: polled again
  wait_status "$second" 'starting|running|sleeping|done' 120
  grep -q "the cache never expires" "$AGENTS_HOME/tasks/$second/task.md"
  grep -q "Fix pass 1" "$AGENTS_HOME/tasks/$second/task.md"
}

@test "--verify: a review's notification shows its verdict" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  echo 'AGENT_REVIEW_GATE=true' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-done", "resume": "team-lead-wait"}, "member": {"new": "team-sleep", "resume": "done"}, "inv": {"new": "review-fixes"}}'
  agent run --team lead,member --verify "team task"
  wait_grep "🔧 needs fixes" "$HOME/notifications.log" 150
}
