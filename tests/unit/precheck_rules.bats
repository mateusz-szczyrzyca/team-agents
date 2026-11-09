#!/usr/bin/env bats
# AGENTS.md's "Never" rules held through prompts and reviewers only: the precheck counted the existing test files a
# round changed and acted on nothing of it, and an --autopilot coordinator "allowed" itself a change of a protected
# test helper. round_rule_issues checks a round's diff against its base: existing tests changed, removed or renamed
# (B1); suppressions its PR description does not name (B2); production code that looks at whether it runs under test
# (B3); changes to the files that judge a round (B4). What only the human may allow, the human allows: agent allow.

load helpers

# repo — a repository with code, existing tests, a Makefile and the files that judge a round; sets r and base
repo() {
  r=$BATS_TEST_TMPDIR/r
  git init -q -b main "$r"
  mkdir -p "$r/agent" "$r/tests/unit" "$r/tools" "$r/.github/workflows"
  printf 'package main\n\nfunc f() int { return 1 }\n' > "$r/agent/f.go"
  printf 'package main\n\nimport "testing"\n\nfunc TestF(t *testing.T) {\n\tif f() != 1 {\n\t\tt.Fatal("f")\n\t}\n}\n' > "$r/agent/f_test.go"
  printf '@test "a" {\n  true\n}\n' > "$r/tests/unit/a.bats"
  printf 'helper() { true; }\n' > "$r/tests/unit/helpers.bash"
  printf 'lint:\n\tgolangci-lint run\n' > "$r/Makefile"
  printf '#!/bin/sh\nmake lint\n' > "$r/tools/gate.sh"
  printf 'on: push\n' > "$r/.github/workflows/ci.yml"
  printf '# rules\n' > "$r/AGENTS.md"
  ci base > /dev/null
  base=$(git -C "$r" rev-parse HEAD)
}

ci() { git -C "$r" add -A; git -C "$r" -c user.name=t -c user.email=t@t commit -qm "$1"; git -C "$r" rev-parse HEAD; }

@test "B1: an existing test changed, removed or renamed is the human's to allow; added tests and whitespace are not" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  printf '\n@test "b" {\n  true\n}\n' >> "$r/tests/unit/a.bats"        # a test added to an existing file
  printf '@test "n" {\n  true\n}\n' > "$r/tests/unit/new.bats"           # a new test file
  sed -i 's/^\tif f/\t if f/' "$r/agent/f_test.go"                         # whitespace only
  head=$(ci additions)
  [ -z "$(round_rule_issues "$r" "$base" "$head" T "")" ]
  sed -i 's/f() != 1/f() != 2/' "$r/agent/f_test.go"                       # an expected value changed
  git -C "$r" rm -q tests/unit/helpers.bash                                # a helper removed
  head=$(ci changes)
  out=$(round_rule_issues "$r" "$base" "$head" T "")
  [[ $out == *"human: agent/f_test.go"* ]]
  [[ $out == *"human: tests/unit/helpers.bash"*"deleted"* ]]
  git -C "$r" mv tests/unit/a.bats tests/unit/renamed.bats
  head=$(ci rename)
  # renamed — or, when git sees too little of it in the new file, deleted: either way the existing file is reported
  [[ $(round_rule_issues "$r" "$base" "$head" T "") == *"human: tests/unit/a.bats — an existing test file "* ]]
}

@test "B2: a lint suppression the PR description does not name goes back; one it names passes; skips are not B2's" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  printf '\nfunc TestG(t *testing.T) {\n\tt.Skip("later")\n}\n' >> "$r/agent/f_test.go"
  printf '\nfunc g() int { return 2 } //nolint:unused\n' >> "$r/agent/f.go"
  printf '\n@test "c" {\n  skip "later"\n}\n' >> "$r/tests/unit/a.bats"
  printf '# shellcheck disable=SC2034\nx=1\n' > "$r/tools/new.sh"
  head=$(ci suppress)
  out=$(round_rule_issues "$r" "$base" "$head" T "")
  # skipped tests are round_check's: listed in the PR's "## Skipped tests" section, with their reasons
  [[ $out != *"agent/f_test.go"* && $out != *"tests/unit/a.bats"* ]]
  [[ $out == *"disclose: agent/f.go:"*"nolint"* ]]
  [[ $out == *"disclose: tools/new.sh:"*"shellcheck disable"* ]]
  body="Suppressions: agent/f_test.go (pending), agent/f.go (unused), tests/unit/a.bats, tools/new.sh (SC2034)"
  [ -z "$(round_rule_issues "$r" "$base" "$head" T "$body")" ]
}

