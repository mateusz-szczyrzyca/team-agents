#!/usr/bin/env bats
# Web configuration wiring uses a capturing executable, never the running web installation.
load helpers

@test "agent web passes the configured node heartbeat timeout" {
  cat > "$HOME/.local/bin/agentlive" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" > "$HOME/web-args"
STUB
  chmod +x "$HOME/.local/bin/agentlive"
  echo 'NODE_TTL_SEC=37' >> "$AGENTS_HOME/agents.conf"
  run agent web
  [ "$status" -eq 0 ]
  [ "$(awk '/^--node-ttl$/{getline; print}' "$HOME/web-args")" = 37 ]
}
