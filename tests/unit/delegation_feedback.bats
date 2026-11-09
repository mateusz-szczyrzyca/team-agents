#!/usr/bin/env bats
# A worker's "done" said nothing of whether its result was right. Requesters rate each done result (the
# delegation_feedback tool: used, corrected, discarded); helper-list shows the rating, and agent debug --agent counts a
# worker's results by their ratings — the measure a choice of workers can rest on, rather than DONE alone.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# asked action — a team whose member asks a worker (both workers answer with action) to count files; sets m, team
asked() {
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario "{\"lead\": {\"new\": \"team-lead-wait\"}, \"member\": {\"new\": \"team-sleep\"}, \"worker\": {\"new\": \"$1\"}, \"worker2\": {\"new\": \"$1\"}}"
  agent run --team lead,member "team task" > /dev/null
  m=$(task_of member); team=$(meta "$m" TEAM)
  wait_status "$m" sleeping
  "$BUILD_DIR/agentbus" delegate --dir "$AGENTS_HOME/bus" --team "$team" --as member --task "$m" --repo repo \
    --branch "$(meta "$m" BRANCH)" --worktree "$(meta "$m" WORKTREE)" --agent-cmd agent --home "$AGENTS_HOME" \
    --request "count the files" --context "c" > /dev/null
}

request_state() { "$BUILD_DIR/agentbus" helper-list --dir "$AGENTS_HOME/bus" --all | sed -n 's/^H1 \[\([a-z]*\).*/\1/p'; }

ends_as() {
  for _ in $(seq 1 150); do
    case $(request_state) in queued | starting | running | "") sleep 0.2 ;; *) break ;; esac
  done
  [ "$(request_state)" = "$1" ]
}

# rate verdict note — H1 rated, as the delegation_feedback tool records it
rate() {
  local s=$AGENTS_HOME/bus/state.json
  jq --arg v "$1" --arg n "$2" '.helpers["1"].verdict = $v | .helpers["1"].verdict_note = $n' "$s" > "$s.new"
  mv "$s.new" "$s"
}

@test "helper-list shows a request's rating; agent debug --agent counts a worker's results by their ratings" {
  asked "done"
  ends_as "done"
  w=$("$BUILD_DIR/agentbus" helper-list --dir "$AGENTS_HOME/bus" --all | sed -n 's/^H1 \[done → \([a-z0-9]*\)\].*/\1/p')
  [ -n "$w" ]
  rate corrected "hidden files left out"
  "$BUILD_DIR/agentbus" helper-list --dir "$AGENTS_HOME/bus" --all | grep -q '^H1 \[done → [a-z0-9]*\] rated corrected'
  agent debug --agent "$w" --out "$BATS_TEST_TMPDIR/d.txt" > /dev/null
  grep -q "== its results, as their requesters rated them" "$BATS_TEST_TMPDIR/d.txt"
  grep -q "corrected 1" "$BATS_TEST_TMPDIR/d.txt"
  agent stop --running > /dev/null
}

@test "the protocol tells a requester to rate each done result with delegation_feedback" {
  grep -q "delegation_feedback" "$REPO_ROOT/prompts/protocol.md"
}