@test "B3: production code that looks at whether it runs under test is the human's to allow; a test file may" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  printf '\nfunc underTest() bool { return testing.Testing() }\n' >> "$r/agent/f.go"
  printf '\nfunc TestH(t *testing.T) { _ = testing.Testing() }\n' >> "$r/agent/f_test.go"
  head=$(ci looks)
  out=$(round_rule_issues "$r" "$base" "$head" T "")
  [[ $out == *"human: agent/f.go:"*"under test"* ]]
  [[ $out != *"human: agent/f_test.go"* ]]
}

@test "B4: the files that judge a round — changed, not added to — are the human's to allow" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  printf 'test:\n\tbats tests\n' >> "$r/Makefile"                        # added to: a registration
  printf '#!/bin/sh\necho check\n' > "$r/tools/check-new.sh"             # a new check
  head=$(ci additions)
  [ -z "$(round_rule_issues "$r" "$base" "$head" T "")" ]
  sed -i 's/golangci-lint run/true/' "$r/Makefile"
  printf '#!/bin/sh\ntrue\n' > "$r/tools/gate.sh"
  printf 'on: pull_request\n' > "$r/.github/workflows/ci.yml"
  printf '# rules, changed\n' > "$r/AGENTS.md"
  head=$(ci judges)
  out=$(round_rule_issues "$r" "$base" "$head" T "")
  for f in Makefile tools/gate.sh .github/workflows/ci.yml AGENTS.md; do
    [[ $out == *"human: $f"* ]] || { echo "not flagged: $f"; false; }
  done
}

@test "a change merged in from main is not the round's: only the round's own diff counts" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  git -C "$r" checkout -q -b round
  printf '\n@test "r" {\n  true\n}\n' > "$r/tests/unit/round.bats"; ci round > /dev/null
  git -C "$r" checkout -q main
  sed -i 's/true/false/' "$r/tests/unit/a.bats"; ci "main changes a test" > /dev/null   # the human's, on main
  git -C "$r" checkout -q round
  git -C "$r" -c user.name=t -c user.email=t@t merge -q --no-edit main
  head=$(git -C "$r" rev-parse HEAD)
  mb=$(git -C "$r" merge-base main "$head")
  # from the round's first base, main's change is in the diff — and seen; from the merge base it is not the round's
  [[ $(round_rule_issues "$r" "$base" "$head" T "") == *"human: tests/unit/a.bats"* ]]
  [ -z "$(round_rule_issues "$r" "$mb" "$head" T "")" ]
}

@test "agent allow <team> <path>: what only the human may allow passes for that team; an unknown team is refused" {
  run agent allow no-such-team agent/f_test.go
  [ "$status" -ne 0 ]
  scenario '{"solo": {"new": "done"}}'
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}}'
  agent run --team lead,member "team task" > /dev/null
  team=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$(task_of lead)/meta.env")
  run agent allow "$team" agent/f_test.go tests/unit/helpers.bash
  [ "$status" -eq 0 ]
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  sed -i 's/f() != 1/f() != 2/' "$r/agent/f_test.go"
  git -C "$r" rm -q tests/unit/helpers.bash
  head=$(ci allowed)
  [ -z "$(round_rule_issues "$r" "$base" "$head" "$team" "")" ]
  [[ $(round_rule_issues "$r" "$base" "$head" other-team "") == *"human: agent/f_test.go"* ]]
  agent stop --running > /dev/null
}

