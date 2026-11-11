#!/usr/bin/env bats
# shellcheck disable=SC2034 # AGENT_REVIEW_GATE and AGENT_GATE_SCOPES are read by the sourced bin/agent
# A team writing new code next to the old (a rewrite in another language, in its own directories) ran the
# repository's whole gate on every change: minutes of the old code's tests that the change cannot touch. Now
# AGENT_GATE_SCOPES maps paths to a gate command: a change whose every path matches one entry runs that entry's
# command; any other change, and a change of nothing, runs the full gate (AGENT_REVIEW_GATE).

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# repo — a repository with old code and docs, at its base; sets r and base
repo() {
  r=$BATS_TEST_TMPDIR/r
  git init -q -b main "$r"
  mkdir -p "$r/bin" "$r/docs"
  printf 'old\n' > "$r/bin/tool"
  printf '# doc\n' > "$r/docs/a.md"
  ci base > /dev/null
  base=$(git -C "$r" rev-parse HEAD)
}

ci() { git -C "$r" add -A; git -C "$r" -c user.name=t -c user.email=t@t commit -qm "$1"; git -C "$r" rev-parse HEAD; }

@test "the gate of a change: the first scope covering every changed path; otherwise, and for no change, the full gate" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  AGENT_REVIEW_GATE=full-gate
  AGENT_GATE_SCOPES=("crates/ Cargo.toml => rust-gate" "crates/ *.md => docs-and-rust-gate")
  [ "$(review_gate_cmd "$r" "$base" "$base")" = full-gate ]               # no change
  mkdir -p "$r/crates/a/src"
  printf 'fn main() {}\n' > "$r/crates/a/src/main.rs"
  printf '[workspace]\n' > "$r/Cargo.toml"
  rust=$(ci rust)
  [ "$(review_gate_cmd "$r" "$base" "$rust")" = rust-gate ]
  printf 'more\n' >> "$r/docs/a.md"
  docs=$(ci docs)
  [ "$(review_gate_cmd "$r" "$rust" "$docs")" = docs-and-rust-gate ]     # the first entry does not cover a.md
  [ "$(review_gate_cmd "$r" "$base" "$docs")" = full-gate ]              # Cargo.toml and a.md: no single entry
  printf 'new\n' > "$r/bin/tool"
  old=$(ci old)
  [ "$(review_gate_cmd "$r" "$docs" "$old")" = full-gate ]
}

@test "the paths are the change's own: what the base gained since does not count; uncommitted work in a worktree does" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  AGENT_REVIEW_GATE=full-gate
  AGENT_GATE_SCOPES=("crates/ => rust-gate")
  git -C "$r" checkout -q -b work
  mkdir -p "$r/crates"
  printf 'x\n' > "$r/crates/x.rs"
  head=$(ci work)
  git -C "$r" checkout -q main
  printf 'main moved\n' > "$r/bin/tool"
  ci main-moved > /dev/null
  git -C "$r" checkout -q work
  [ "$(review_gate_cmd "$r" main "$head")" = rust-gate ]                 # from the merge base, not from main
  [ "$(review_gate_cmd "$r" main)" = rust-gate ]                         # the worktree, clean
  printf 'y\n' > "$r/crates/y.rs"                                         # untracked, in scope
  [ "$(review_gate_cmd "$r" main)" = rust-gate ]
  printf 'z\n' > "$r/bin/other"                                          # untracked, out of scope
  [ "$(review_gate_cmd "$r" main)" = full-gate ]
}

@test "an entry without '=>' or without a command is skipped and logged; no scopes: the full gate as before" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  repo
  AGENT_REVIEW_GATE=full-gate
  mkdir -p "$r/crates"
  printf 'x\n' > "$r/crates/x.rs"
  head=$(ci rust)
  AGENT_GATE_SCOPES=()
  [ "$(review_gate_cmd "$r" "$base" "$head")" = full-gate ]
  [ "$(review_gate_cmd "$r")" = full-gate ]                              # no range given: the full gate
  AGENT_GATE_SCOPES=("crates/ rust-gate" "crates/ => " "crates/ => rust-gate")
  [ "$(review_gate_cmd "$r" "$base" "$head")" = rust-gate ]
  grep -q '"event":"gate_scope_invalid"' "$AGENTS_HOME/logs/agent.jsonl"
}

@test "agent gate in a task's worktree runs the scope's command for a change inside it, and records it" {
  printf "AGENT_REVIEW_GATE='touch %s/full'\n" "$BATS_TEST_TMPDIR" >> "$AGENTS_HOME/agents.conf"
  printf "AGENT_GATE_SCOPES=('docs/ => touch %s/scoped')\n" "$BATS_TEST_TMPDIR" >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "questions-file"}}'
  agent run solo "a task" > /dev/null
  t=$(task_of solo)
  wait_status "$t" waiting
  wt=$(meta "$t" WORKTREE)
  mkdir -p "$wt/docs"
  printf '# new\n' > "$wt/docs/new.md"
  git -C "$wt" add docs/new.md
  git -C "$wt" -c user.name=t -c user.email=t@t commit -qm docs
  agent gate "$t" > /dev/null
  [ -e "$BATS_TEST_TMPDIR/scoped" ]
  [ ! -e "$BATS_TEST_TMPDIR/full" ]
  [ "$(jq -r '.command' "$AGENTS_HOME/gates/$t.json")" = "touch $BATS_TEST_TMPDIR/scoped" ]
  agent stop --running > /dev/null
}
