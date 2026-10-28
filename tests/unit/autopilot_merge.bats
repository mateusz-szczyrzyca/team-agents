#!/usr/bin/env bats
# --autopilot after two ready-to-merge reviews: the PR is merged by the script — unless it touches a path the human
# merges (AUTO_MERGE_HUMAN_PATHS) or changes existing tests; the gate is run on the merged base; the next round
# starts. agent autopilot <team> off stops it; a review pair waiting long for investigators is reported.

load helpers

# meta task key — a field of a task (as in verify_loop.bats)
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }
# leads — the lead's tasks, oldest first
leads() { local d; for d in "$AGENTS_HOME"/tasks/*-lead/; do d=${d%/}; echo "${d##*/}"; done; }

# pair_setup — a PR to merge, two investigators that both say ready-to-merge, the gate passing
pair_setup() {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  printf 'INVESTIGATORS_TYPE[inv2]=claude\n' >> "$AGENTS_HOME/agents.conf"
}

@test "merge_blockers: a path the human merges, and changed existing tests, keep the PR for the human" {
  source "$REPO_ROOT/bin/agent"
  r=$BATS_TEST_TMPDIR/r
  git init -q "$r"
  mkdir -p "$r/.github" "$r/src"
  echo a > "$r/install.sh"; echo b > "$r/.github/ci.yml"; echo c > "$r/src/x.go"
  git -C "$r" add -A; git -C "$r" -c user.name=t -c user.email=t@t commit -qm base
  base=$(git -C "$r" rev-parse HEAD)
  echo c2 >> "$r/src/x.go"; git -C "$r" add -A; git -C "$r" -c user.name=t -c user.email=t@t commit -qm code
  plain=$(git -C "$r" rev-parse HEAD)
  [ -z "$(AUTO_MERGE_HUMAN_PATHS="install.sh .github/" merge_blockers "$r" "$base" "$plain" "code=1 added=1 changed=0")" ]
  echo a2 >> "$r/install.sh"; echo b2 >> "$r/.github/ci.yml"; git -C "$r" add -A; git -C "$r" -c user.name=t -c user.email=t@t commit -qm protected
  prot=$(git -C "$r" rev-parse HEAD)
  out=$(AUTO_MERGE_HUMAN_PATHS="install.sh .github/" merge_blockers "$r" "$base" "$prot" "code=1 added=1 changed=0")
  [[ $out == *install.sh* && $out == *.github/ci.yml* ]]
  out=$(AUTO_MERGE_HUMAN_PATHS="" merge_blockers "$r" "$base" "$plain" "code=1 added=0 changed=2")
  [[ $out == *"changes existing tests"* ]]
}

@test "--autopilot: both reviews ready-to-merge — the PR is merged, the gate passes on the base, the next round starts" {
  pair_setup
  echo 'AGENT_REVIEW_GATE=true' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-round:1/2", "resume": "team-lead-wait"}, "member": {"new": "team-sleep", "resume": "team-sleep"}, "inv": {"new": "review-ready"}, "inv2": {"new": "review-ready"}}'
  agent run --team lead,member --autopilot "a task in two rounds"
  wait_grep "pr merge https://github.com/o/r/pull/7 --merge" "$HOME/gh.log" 150
  wait_grep "round 2/2 starts" "$HOME/notifications.log" 150
  for _ in $(seq 1 300); do [ "$(leads | wc -l)" -ge 2 ] && break; sleep 0.2; done
  second=$(leads | sed -n 2p)
  [ -n "$second" ]
  wait_status "$second" 'starting|running|sleeping|done'   # created in full (a directory comes before its fields)
  [ "$(meta "$second" AUTOPILOT)" = 1 ]
  agent team "$(meta "$second" TEAM)" --close > /dev/null   # the next round's work is not part of this test
  sleep 3
}

@test "agent autopilot <team> off: after two ready-to-merge reviews the PR is not merged — the human is told" {
  pair_setup
  echo 'AGENT_REVIEW_GATE=true' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-round:1/2"}, "member": {"new": "team-sleep", "resume": "team-sleep"}, "inv": {"new": "review-ready"}, "inv2": {"new": "review-ready"}}'
  out=$(agent run --team lead,member --autopilot "a task in two rounds")
  team=$(sed -n 's/^Started team \([^:]*\):.*/\1/p' <<< "$out")
  run agent autopilot "$team" off
  [ "$status" -eq 0 ]
  wait_grep "autopilot is off" "$HOME/notifications.log" 150
  run ! grep -q "pr merge" "$HOME/gh.log"
  [ "$(leads | wc -l)" -eq 1 ]
}

@test "--autopilot: the gate fails on the merged base — the autopilot stops; no next round" {
  pair_setup
  # the gate passes before the review and fails after the merge (its second run)
  echo "AGENT_REVIEW_GATE='if [ -f \"\$HOME/gate-ran\" ]; then echo \"FAIL after the merge\"; exit 1; fi; touch \"\$HOME/gate-ran\"'" >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-round:1/2", "resume": "team-lead-wait"}, "member": {"new": "team-sleep", "resume": "team-sleep"}, "inv": {"new": "review-ready"}, "inv2": {"new": "review-ready"}}'
  agent run --team lead,member --autopilot "a task in two rounds"
  wait_grep "pr merge" "$HOME/gh.log" 150
  wait_grep "autopilot stopped" "$HOME/notifications.log" 150
  grep -q "FAIL after the merge" "$HOME/notifications.log"
  sleep 3
  [ "$(leads | wc -l)" -eq 1 ]
}

@test "a review pair waiting for two free investigators longer than AGENT_REVIEW_WAIT_MIN is reported, once" {
  local d="$AGENTS_HOME/tasks/20261028-120000-lead"
  mkdir -p "$d"
  printf 'ID=20261028-120000-lead\nAGENT=lead\nTYPE=claude\nKIND=worker\nSTATUS=done\nREPO_NAME=repo\nRUN=1\nROUND=0\nPR_NUMBER=7\nPR_URL=https://github.com/o/r/pull/7\nREVIEW_PAIR_PENDING=1\nREVIEW_PAIR_SINCE=1\n' > "$d/meta.env"
  AGENT_REVIEW_WAIT_MIN=0 AGENT_SUPERVISE_GRACE_SEC=0 agent _supervise
  wait_grep "two free investigators" "$HOME/notifications.log"
  AGENT_REVIEW_WAIT_MIN=0 AGENT_SUPERVISE_GRACE_SEC=0 agent _supervise
  [ "$(grep -c "two free investigators" "$HOME/notifications.log")" -eq 1 ]
}