@test "--autopilot: a round that changes an existing test goes back to the team with the rule; no review starts" {
  mkdir -p "$HOME/repo/tests"
  printf '@test "e" {\n  true\n}\n' > "$HOME/repo/tests/existing.bats"
  git -C "$HOME/repo" add tests/existing.bats
  git -C "$HOME/repo" -c user.name=t -c user.email=t@t commit -qm "an existing test"
  git -C "$HOME/repo" push -q origin main
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  echo "AGENT_REVIEW_GATE='true'" >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-test-edit", "resume": "team-lead-done"}, "member": {"new": "team-sleep", "resume": "done"}, "inv": {"new": "review-ready"}}'
  agent run --team lead,member --autopilot "team task"
  wait_grep '"event":"precheck_rules"' "$AGENTS_HOME/logs/agent.jsonl" 150
  for _ in $(seq 1 150); do [ "$(ls -d "$AGENTS_HOME"/tasks/*-lead/ | wc -l)" -ge 2 ] && break; sleep 0.2; done
  next=$(ls -d "$AGENTS_HOME"/tasks/*-lead/ | sed -n 2p)
  # its directory comes a moment before its task.md: wait for the file's text, not the directory
  wait_grep "only the human may allow" "$next/task.md" 60
  grep -q "tests/existing.bats" "$next/task.md"
  ! grep -lq '^REVIEW=1$' "$AGENTS_HOME"/tasks/*/meta.env || false
  agent stop --running > /dev/null
}

@test "B2: the lint suppressions of Python, TypeScript and Rust count too" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  printf 'import os  # noqa: F401\nx: int = "a"  # type: ignore\n' > "$r/agent/m.py"
  printf '// @ts-ignore\nconst a: number = "x";\n/* eslint-disable no-console */\n' > "$r/agent/m.ts"
  printf '#[allow(dead_code)]\nfn f() {}\n' > "$r/agent/m.rs"
  head=$(ci others)
  out=$(round_rule_issues "$r" "$base" "$head" T "")
  [[ $out == *"disclose: agent/m.py:1"*"noqa"* ]]
  [[ $out == *"disclose: agent/m.py:2"*"type: ignore"* ]]
  [[ $out == *"disclose: agent/m.ts:1"*"ts-ignore"* ]]
  [[ $out == *"disclose: agent/m.ts:3"*"eslint-disable"* ]]
  [[ $out == *"disclose: agent/m.rs:1"*"allow"* ]]
  [ -z "$(round_rule_issues "$r" "$base" "$head" T "agent/m.py agent/m.ts agent/m.rs: why each")" ]
}

@test "B1: a data file under tests/ named in RULE_TEST_DATA (a ledger, not a test) may change; unnamed, it may not" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  printf 'tests/unit/a.bats\ta\tplanned\n' > "$r/tests/map.tsv"
  base=$(ci "a ledger")
  sed -i 's/planned/done/' "$r/tests/map.tsv"
  head=$(ci "a row done")
  [[ $(round_rule_issues "$r" "$base" "$head" T "") == *"human: tests/map.tsv"* ]]
  [ -z "$(RULE_TEST_DATA="tests/map.tsv" round_rule_issues "$r" "$base" "$head" T "")" ]
  [ -z "$(RULE_TEST_DATA="*.tsv" round_rule_issues "$r" "$base" "$head" T "")" ]   # a file name, as ROUND_EXCLUDE's
}

@test "B2: a suppression's word in a document, or in a path the round's size leaves out, is no suppression" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  mkdir -p "$r/.agents/reports" "$r/vendor/lib"
  printf 'the tests use #nosec G302 for fixtures\n' > "$r/.agents/reports/r.md"
  printf 'Never add //nolint without a reason.\n' >> "$r/README.md"
  printf 'notes: # noqa is how Python silences a line\n' > "$r/notes.txt"
  printf 'package lib\n\nfunc v() {} //nolint:unused\n' > "$r/vendor/lib/v.go"
  head=$(ci documents)
  [ -z "$(round_rule_issues "$r" "$base" "$head" T "")" ]
}
