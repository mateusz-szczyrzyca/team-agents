#!/usr/bin/env bats
# A retained local proxy must route human answers to its remote task.

load helpers
load remote_control_homes

@test "agent answer on a local remote-member proxy uses the mirrored editable copy" {
  LEAD_PORT=$(free_port)
  export LEAD_PORT
  setup_node
  echo '{"solo":{"new":"questions-file","resume":"done"}}' > "$NODE/scenario.json"
  start_lead
  start_node
  submit_job j-question team-agents '["hw:rp2350"]' solo
  wait_job j-question waiting 30
  local tid proxy=20261030-000000-proxy
  tid=$(job_field j-question TaskID)
  wait_grep 'Which board?' "$AGENTS_HOME/remote-questions/node2/$tid/questions-1.md" 30
  mkdir -p "$AGENTS_HOME/tasks/$proxy"
  cat > "$AGENTS_HOME/tasks/$proxy/meta.env" <<EOF
ID=$proxy
AGENT=solo@node2
KIND=remote-member
REMOTE_NODE=node2
REMOTE_AGENT=solo
TEAM=20261030-000000-team
STATUS=waiting
ROUND=0
RUN=0
JOB_ID=j-question
EOF
  cat > "$HOME/proxy-editor" <<'EDITOR'
#!/usr/bin/env bash
set -euo pipefail
sed -i 's/^\*\*Answer:\*\*$/\*\*Answer:\*\* ok/' "$1"
printf '\n**ANSWERED**\n' >> "$1"
EDITOR
  chmod +x "$HOME/proxy-editor"
  run bash -c 'EDITOR="$HOME/proxy-editor" agent answer "$1" <<< "y"' _ "$proxy"
  [ "$status" -eq 0 ]
  wait_job j-question "done" 30
  grep -q 'solo resume done' "$NODE/calls.log"
}
