#!/usr/bin/env bats
# A worker that cannot do a request without an answer — and gets none from its requester — finished with
# "STATUS: FAILED" and "Result: UNCLEAR: ...": an honest question counted as a failure, and a model that asked was
# worse in every statistic than one that guessed. UNCLEAR is now a result of its own: the worker ends with
# "STATUS: UNCLEAR" and its questions; the request's state is "unclear" (not "failed"), the requester gets the
# questions as the result, and the worker's task is done — it gave a valid answer. The former shape (STATUS: FAILED
# with Result: UNCLEAR) is read the same way.

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

# request_state — H1's state, as helper-list shows it
request_state() { "$BUILD_DIR/agentbus" helper-list --dir "$AGENTS_HOME/bus" --all | sed -n 's/^H1 \[\([a-z]*\).*/\1/p'; }

# ends_as state — waits until H1 ends, then checks its state
ends_as() {
  for _ in $(seq 1 150); do
    case $(request_state) in queued | starting | running | "") sleep 0.2 ;; *) break ;; esac
  done
  [ "$(request_state)" = "$1" ]
}

helper_task() { grep -lx 'KIND=helper' "$AGENTS_HOME"/tasks/*/meta.env | head -n 1 | xargs dirname | xargs basename; }

@test "a worker's STATUS: UNCLEAR is the request's state unclear, its questions the result; its task is done" {
  asked unclear
  ends_as unclear
  wait_grep "RESULT H1 [unclear]" "$AGENTS_HOME/bus/messages.jsonl" 30
  grep -q "Which directory" "$AGENTS_HOME/bus/messages.jsonl"
  wait_status "$(helper_task)" 'done'
  agent stop --running > /dev/null
}

@test "the former shape — STATUS: FAILED with Result: UNCLEAR — is unclear too" {
  asked unclear-legacy
  ends_as unclear
  agent stop --running > /dev/null
}

@test "a worker's plain STATUS: FAILED is still a failure" {
  asked failed-result
  ends_as failed
  agent stop --running > /dev/null
}

@test "the worker's prompt has STATUS: UNCLEAR; the protocol tells a requester what to do with it" {
  grep -q "STATUS: DONE | FAILED | UNCLEAR" "$REPO_ROOT/prompts/helper.md"
  grep -q "\[unclear\]" "$REPO_ROOT/prompts/protocol.md"
}

@test "agent debug --agent counts a worker's unclear answers apart from its failures" {
  asked unclear
  ends_as unclear
  w=$(meta "$(helper_task)" AGENT)
  agent debug --agent "$w" --out "$BATS_TEST_TMPDIR/d.txt" > /dev/null
  grep -q "== worker requests it answered with questions (unclear)" "$BATS_TEST_TMPDIR/d.txt"
  grep -q "^H1 \[unclear" "$BATS_TEST_TMPDIR/d.txt"
  agent stop --running > /dev/null
}
