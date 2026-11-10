#!/usr/bin/env bats
# A worker ended H1121 with STATUS: UNCLEAR and five questions after 20 s, having run no command and read no file —
# the files the request named would have answered four of them. UNCLEAR made an honest question a result of its own;
# it also made it an easy way out. A worker's UNCLEAR (or QUESTIONS) from a run with no action — no command, no edit,
# no tool call — now goes back to it once, with what to do: read what the request names, ask only what it leaves open,
# choose what would not change the result and say so. Not sent to the requester meanwhile; an UNCLEAR after looking,
# or a second one, goes to the requester as before.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# asked new resume — a team whose member asks a worker (both workers: action new, then resume) to count files; sets m
asked() {
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario "{\"lead\": {\"new\": \"team-lead-wait\"}, \"member\": {\"new\": \"team-sleep\"}, \"worker\": {\"new\": \"$1\", \"resume\": \"$2\"}, \"worker2\": {\"new\": \"$1\", \"resume\": \"$2\"}}"
  agent run --team lead,member "team task" > /dev/null
  m=$(task_of member)
  wait_status "$m" sleeping
  "$BUILD_DIR/agentbus" delegate --dir "$AGENTS_HOME/bus" --team "$(meta "$m" TEAM)" --as member --task "$m" \
    --repo repo --branch "$(meta "$m" BRANCH)" --worktree "$(meta "$m" WORKTREE)" --agent-cmd agent \
    --home "$AGENTS_HOME" --request "count the files" --context "c" > /dev/null
}

request_state() { "$BUILD_DIR/agentbus" helper-list --dir "$AGENTS_HOME/bus" --all | sed -n 's/^H1 \[\([a-z]*\).*/\1/p'; }

ends_as() {
  for _ in $(seq 1 200); do
    case $(request_state) in queued | starting | running | "") sleep 0.2 ;; *) break ;; esac
  done
  [ "$(request_state)" = "$1" ]
}

helper_task() { grep -lx 'KIND=helper' "$AGENTS_HOME"/tasks/*/meta.env | head -n 1 | xargs dirname | xargs basename; }

@test "an UNCLEAR without looking at anything goes back to the worker once, with what to do; then its result counts" {
  asked unclear "done"
  ends_as "done"
  w=$(helper_task)
  grep -q "UNCLEAR without looking at anything" "$AGENTS_HOME/tasks/$w/activity.jsonl"
  grep -q "read what the request names" "$AGENTS_HOME/tasks/$w/prompt-2.md"
  ! grep -q "RESULT H1 \[unclear\]" "$AGENTS_HOME/bus/messages.jsonl" || false   # the requester never got it
  agent stop --running > /dev/null
}

@test "a second UNCLEAR without looking goes to the requester: sent back once only" {
  asked unclear unclear
  ends_as unclear
  [ "$(grep -c "UNCLEAR without looking at anything" "$AGENTS_HOME/tasks/$(helper_task)/activity.jsonl")" -eq 1 ]
  agent stop --running > /dev/null
}

@test "an UNCLEAR after looking (a command run) goes to the requester at once" {
  asked look-unclear "done"
  ends_as unclear
  ! grep -q "UNCLEAR without looking at anything" "$AGENTS_HOME/tasks/$(helper_task)/activity.jsonl" || false
  agent stop --running > /dev/null
}

@test "the worker's prompt: read what the request names before asking; what would not change the result, choose" {
  grep -q "read what the request names" "$REPO_ROOT/prompts/helper.md"
  grep -q "say so in Notes" "$REPO_ROOT/prompts/helper.md"
}
