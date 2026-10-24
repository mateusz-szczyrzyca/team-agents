#!/usr/bin/env bats
# agent authenticated: every agent is checked with its own model or preset; a failure shows the HTTP code, the
# model and the provider's own message (never only "usage limit reached"). A fake API on loopback stands in for
# OpenRouter and DeepInfra.

load helpers

setup_api() {
  API_PORT=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  fake-llm-api "$API_PORT" "$HOME/api.log" &
  echo $! > "$HOME/api.pid"
  for _ in $(seq 1 50); do
    curl -s -o /dev/null "http://127.0.0.1:$API_PORT/" && break
    sleep 0.1
  done
  printf 'OPENROUTER_API_KEY=or-key\nDEEPINFRA_API_KEY=di-key\n' >> "$AGENTS_HOME/secrets.env"
  cat >> "$AGENTS_HOME/agents.conf" <<CONF
AUTH_OPENROUTER_URL=http://127.0.0.1:$API_PORT/or
AUTH_DEEPINFRA_URL=http://127.0.0.1:$API_PORT/di
AGENT_TYPE[di-good]=opencode;    AGENT_MODEL[di-good]=deepinfra/zai-org/good-model
AGENT_TYPE[di-limited]=opencode; AGENT_MODEL[di-limited]=deepinfra/acme/limited-model
AGENT_TYPE[or-busy]=opencode;    AGENT_MODEL[or-busy]=openrouter/@preset/busy-pracus
CONF
}

teardown() {
  if [[ -f $HOME/api.pid ]]; then kill "$(cat "$HOME/api.pid")" 2> /dev/null || true; fi
}

@test "each agent is checked with its own model; a failure shows the HTTP code, the model and the provider's message" {
  setup_api
  run agent authenticated di-good di-limited or-busy
  [ "$status" -eq 0 ]
  [[ $(grep '^di-good ' <<< "$output") == *"deepinfra"*"zai-org/good-model"*"OK"* ]]
  [[ $(grep '^di-limited ' <<< "$output") == *"acme/limited-model"*"HTTP 402: Monthly spending limit reached"* ]]
  [[ $(grep '^or-busy ' <<< "$output") == *"@preset/busy-pracus"*"HTTP 429: Model is overloaded, try again later"* ]]
  [[ $output != *"usage limit reached"* ]]
  grep -qx '/di/chat/completions zai-org/good-model' "$HOME/api.log"
  grep -qx '/di/chat/completions acme/limited-model' "$HOME/api.log"
  grep -qx '/or/chat/completions @preset/busy-pracus' "$HOME/api.log"
}

@test "a rejected key is 'needs authentication', with the provider's message" {
  setup_api
  echo 'AGENT_TYPE[di-key]=opencode; AGENT_MODEL[di-key]=deepinfra/acme/badkey-model' >> "$AGENTS_HOME/agents.conf"
  run agent authenticated di-key
  [[ $(grep '^di-key ' <<< "$output") == *"needs authentication"*"HTTP 401: Invalid API key"* ]]
}

@test "an agent name that is not configured is refused" {
  run agent authenticated no-such-agent
  [ "$status" -ne 0 ]
  [[ $output == *"no-such-agent"* ]]
}
