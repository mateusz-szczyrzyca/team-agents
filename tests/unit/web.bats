#!/usr/bin/env bats
# "done" is a status name passed as an argument (wait_status "$t" done), not the end of a loop.
# shellcheck disable=SC1010
# End-to-end tests of the web interface: `agent web` serves the real files of a task, and answering in the browser
# resumes it through the same path as saving the questions file. Stub CLIs only, a free loopback port.

load helpers

setup_file() {
  export BUILD_DIR="$BATS_FILE_TMPDIR/bin"
  mkdir -p "$BUILD_DIR"
  (cd "$REPO_ROOT/agentbus" && go build -o "$BUILD_DIR/agentbus" .)
  (cd "$REPO_ROOT/live" && go build -o "$BUILD_DIR/agentlive" .)
}

# start_web — runs `agent web` on a free loopback port in the background; sets WEB (its URL) and WEB_PID
start_web() {
  local port
  port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  cp "$BUILD_DIR/agentlive" "$HOME/.local/bin/agentlive"
  echo "AGENT_WEB_LISTEN=127.0.0.1:$port" >> "$AGENTS_HOME/agents.conf"
  agent web > "$HOME/web.log" 2>&1 &
  WEB_PID=$!
  echo "$WEB_PID" > "$HOME/web.pid"
  WEB="http://127.0.0.1:$port"
  for _ in $(seq 1 100); do
    curl -fsS "$WEB/healthz" > /dev/null 2>&1 && return 0
    sleep 0.1
  done
  cat "$HOME/web.log"
  return 1
}

teardown() {
  if [[ -f $HOME/web.pid ]]; then kill "$(cat "$HOME/web.pid")" 2> /dev/null || true; fi
  for p in "$STUB_UNITS"/*.pid; do   # as the teardown of helpers.bash, which this one replaces
    [[ -f $p ]] && kill -TERM -- -"$(cat "$p")" 2> /dev/null || true
  done
}

@test "agent _agents-json lists every configured agent and investigator" {
  run agent _agents-json
  [ "$status" -eq 0 ]
  [ "$(jq -s length <<< "$output")" -eq 6 ]
  [ "$(jq -rs 'map(select(.name == "worker"))[0].tier' <<< "$output")" = worker ]
  [ "$(jq -rs 'map(select(.name == "inv"))[0].tier' <<< "$output")" = investigator ]
}

@test "the web interface shows the tasks, the agents and a task's activity" {
  scenario '{"solo": {"new": "questions-file"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" waiting
  start_web
  curl -fsS "$WEB/" | grep -q "href=\"/task/$t\""
  curl -fsS "$WEB/agents" | grep -q 'investigator'
  curl -fsS "$WEB/task/$t" | grep -q "sse-connect=\"/events/task/$t"
  curl -fsS "$WEB/questions" | grep -q "href=\"/questions/$t\""
}

@test "answering in the browser resumes the task; ok accepts the recommendation" {
  scenario '{"solo": {"new": "questions-file", "resume": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" waiting
  start_web
  curl -fsS "$WEB/questions/$t" | grep -q 'the simpler one (fake)'
  curl -fsS -X POST -H 'HX-Request: true' -H "Origin: $WEB" \
    --data-urlencode c0=ok --data-urlencode t0= --data-urlencode action=send "$WEB/questions/$t" | grep -q 'Answers sent'
  wait_status "$t" done
  grep -q 'the human accepts the recommendation: a) — the simpler one (fake)' "$HOME/prompt-solo-resume.txt"
}

@test "the web interface refuses proxied requests without an identity and changes from elsewhere" {
  start_web
  [ "$(curl -s -o /dev/null -w '%{http_code}' -H 'X-Forwarded-For: 203.0.113.9' "$WEB/")" = 403 ]
  [ "$(curl -s -o /dev/null -w '%{http_code}' -H 'Tailscale-User-Login: eve@example.com' "$WEB/")" = 403 ]
  [ "$(curl -s -o /dev/null -w '%{http_code}' -X POST --data action=send "$WEB/questions/x")" = 403 ]
}

@test "agent web refuses to listen on a non-loopback address" {
  cp "$BUILD_DIR/agentlive" "$HOME/.local/bin/agentlive"
  echo "AGENT_WEB_LISTEN=0.0.0.0:0" >> "$AGENTS_HOME/agents.conf"
  run agent web
  [ "$status" -ne 0 ]
  [[ $output == *"not a loopback address"* ]]
}
