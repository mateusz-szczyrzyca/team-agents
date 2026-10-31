#!/usr/bin/env bats
# A stale browser form must not write or resume a later remote question round.

load helpers
load remote_control_homes

# setup_file builds the real web handler and lead/node protocol for the isolated homes.
setup_file() {
  export BUILD_DIR="$BATS_FILE_TMPDIR/bin"
  mkdir -p "$BUILD_DIR"
  (cd "$REPO_ROOT/agentbus" && go build -o "$BUILD_DIR/agentbus" .)
  (cd "$REPO_ROOT/live" && go build -o "$BUILD_DIR/agentlive" .)
}

@test "an old remote browser form cannot write or resume a newer question round" {
  LEAD_PORT=$(free_port)
  export LEAD_PORT
  setup_node
  echo '{"solo":{"new":"questions-file","resume":"done"}}' > "$NODE/scenario.json"
  start_lead
  start_node
  submit_job j-stale-form team-agents '["hw:rp2350"]' solo
  wait_job j-stale-form waiting 30
  local tid copy web_port base code
  tid=$(job_field j-stale-form TaskID)
  copy="$AGENTS_HOME/remote-questions/node2/$tid"
  wait_grep 'Which board?' "$copy/questions-1.md" 30
  cp "$BUILD_DIR/agentlive" "$HOME/.local/bin/agentlive"
  web_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
  base="http://127.0.0.1:$web_port"
  printf '\nAGENT_WEB_LISTEN=127.0.0.1:%s\n' "$web_port" >> "$AGENTS_HOME/agents.conf"
  systemd-run --user --quiet --unit=control-web --setenv=AGENTS_HOME="$AGENTS_HOME" \
    --setenv=PATH="$PATH" "$HOME/.local/bin/agent" web
  wait_grep '"event":"web_started"' "$AGENTS_HOME/logs/agentlive.jsonl" 20
  curl -fsS --noproxy '*' "$base/questions/$tid@node2" > "$HOME/old-form.html"
  grep -q 'round 1' "$HOME/old-form.html"
  cp "$NODE/agents/tasks/$tid/questions-1.md" "$NODE/agents/tasks/$tid/questions-2.md"
  sed -i 's/^ROUND=1$/ROUND=2/' "$NODE/agents/tasks/$tid/meta.env"
  wait_grep 'Which board?' "$copy/questions-2.md" 30
  cp "$copy/questions-2.md" "$HOME/round-two-before.md"
  for action in draft send; do
    code=$(curl -sS --noproxy '*' -o "$HOME/rejected.html" -w '%{http_code}' \
      -H 'HX-Request: true' -H "Origin: $base" \
      --data "action=$action&round=1&c0=own&t0=stale-answer" "$base/questions/$tid@node2")
    [ "$code" = 409 ]
    cmp "$HOME/round-two-before.md" "$copy/questions-2.md"
    cmp "$HOME/round-two-before.md" "$NODE/agents/tasks/$tid/questions-2.md"
    [ "$(job_field j-stale-form State)" = waiting ]
  done
  curl -fsS --noproxy '*' -H 'HX-Request: true' -H "Origin: $base" \
    --data 'action=send&round=2&c0=ok' "$base/questions/$tid@node2" > "$HOME/sent.html"
  grep -q 'Answers queued' "$HOME/sent.html"
  wait_job j-stale-form "done" 30
  [ "$(grep -c 'solo resume done' "$NODE/calls.log")" -eq 1 ]
}
