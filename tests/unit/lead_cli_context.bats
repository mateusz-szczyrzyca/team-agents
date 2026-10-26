#!/usr/bin/env bats
# Remote job submission preserves repository context and node listings expose heartbeat freshness.

load helpers

# teardown stops and reaps only this file's isolated lead, checking unexpected cleanup failures.
teardown() {
  [[ -f "$HOME/lead.pid" ]] || return 0
  local lead_pid rc=0
  lead_pid=$(cat "$HOME/lead.pid")
  if kill -0 "$lead_pid" 2> "$HOME/lead-cleanup.log"; then
    if ! kill -TERM "$lead_pid" 2>> "$HOME/lead-cleanup.log"; then
      cat "$HOME/lead-cleanup.log" >&2
      return 1
    fi
    wait "$lead_pid" || rc=$?
    if [[ $rc -ne 0 && $rc -ne 143 ]]; then
      cat "$HOME/lead.log" >&2
      return 1
    fi
  fi
}

# start_context_lead starts an actual lead adapter with an isolated loopback port and deterministic node identity.
start_context_lead() {
  local port
  port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  export CONTEXT_LEAD_URL="http://127.0.0.1:$port"
  printf 'LEAD_LISTEN=127.0.0.1:%s\nINSTANCE_NAME=context-lead\n' "$port" >> "$AGENTS_HOME/agents.conf"
  agent lead > "$HOME/lead.log" 2>&1 &
  echo "$!" > "$HOME/lead.pid"
  wait_grep '"event":"lead_listening"' "$AGENTS_HOME/logs/agentbus.jsonl" 15
}

# fail_git_operation wraps real git but injects a diagnostic failure for one revision lookup.
fail_git_operation() {
  export CONTEXT_REAL_GIT
  CONTEXT_REAL_GIT=$(command -v git)
  export CONTEXT_GIT_FAIL=$1
  mkdir -p "$HOME/context-bin"
  cat > "$HOME/context-bin/git" <<'GIT'
#!/usr/bin/env bash
set -euo pipefail
for arg in "$@"; do
  if [[ $arg == "$CONTEXT_GIT_FAIL" ]]; then
    echo 'injected git lookup failure' >&2
    exit 42
  fi
done
exec "$CONTEXT_REAL_GIT" "$@"
GIT
  chmod +x "$HOME/context-bin/git"
  export PATH="$HOME/context-bin:$PATH"
}

@test "remote job from linked worktree uses main clone repository name and current branch" {
  start_context_lead
  git worktree add -q -b topic "$HOME/wt"
  cd "$HOME/wt"
  run agent run --need os:linux worker 'linked checkout task'
  [ "$status" -eq 0 ]
  run curl -fsS --noproxy '*' --max-time 2 "$CONTEXT_LEAD_URL/v1/jobs"
  [ "$status" -eq 0 ]
  [ "$(jq -r '.[0].Request.Repository' <<< "$output")" = repo ]
  [ "$(jq -r '.[0].Request.BaseRef' <<< "$output")" = topic ]
}

@test "remote job refuses branch lookup failure without submitting a fallback main job" {
  start_context_lead
  fail_git_operation --abbrev-ref
  run agent run --need os:linux worker 'must not submit'
  [ "$status" -ne 0 ]
  [[ $output == *'injected git lookup failure'* ]]
  run curl -fsS --noproxy '*' --max-time 2 "$CONTEXT_LEAD_URL/v1/jobs"
  [ "$status" -eq 0 ]
  [ "$(jq 'length' <<< "$output")" -eq 0 ]
}

@test "remote job refuses common directory lookup failure without submitting a job" {
  start_context_lead
  fail_git_operation --git-common-dir
  run agent run --need os:linux worker 'must not submit'
  [ "$status" -ne 0 ]
  [[ $output == *'injected git lookup failure'* ]]
  run curl -fsS --noproxy '*' --max-time 2 "$CONTEXT_LEAD_URL/v1/jobs"
  [ "$status" -eq 0 ]
  [ "$(jq 'length' <<< "$output")" -eq 0 ]
}

@test "remote job deliberately uses main for detached HEAD" {
  start_context_lead
  git checkout -q --detach
  run agent run --need os:linux worker 'detached checkout task'
  [ "$status" -eq 0 ]
  run curl -fsS --noproxy '*' --max-time 2 "$CONTEXT_LEAD_URL/v1/jobs"
  [ "$status" -eq 0 ]
  [ "$(jq -r '.[0].Request.BaseRef' <<< "$output")" = main ]
}

@test "agent nodes renders a registered node including exact last seen heartbeat" {
  start_context_lead
  run curl -fsS --noproxy '*' --max-time 2 -H 'Content-Type: application/json' \
    -d '{"Node":"context-lead","Capabilities":["os:linux","hw:rp2350"],"FreeAgents":[{"Name":"worker","Tier":"worker"}],"WebURL":"http://example.com/node"}' \
    "$CONTEXT_LEAD_URL/v1/heartbeat"
  [ "$status" -eq 0 ]
  local last_seen
  last_seen=$(curl -fsS --noproxy '*' --max-time 2 "$CONTEXT_LEAD_URL/v1/nodes" | jq -r '.[0].LastSeen')
  [[ -n $last_seen && $last_seen != null ]]
  run agent nodes
  [ "$status" -eq 0 ]
  [[ $output == *'LAST SEEN'* ]]
  [[ $output == *context-lead* ]]
  [[ $output == *online* ]]
  [[ $output == *'hw:rp2350,os:linux'* ]]
  [[ $output == *worker* ]]
  [[ $output == *'http://example.com/node'* ]]
  [[ $output == *"$last_seen"* ]]
}
