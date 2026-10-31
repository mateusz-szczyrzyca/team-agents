#!/usr/bin/env bats
# Review regressions keep remote controls scoped to the intended run and require an explicit interactive send.
load helpers

# review_kick_fixture intercepts only the command boundary; no real units or agent CLIs run.
review_kick_fixture() {
  # The repository path is provided by the isolated Bats home.
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/T1"
  printf 'ID=T1\nAGENT=solo\nSTATUS=running\nRUN=1\nJOB_ID=j1\n' > "$TASKS/T1/meta.env"
  NODE_STATE='{"jobs":{"j1":{"task_id":"T1","assignment":1,"launched":true,"obsolete":false,"state":"running"}}}'
  export REVIEW_AGENT="$REPO_ROOT/bin/agent"
  SELF="$HOME/kick-boundary"
  cat > "$SELF" <<'BOUNDARY'
#!/usr/bin/env bash
set -euo pipefail
# The parent supplies a repository script, under the isolated test HOME.
# shellcheck source=/dev/null
source "$REVIEW_AGENT"
# act keeps this fixture independent of activity streaming.
act() { :; }
# systemctl simulates a failure or a new run at the actual stop boundary.
systemctl() {
  printf '%s\n' "$*" >> "$HOME/stops"
  [[ ! -f $HOME/stop-fails ]] || return 1
  if [[ -f $HOME/roll-during-stop ]]; then
    meta_set T1 RUN 2
    meta_set T1 STATUS running
    # Exercise the old runner's exit after the new run has appeared.
    (RUNNER_ID=T1 RUNNER_DONE=0 JOB_ID=j1 TEAM='' RUN=1 runner_exit 143)
  fi
}
if [[ -f $HOME/roll-before-command ]]; then meta_set T1 RUN 2; fi
cmd_kick "$2" "$3" "${4:-}"
BOUNDARY
  chmod +x "$SELF"
}

# review_kick submits the assigned first run's control and leaves its receipt in NODE_STATE.
review_kick() {
  node_apply_kick '{"Job":"j1","TaskID":"T1","Run":1,"SHA256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}'
}

@test "a kick carries the expected run across command execution" {
  review_kick_fixture
  touch "$HOME/roll-before-command"
  review_kick
  [ ! -f "$HOME/stops" ]
  [ "$(task_field T1 STATUS)" = running ]
  [ -z "$(task_field T1 LEAD_KICK_RUN)" ]
  jq -e '.jobs.j1.kick_ack.Run==1' <<< "$NODE_STATE" >/dev/null
}

@test "a completed old kick and runner exit do not overwrite a newer run" {
  review_kick_fixture
  touch "$HOME/roll-during-stop"
  review_kick
  [ "$(task_field T1 RUN)" = 2 ]
  [ "$(task_field T1 STATUS)" = running ]
  [ -z "$(task_field T1 FINISHED)" ]
  grep -q 'agent-T1-r1.service' "$HOME/stops"
}

@test "a failed stop retains the remote kick for retry without claiming a stop" {
  review_kick_fixture
  touch "$HOME/stop-fails"
  review_kick
  jq -e '.jobs.j1 | has("kick_ack") | not' <<< "$NODE_STATE" >/dev/null
  [ "$(task_field T1 STATUS)" = running ]
  [ -z "$(task_field T1 LEAD_KICK_RUN)" ]
  [ -z "$(task_field T1 KICKED)" ]
  rm "$HOME/stop-fails"
  review_kick
  jq -e '.jobs.j1.kick_ack.Run==1' <<< "$NODE_STATE" >/dev/null
  [ "$(task_field T1 STATUS)" = stopped ]
}

# review_answer_fixture supplies an already mirrored question without launching nodes or a listener.
review_answer_fixture() {
  # The repository script is loaded only inside the test's temporary HOME.
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  export LEAD_LISTEN=127.0.0.1:1
  mkdir -p "$BUS_DIR/remote/node/T1" "$AGENTS_HOME/remote-questions/node/T1"
  printf '{"STATUS":"waiting","RUN":"1","JOB_ID":"j1"}\n' > "$BUS_DIR/remote/node/T1/metadata.json"
  printf '{"Jobs":[{"Request":{"ID":"j1","Repository":"team-agents"},"Node":"node","TaskID":"T1","State":"waiting"}]}\n' > "$BUS_DIR/jobs.json"
  printf '# Questions\n\n## 1. Proceed?\n**Verdict:**\n' > "$BUS_DIR/remote/node/T1/questions-1.md"
  cp "$BUS_DIR/remote/node/T1/questions-1.md" "$AGENTS_HOME/remote-questions/node/T1/questions-1.md"
  EDITOR="$HOME/editor"
  printf '#!/usr/bin/env bash\nsed -i "s/^\\*\\*Verdict:\\*\\*$/\\*\\*Verdict:\\*\\* ok/" "$1"\n' > "$EDITOR"
  chmod +x "$EDITOR"
  # bus records the queued content instead of contacting any service.
  bus() { cat > "$HOME/queued"; }
  # stop_answers_watch avoids systemd in this focused command fixture.
  stop_answers_watch() { :; }
}

@test "interactive remote answers validate confirm and queue filled unmarked verdicts" {
  review_answer_fixture
  cmd_answer T1@node <<< y
  [ -f "$HOME/queued" ]
  grep -q '^\*\*ANSWERED\*\*$' "$HOME/queued"
  ! grep -q '^\*\*ANSWERED\*\*$' "$BUS_DIR/remote/node/T1/questions-1.md"
}

@test "declining remote answer submission leaves an editable unqueued draft" {
  review_answer_fixture
  cmd_answer T1@node <<< n
  [ ! -f "$HOME/queued" ]
  ! grep -q '^\*\*ANSWERED\*\*$' "$AGENTS_HOME/remote-questions/node/T1/questions-1.md"
}

@test "interactive remote answers diagnose incomplete verdicts without queueing" {
  review_answer_fixture
  EDITOR=/bin/true
  run cmd_answer T1@node <<< y
  [ "$status" -ne 0 ]
  [[ $output == *'Not all questions are answered'* ]]
  [ ! -f "$HOME/queued" ]
}

@test "ShellCheck retains the incoming handover and prompt-file coverage" {
  local script
  for script in handover_failed prompt_file; do
    run make -s -C "$REPO_ROOT" -n lint-shell
    [ "$status" -eq 0 ]
    [[ $output == *"tests/unit/$script.bats"* ]]
  done
}
