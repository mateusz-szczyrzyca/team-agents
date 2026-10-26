#!/usr/bin/env bats
# install.sh writes and starts agentnode.service (agent node, the node daemon) only when this instance is a node:
# LEAD_URL is set in agents.conf. The Go builds are replaced by a stub `go` (no network); systemctl and systemd-run
# are the test stubs. Everything is written under the temporary HOME.

load helpers

# install_here — runs the repository's install.sh with the stubs first on PATH.
install_here() {
  mkdir -p "$HOME/stubs"
  printf '#!/usr/bin/env bash\nexit 0\n' > "$HOME/stubs/go"
  chmod +x "$HOME/stubs/go"
  ln -sf "$REPO_ROOT/tests/unit/stubs/systemctl" "$HOME/stubs/systemctl"
  ln -sf "$REPO_ROOT/tests/unit/stubs/systemd-run" "$HOME/stubs/systemd-run"
  run env PATH="$HOME/stubs:$PATH" XDG_CONFIG_HOME="$HOME/.config" bash "$REPO_ROOT/install.sh"
}

@test "install.sh installs no node unit when LEAD_URL is empty" {
  install_here
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/.config/systemd/user/agentnode.service" ]
  [[ $output != *"agentnode"* ]]
}

@test "install.sh writes agentnode.service running agent node, and starts it, when LEAD_URL is set" {
  echo 'LEAD_URL="http://127.0.0.2:7780"' >> "$AGENTS_HOME/agents.conf"
  install_here
  [ "$status" -eq 0 ]
  local unit="$HOME/.config/systemd/user/agentnode.service"
  [ -f "$unit" ]
  grep -qxF "ExecStart=$HOME/.local/bin/agent node" "$unit"
  grep -qxF "Restart=on-failure" "$unit"
  grep -qxF "WantedBy=default.target" "$unit"
  [[ $output == *"node daemon (agentnode.service: agent node, lead http://127.0.0.2:7780)"* ]]
  [[ $output == *"running — see README"* ]]
}

@test "install.sh installs no lead unit for a node that is not a lead" {
  echo 'LEAD_URL="http://127.0.0.2:7780"' >> "$AGENTS_HOME/agents.conf"
  install_here
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/.config/systemd/user/agentlead.service" ]
}

@test "agents.conf documents LEAD_URL, NODE_POLL_SEC and INSTANCE_CAPABILITIES with their defaults" {
  grep -qE '^LEAD_URL=""' "$REPO_ROOT/config/agents.conf"
  grep -qE '^NODE_POLL_SEC=10$' "$REPO_ROOT/config/agents.conf"
  grep -qE '^INSTANCE_CAPABILITIES=\(\)' "$REPO_ROOT/config/agents.conf"
}
