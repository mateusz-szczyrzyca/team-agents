#!/usr/bin/env bats
# CLI commands for remote jobs and instances:
#   agent run --need <capabilities> <agent|normal|worker> "task"
#   agent run --on <instance> <agent> "task"
#   agent jobs [<id>]
#   agent jobs --cancel <id>
#   agent nodes

load helpers

teardown() {
  [[ -f "$HOME/lead.pid" ]] || return 0
  local lead_pid rc=0
  lead_pid=$(cat "$HOME/lead.pid")
  # A process already reaped by the test is an expected non-running answer.
  if kill -0 "$lead_pid" 2> "$HOME/lead-cleanup.log"; then
    if ! kill -TERM "$lead_pid" 2>> "$HOME/lead-cleanup.log"; then
      cat "$HOME/lead-cleanup.log" >&2
      return 1
    fi
    wait "$lead_pid" || rc=$?
    # 143 is the status of a process stopped by SIGTERM; anything else is a failed cleanup.
    if [[ $rc -ne 0 && $rc -ne 143 ]]; then
      cat "$HOME/lead.log" >&2
      return 1
    fi
  fi
  rm -f "$HOME/lead.pid"
}

start_test_lead() {
  local port
  port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  echo "LEAD_LISTEN=127.0.0.1:$port" >> "$AGENTS_HOME/agents.conf"
  agent lead > "$HOME/lead.log" 2>&1 &
  local pid=$!
  echo "$pid" > "$HOME/lead.pid"
  wait_grep '"event":"lead_listening"' "$AGENTS_HOME/logs/agentbus.jsonl" 15
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
  [[ $output == *"NODE"* || $output == *"No nodes registered yet"* ]]
}

@test "agent lead and agent jobs --cancel deliver terminal notifications to NOTIFY_DIR" {
  mkdir -p "$HOME/notify"
  echo "NOTIFY_DIR=$HOME/notify" >> "$AGENTS_HOME/agents.conf"
  echo "INSTANCE_NAME=lead-test" >> "$AGENTS_HOME/agents.conf"
  start_test_lead

  mkdir -p "$HOME/src/myrepo"
  cd "$HOME/src/myrepo"
  git init -q -b main
  git config user.name "test"
  git config user.email "test@example.com"
  echo "content" > file.txt
  git add file.txt
  git commit -q -m "initial"

  run agent run --need hw:rp2350 worker "notification test task"
  [ "$status" -eq 0 ]
  [[ $output =~ Job[[:space:]]+([^[:space:]]+) ]]
  local job_id="${BASH_REMATCH[1]}"
  [ -n "$job_id" ]

  # Cancel job on the lead
  run agent jobs --cancel "$job_id"
  [ "$status" -eq 0 ]

  # Verify notification file created by agent jobs --cancel
  [ -f "$HOME/notify/$job_id-summary.md" ]
  run cat "$HOME/notify/$job_id-summary.md"
  [[ $output == *"Status: stopped"* ]]
  [[ $output == *"Job: $job_id"* ]]

  # Submit another job and report done via lead HTTP API to test agent lead adapter
  run agent run --need hw:other worker "second test task"
  [ "$status" -eq 0 ]
  [[ $output =~ Job[[:space:]]+([^[:space:]]+) ]]
  local job_id2="${BASH_REMATCH[1]}"
  [ -n "$job_id2" ]

  local port
  port=$(grep '^LEAD_LISTEN=' "$AGENTS_HOME/agents.conf" | cut -d: -f2)

  local beat_body='{"Node":"lead-test","Tier":"worker","Free":[{"Name":"w1","Tier":"worker"}],"Capabilities":["hw:other"]}'
  local assign_resp
  assign_resp=$(curl -s -X POST -H "Content-Type: application/json" -d "$beat_body" "http://127.0.0.1:$port/v1/heartbeat")
  local assignment
  assignment=$(echo "$assign_resp" | jq -r '.Assignment // empty')
  [ -n "$assignment" ]

  local rep_running='{"ID":"'"$job_id2"'","Assignment":"'"$assignment"'","State":"running","Node":"lead-test","Branch":"agents/w1/job"}'
  curl -s -X POST -H "Content-Type: application/json" -d "$rep_running" "http://127.0.0.1:$port/v1/jobs/$job_id2/state" >/dev/null

  local rep_done='{"ID":"'"$job_id2"'","Assignment":"'"$assignment"'","State":"done","Node":"lead-test","Result":"done ok"}'
  curl -s -X POST -H "Content-Type: application/json" -d "$rep_done" "http://127.0.0.1:$port/v1/jobs/$job_id2/state" >/dev/null

  [ -f "$HOME/notify/$job_id2-summary.md" ]
  run cat "$HOME/notify/$job_id2-summary.md"
  [[ $output == *"Status: done"* ]]
  [[ $output == *"Job: $job_id2"* ]]
  [[ $output == *"done ok"* ]]
}
