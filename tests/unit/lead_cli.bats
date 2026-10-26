#!/usr/bin/env bats
# CLI commands for remote jobs and instances:
#   agent run --need <capabilities> <agent|normal|worker> "task"
#   agent run --on <instance> <agent> "task"
#   agent jobs [<id>]
#   agent jobs --cancel <id>
#   agent nodes

load helpers

teardown() {
  if [[ -f "$HOME/lead.pid" ]]; then
    local lead_pid
    lead_pid=$(cat "$HOME/lead.pid")
    if kill -0 "$lead_pid" 2>/dev/null; then
      kill -TERM "$lead_pid" 2>/dev/null || true
      wait "$lead_pid" 2>/dev/null || true
    fi
    rm -f "$HOME/lead.pid"
  fi
}

start_test_lead() {
  local port
  port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  echo "LEAD_LISTEN=127.0.0.1:$port" >> "$AGENTS_HOME/agents.conf"
  agent lead > "$HOME/lead.log" 2>&1 &
  local pid=$!
  echo "$pid" > "$HOME/lead.pid"
  wait_grep '"event":"lead_listening"' "$AGENTS_HOME/logs/agentbus.jsonl" 15
  LEAD_PORT=$port
}

@test "agent jobs and nodes require LEAD_URL or LEAD_LISTEN" {
  run agent jobs
  [ "$status" -ne 0 ]
  [[ $output == *"neither LEAD_URL nor LEAD_LISTEN is configured"* ]]

  run agent nodes
  [ "$status" -ne 0 ]
  [[ $output == *"neither LEAD_URL nor LEAD_LISTEN is configured"* ]]
}

@test "agent run --need and --on submit remote jobs to lead" {
  start_test_lead

  # Create a git repo to run inside
  mkdir -p "$HOME/src/myrepo"
  cd "$HOME/src/myrepo"
  git init -q -b main
  git config user.name "test"
  git config user.email "test@example.com"
  echo "content" > file.txt
  git add file.txt
  git commit -q -m "initial"

  # Submit job with --need
  run agent run --need hw:rp2350 worker "test task on hardware"
  [ "$status" -eq 0 ]
  [[ $output == *"Job "* && $output == *"submitted"* ]]

  # Submit job with --on
  run agent run --on node2 normal "test task on node2"
  [ "$status" -eq 0 ]
  [[ $output == *"Job "* && $output == *"submitted"* ]]

  # agent jobs lists the queued jobs and their wait reasons
  run agent jobs
  [ "$status" -eq 0 ]
  [[ $output == *"test task on hardware"* || $output == *"hw:rp2350"* ]]
  [[ $output == *"no node with hw:rp2350"* ]]

  # Extract first job ID
  local job_id
  job_id=$(agent jobs | awk '/queued/ {print $1; exit}')
  [ -n "$job_id" ]

  # agent jobs <id> shows detailed view
  run agent jobs "$job_id"
  [ "$status" -eq 0 ]
  [[ $output == *"Job: $job_id"* ]]
  [[ $output == *"State: queued"* ]]

  # agent jobs --cancel <id> cancels the job
  run agent jobs --cancel "$job_id"
  [ "$status" -eq 0 ]

  run agent jobs "$job_id"
  [ "$status" -eq 0 ]
  [[ $output == *"stopped"* ]]
}

@test "agent nodes lists registered nodes" {
  start_test_lead

  run agent nodes
  [ "$status" -eq 0 ]
  [[ $output == *"Node"* || $output == *"Last seen"* || $output == *"(no nodes registered yet)"* ]]
}
