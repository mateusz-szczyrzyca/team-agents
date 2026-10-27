#!/usr/bin/env bats
# All ordinary configured agents are declared; only available agents affect routing.

load helpers

@test "heartbeat inventories busy and missing-CLI agents without offering them as free" {
  sed -n '/^node_heartbeat_json() {/,/^}/p' "$BATS_TEST_DIRNAME/../../bin/agent" > "$HOME/heartbeat.sh"
  run bash -c '
    set -euo pipefail
    source "$HOME/heartbeat.sh"
    declare -A AGENT_TYPE=([busy]=codex [free]=claude [missing]=opencode [legacyinv]=claude)
    declare -A AGENT_MODEL=([busy]=model-example [missing]=worker-example)
    declare -A NODE_MISSING_CLI=()
    INSTANCE_CAPABILITIES=(arch:arm64); INSTANCE_NAME=node-a; AGENT_WEB_URL=https://example.com/node
    is_investigator() { [[ $1 == legacyinv ]]; }
    agent_tier() { if [[ $1 == missing ]]; then echo worker; else echo normal; fi; }
    agent_cli_present() { [[ $1 != missing ]]; }
    agent_running_task() { [[ $1 == busy ]]; }
    node_activity_json() { echo "[]"; }
    log_error() { printf "%s\n" "$*" >> "$HOME/heartbeat.log"; }
    node_heartbeat_json
  '
  [ "$status" -eq 0 ]
  jq -e '.Agents == [{Name:"busy",Type:"codex",Model:"model-example",Tier:"normal"},{Name:"free",Type:"claude",Model:"",Tier:"normal"},{Name:"missing",Type:"opencode",Model:"worker-example",Tier:"worker"}]' <<< "$output"
  jq -e '.FreeAgents == [{Name:"free",Tier:"normal"}] and .Capabilities == ["arch:arm64"] and .Node == "node-a"' <<< "$output"
  grep -q 'node_agent_cli_missing agent=missing' "$HOME/heartbeat.log"
}
