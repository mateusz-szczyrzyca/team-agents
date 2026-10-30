#!/usr/bin/env bats
# Real loopback transport and stub CLIs in two isolated homes exercise remote human controls.

load helpers
load remote_control_homes

# ask_remote_job starts a question-producing job and returns its mirrored task ID in REMOTE_TASK.
ask_remote_job() {
  LEAD_PORT=$(free_port)
  export LEAD_PORT
  setup_node
  # Record every notification attempt independently in the node's own home.
  cp "$HOME/notify.sh" "$NODE/notify.sh"
  echo '{"solo":{"new":"questions-file","resume":"done"}}' > "$NODE/scenario.json"
  start_lead
  start_node
  submit_job j-question team-agents '["hw:rp2350"]' solo
  wait_job j-question waiting 30
  REMOTE_TASK=$(job_field j-question TaskID)
  wait_grep 'Which board?' "$AGENTS_HOME/remote-questions/node2/$REMOTE_TASK/questions-1.md" 30
}

# complete_remote_copy supplies a complete verdict using the same editable file as VS Code.
complete_remote_copy() {
  local file=$1
  sed -i 's/^\*\*Answer:\*\*$/\*\*Answer:\*\* ok/' "$file"
  printf '\n**ANSWERED**\n' >> "$file"
}

@test "job questions notify only the lead and its answer resumes the node" {
  ask_remote_job
  wait_grep "$REMOTE_TASK@node2" "$HOME/notifications.log" 20
  [ ! -f "$NODE/notifications.log" ]
  local copy="$AGENTS_HOME/remote-questions/node2/$REMOTE_TASK/questions-1.md"
  complete_remote_copy "$copy"
  run agent _answers_saved "$REMOTE_TASK@node2" 1
  [ "$status" -eq 0 ]
  wait_job j-question "done" 30
  grep -q 'solo resume done' "$NODE/calls.log"
  grep -q '\*\*ANSWERED\*\*' "$NODE/agents/tasks/$REMOTE_TASK/questions-1.md"
}

@test "lead answers queue while its listener is down and survive its restart" {
  ask_remote_job
  stop_lead
  local copy="$AGENTS_HOME/remote-questions/node2/$REMOTE_TASK/questions-1.md"
  complete_remote_copy "$copy"
  cat > "$HOME/answer-editor" <<'EDITOR'
#!/usr/bin/env bash
test -f "$1"
EDITOR
  chmod +x "$HOME/answer-editor"
  run bash -c 'EDITOR="$HOME/answer-editor" agent answer "$1"' _ "$REMOTE_TASK@node2"
  [ "$status" -eq 0 ]
  start_lead
  wait_job j-question "done" 30
  [ "$(grep -c 'solo resume done' "$NODE/calls.log")" -eq 1 ]
}

@test "a qualified lead kick reaches the node through the durable heartbeat queue" {
  LEAD_PORT=$(free_port)
  export LEAD_PORT
  setup_node
  echo '{"solo":{"new":"hang"}}' > "$NODE/scenario.json"
  start_lead
  start_node
  submit_job j-kick team-agents '["hw:rp2350"]' solo
  wait_job j-kick running 30
  local tid
  tid=$(job_field j-kick TaskID)
  run agent kick "$tid@node2" web
  [ "$status" -eq 0 ]
  wait_job j-kick stopped 30
  wait_jq 'select(.event == "task_kicked" and .from == "lead")' "$NODE/agents/logs/agent.jsonl" 20
}
