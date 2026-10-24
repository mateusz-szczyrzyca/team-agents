#!/usr/bin/env bats
# "done" is a status name passed as an argument (wait_status "$t" done), not the end of a loop.
# shellcheck disable=SC1010
# Skipped tests are exceptions, not placeholders (docs/tdd.md "Skipped tests"): a skip added in a round must be
# listed with its reason in the PR description's "## Skipped tests" section, and every run's prompt shows the
# repository's skipped tests so they get filled. Plus the PR attribution: one line per team, coordinator, member.

load helpers

# meta id key — a value from a task's meta.env
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "a skipped test added in a round goes back unless the PR lists it under Skipped tests" {
  printf 'ROUND_BOUNCES=1\n' >> "$AGENTS_HOME/agents.conf"
  echo '[{"number": 5, "url": "https://github.com/o/r/pull/5", "state": "OPEN", "body": "## Plan\nparser"}]' > "$HOME/gh-prs.json"
  scenario '{"solo": {"new": "pending:2:1/2", "resume": "round-done:1/2"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  [ "$(meta "$t" RUN)" = 2 ]
  grep -q 'Skipped tests' "$HOME/prompt-solo-resume.txt"
  grep -q 'parser_test.go' "$HOME/prompt-solo-resume.txt"
}

@test "a skipped test listed with its reason in the PR's Skipped tests section passes" {
  echo '[{"number": 5, "url": "https://github.com/o/r/pull/5", "state": "OPEN", "body": "## Plan\nparser\n\n## Skipped tests\n- `parser_test.go`: needs the board on USB"}]' > "$HOME/gh-prs.json"
  scenario '{"solo": {"new": "pending:2:1/2"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  [ "$(meta "$t" RUN)" = 1 ]
}

@test "every run's prompt lists the skipped tests of the repository" {
  cd "$HOME/repo"
  printf 'package p\n\nimport "testing"\n\nfunc TestOld(t *testing.T) {\n\tt.Skip("needs the board")\n}\n' > old_test.go
  git add old_test.go && git -c user.name=t -c user.email=t@t commit -q -m "a skipped test"
  scenario '{"solo": {"new": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  grep -q 'Skipped tests in this repository' "$AGENTS_HOME/tasks/$t/prompt-1.md"
  grep -q 'old_test.go:6' "$AGENTS_HOME/tasks/$t/prompt-1.md"
}

@test "the PR attribution of a solo task names the agent in code format" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  grep -q '^- agent: `solo` (claude' "$AGENTS_HOME/tasks/$t/prompt-1.md"
}

@test "the PR attribution of a team has one line per team, coordinator and member, in code format" {
  scenario '{"lead": {"new": "done"}, "member": {"new": "done"}}'
  agent run --team lead,member "team task"
  t=$(task_of lead)
  wait_grep 'Opened by team-agents' "$AGENTS_HOME/tasks/$t/prompt-1.md"
  grep -q '^   - team: `[0-9-]*team`$' "$AGENTS_HOME/tasks/$t/prompt-1.md"
  grep -q '^   - coordinator: `lead` (claude' "$AGENTS_HOME/tasks/$t/prompt-1.md"
  grep -q '^   - member: `member` (claude' "$AGENTS_HOME/tasks/$t/prompt-1.md"
}

@test "skip detection counts skip statements of each kind of test file — not assertions, comments, strings or Markdown" {
  # the cases of the former pending detector tests (tests/unit/pending_detector.bats), for the new detector
  source "$REPO_ROOT/bin/agent"
  cd "$HOME/repo"
  printf '%s\n' 'package p' 'func TestA(t *testing.T) {' '	t.Skip("needs the board")' '	s.Skipf("no %s", "gpu")' \
    '	want := "t.Skip(quoted)"' '	// t.Skip("comment")' '}' > a_test.go
  printf '%s\n' '@test "x" {' '  skip "needs a device"' '  run grep -q "skip" f' "  expected='skip fixture'" \
    '  cat > f_test.go <<X' '  t.Skip("Go in a heredoc of a bats file")' 'X' '}' > b.bats
  printf '%s\n' '  t.Skip("in Markdown")' > notes.md
  printf '%s\n' '  t.Skip("not a test file")' > main.go
  git add a_test.go b.bats notes.md main.go
  run skips_in_repo "$HOME/repo"
  [ "$status" -eq 0 ]
  [ "$output" = $'a_test.go:3: t.Skip("needs the board")\na_test.go:4: s.Skipf("no %s", "gpu")\nb.bats:2: skip "needs a device"' ]
}
