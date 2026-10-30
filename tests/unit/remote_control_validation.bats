#!/usr/bin/env bats
# Adversarial control data must remain data; receipts must survive retries without losing newer acknowledgements.

load helpers

# control_fixture loads the script in an isolated home and prepares one assigned waiting task without a CLI.
control_fixture() {
  # The script path is dynamic; all commands below use the isolated test configuration.
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  NODE_STATE='{"jobs":{"j1":{"task_id":"T1","assignment":1,"launched":true,"obsolete":false,"state":"waiting"}}}'
  mkdir -p "$TASKS/T1" "$LOG_DIR"
  : > "$LOG_DIR/agent.jsonl"
  printf 'ID=T1\nAGENT=solo\nSTATUS=waiting\nROUND=1\nRUN=2\nJOB_ID=j1\nJOB_LEAD=http://127.0.0.1:1\n' > "$TASKS/T1/meta.env"
  printf 'original question\n' > "$TASKS/T1/questions-1.md"
  printf '# Questions\n\n## 1. Proceed?\n**Verdict:** ok\n\n**ANSWERED**\n' > "$HOME/answer.md"
}

# answer_payload emits an exact-content payload without passing large content through an exec argument.
answer_payload() {
  local sha
  sha=$(sha256sum "$HOME/answer.md"); sha=${sha%% *}
  jq -nc --rawfile content "$HOME/answer.md" --arg sha "$sha" '{Job:"j1",TaskID:"T1",File:"questions-1.md",Content:$content,SHA256:$sha}'
}

# rejected_control requires an explicit diagnostic and preservation of the original local questions.
rejected_control() {
  jq -se --arg cause "$1" 'any(.[]; .event=="node_answer_rejected" and .cause==$cause)' "$LOG_DIR/agent.jsonl" >/dev/null
  [ "$(cat "$TASKS/T1/questions-1.md")" = 'original question' ]
}

@test "node refuses an answer for another assigned task without changing questions" {
  control_fixture
  mkdir -p "$TASKS/T2"
  printf 'other task\n' > "$TASKS/T2/questions-1.md"
  node_apply_answer "$(answer_payload | jq -c '.TaskID="T2"')"
  rejected_control NoSuchJob
  [ "$(cat "$TASKS/T2/questions-1.md")" = 'other task' ]
}

@test "node refuses question path traversal and unknown control fields" {
  control_fixture
  node_apply_answer "$(answer_payload | jq -c '.File="../outside.md"')"
  rejected_control InvalidFile
  [ ! -e "$TASKS/outside.md" ]
  node_apply_answer "$(answer_payload | jq -c '.Command="untrusted executable"')"
  rejected_control MalformedBody
}

@test "node refuses answers with a mismatched digest or content over 256 KiB" {
  control_fixture
  node_apply_answer "$(answer_payload | jq -c '.SHA256=("a"*64)')"
  rejected_control InvalidContent
  : > "$LOG_DIR/agent.jsonl"
  python3 - <<'PY'
import os
from pathlib import Path
Path(os.environ['HOME'], 'answer.md').write_text('# Questions\n\n## 1. Proceed?\n**Verdict:** ok\n' + 'x' * 262145 + '\n**ANSWERED**\n')
PY
  node_apply_answer "$(answer_payload)"
  rejected_control InvalidContent
}

@test "node bounds incoming answer and kick lists before processing any entries" {
  control_fixture
  node_handle_controls '{"Answers":[{},{},{},{},{},{},{},{},{}]}'
  jq -se 'any(.[]; .event=="node_controls_rejected" and .cause=="MalformedBody")' "$LOG_DIR/agent.jsonl" >/dev/null
  : > "$LOG_DIR/agent.jsonl"
  node_handle_controls "$(jq -nc '{Kicks:[range(65)|{}]}')"
  jq -se 'any(.[]; .event=="node_controls_rejected" and .cause=="MalformedBody")' "$LOG_DIR/agent.jsonl" >/dev/null
}

@test "a stale kick is acknowledged without stopping the newer run" {
  control_fixture
  meta_set T1 STATUS running
  node_apply_kick "$(jq -nc '{Job:"j1",TaskID:"T1",Run:1,SHA256:("a"*64)}')"
  [ "$(task_field T1 STATUS)" = running ]
  jq -e '.jobs.j1.kick_ack.Run==1 and .jobs.j1.kick_ack.TaskID=="T1"' <<< "$NODE_STATE" >/dev/null
}

@test "successful heartbeat clears only matching acknowledgements that were sent" {
  control_fixture
  NODE_STATE='{"jobs":{"j1":{"answer_ack":{"Job":"j1","TaskID":"T1","File":"questions-2.md","SHA256":"new"}},"j2":{"answer_ack":{"Job":"j2","TaskID":"T2","File":"questions-1.md","SHA256":"old"}}}}'
  node_clear_control_acks '{"AnswerAcks":[{"Job":"j1","TaskID":"T1","File":"questions-1.md","SHA256":"old"},{"Job":"j2","TaskID":"T2","File":"questions-1.md","SHA256":"old"}]}'
  jq -e '.jobs.j1.answer_ack.SHA256=="new" and (.jobs.j2|has("answer_ack")|not)' <<< "$NODE_STATE" >/dev/null
}

@test "editable remote question copies expire with their retained task mirror" {
  control_fixture
  LEAD_LISTEN=127.0.0.1:1
  export LEAD_LISTEN
  mkdir -p "$AGENTS_HOME/remote-questions/n1/T_OLD" "$AGENTS_HOME/remote-questions/n1/T_KEEP" "$BUS_DIR/remote/n1/T_KEEP"
  printf 'answer\n' > "$AGENTS_HOME/remote-questions/n1/T_OLD/questions-1.md"
  printf 'draft\n' > "$AGENTS_HOME/remote-questions/n1/T_KEEP/questions-1.md"
  printf '{}\n' > "$BUS_DIR/remote/n1/T_KEEP/metadata.json"
  supervise_remote_questions
  [ ! -d "$AGENTS_HOME/remote-questions/n1/T_OLD" ]
  [ -f "$AGENTS_HOME/remote-questions/n1/T_KEEP/questions-1.md" ]
}
