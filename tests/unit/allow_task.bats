#!/usr/bin/env bats
# The human's allowances (agent allow) were kept per team only. A task of no team — an --investigation, a solo
# task — was checked against the allowances of an empty team name, so nothing could ever be allowed for it: agent
# allow refused anything but a team, and the precheck's refusal printed "agent allow  <path>", without a key. Now the
# key is the task's team or, without one, the task itself; agent allow takes either; the refusal prints the whole
# command, with the key and every flagged path.

load helpers

# a_task id [team] — a task's metadata, of a team or of none
a_task() {
  mkdir -p "$AGENTS_HOME/tasks/$1"
  printf 'ID=%s\nAGENT=inv\nKIND=investigation-team\nTEAM=%s\nSTATUS=waiting\n' "$1" "${2:-}" > "$AGENTS_HOME/tasks/$1/meta.env"
}

# repo — a repository whose round changes the files that judge a round; sets r, base and head
repo() {
  r=$BATS_TEST_TMPDIR/r
  git init -q -b main "$r"
  mkdir -p "$r/tools"
  printf 'lint:\n\tgolangci-lint run\n' > "$r/Makefile"
  printf '#!/bin/sh\nmake lint\n' > "$r/tools/gate.sh"
  printf '# rules\n' > "$r/AGENTS.md"
  git -C "$r" add -A && git -C "$r" -c user.name=t -c user.email=t@t commit -qm base
  base=$(git -C "$r" rev-parse HEAD)
  sed -i 's/golangci-lint run/true/' "$r/Makefile"
  printf '#!/bin/sh\ntrue\n' > "$r/tools/gate.sh"
  printf '# rules, changed\n' > "$r/AGENTS.md"
  git -C "$r" add -A && git -C "$r" -c user.name=t -c user.email=t@t commit -qm judges
  head=$(git -C "$r" rev-parse HEAD)
}

@test "the allowances of a task's rounds are kept under its team or, of no team, under the task itself" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  a_task T-inv
  a_task T-member some-team
  [ "$(rule_allow_key T-inv)" = T-inv ]
  [ "$(rule_allow_key T-member)" = some-team ]
}

@test "agent allow <task> for a task of no team: its rounds may change those paths; a task of a team is sent to its team" {
  a_task T-inv
  a_task T-member some-team
  run agent allow T-inv AGENTS.md Makefile tools/gate.sh
  [ "$status" -eq 0 ]
  [[ $output == *"task T-inv"* ]]
  run agent allow T-inv
  [ "$output" = "$(printf 'AGENTS.md\nMakefile\ntools/gate.sh')" ]
  run agent allow T-member Makefile
  [ "$status" -ne 0 ]
  [[ $output == *"agent allow some-team"* ]]
  run agent allow no-such-thing Makefile
  [ "$status" -ne 0 ]
  [[ $output == *"no team or task"* ]]
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  [ -z "$(round_rule_issues "$r" "$base" "$head" "$(rule_allow_key T-inv)" "")" ]
  [[ $(round_rule_issues "$r" "$base" "$head" other "") == *"human: Makefile"* ]]
}

@test "the precheck's refusal prints the whole allow command: the key and each flagged path once, without line numbers" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  human=$'- AGENTS.md — changed (M); AGENTS.md: never change it\n- Makefile — 6 line(s) of an existing check file changed or removed\n- agent/f.go:12 — production code that looks at whether it runs under test (testing.Testing)\n- agent/f.go:30 — production code that looks at whether it runs under test (testing.Testing)'
  [ "$(rule_allow_command T-inv "$human")" = "agent allow T-inv AGENTS.md Makefile agent/f.go" ]
}
