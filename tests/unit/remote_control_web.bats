#!/usr/bin/env bats
# A real web handler, lead registry and node daemon share only loopback transport and test artifacts.

load helpers
load remote_control_homes

# setup_file builds both frontends so the browser answer reaches the real isolated agent command.
setup_file() {
  export BUILD_DIR="$BATS_FILE_TMPDIR/bin"
  mkdir -p "$BUILD_DIR"
  (cd "$REPO_ROOT/agentbus" && go build -o "$BUILD_DIR/agentbus" .)
  (cd "$REPO_ROOT/live" && go build -o "$BUILD_DIR/agentlive" .)
}

@test "web answers travel from the lead form to a waiting node and resume it once" {
  LEAD_PORT=$(free_port)
  export LEAD_PORT
  setup_node
  echo '{"solo":{"new":"questions-file","resume":"done"}}' > "$NODE/scenario.json"
  start_lead
  start_node
  submit_job j-web-answer team-agents '["hw:rp2350"]' solo
  wait_job j-web-answer waiting 30
  local tid mirror web_port base
  tid=$(job_field j-web-answer TaskID)
  mirror="$AGENTS_HOME/bus/remote/node2/$tid/questions-1.md"
  wait_grep 'Which board?' "$mirror" 30
  # Copy readiness has its own end-to-end test; seed it here so RED fails on the actual web form.
  mkdir -p "$AGENTS_HOME/remote-questions/node2/$tid"
  cp "$mirror" "$AGENTS_HOME/remote-questions/node2/$tid/questions-1.md"
  cp "$BUILD_DIR/agentlive" "$HOME/.local/bin/agentlive"
  web_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
  base="http://127.0.0.1:$web_port"
  printf '\nAGENT_WEB_LISTEN=127.0.0.1:%s\n' "$web_port" >> "$AGENTS_HOME/agents.conf"
  # The test systemd stub owns and reaps the web process just like its fake agent units.
  systemd-run --user --quiet --unit=control-web --setenv=AGENTS_HOME="$AGENTS_HOME" \
    --setenv=PATH="$PATH" "$HOME/.local/bin/agent" web
  wait_grep '"event":"web_started"' "$AGENTS_HOME/logs/agentlive.jsonl" 20
  curl -fsS --noproxy '*' "$base/questions/$tid@node2" > "$HOME/form.html"
  grep -q 'Which board?' "$HOME/form.html"
  curl -fsS --noproxy '*' -H 'HX-Request: true' -H "Origin: $base" \
    --data 'action=send&round=1&c0=ok' "$base/questions/$tid@node2" > "$HOME/sent.html"
  grep -q 'Answers queued' "$HOME/sent.html"
  wait_job j-web-answer "done" 30
  [ "$(grep -c 'solo resume done' "$NODE/calls.log")" -eq 1 ]
}
