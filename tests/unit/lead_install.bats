#!/usr/bin/env bats
# install.sh writes and starts agentlead.service only when this instance is a lead (LEAD_LISTEN set in agents.conf).
# The Go builds are replaced by a stub `go` (no network, no module download, no change to live/go.mod); systemctl
# and systemd-run are the test stubs. Everything is written under the temporary HOME.

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

@test "install.sh installs no lead unit when LEAD_LISTEN is empty" {
  install_here
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/.config/systemd/user/agentlead.service" ]
  [[ $output != *"agentlead"* ]]
}

@test "install.sh writes agentlead.service running agent lead, and starts it, when LEAD_LISTEN is set" {
  echo 'LEAD_LISTEN=127.0.0.2:7780' >> "$AGENTS_HOME/agents.conf"
  install_here
  [ "$status" -eq 0 ]
  local unit="$HOME/.config/systemd/user/agentlead.service"
  [ -f "$unit" ]
  grep -qxF "ExecStart=$HOME/.local/bin/agent lead" "$unit"
  grep -qxF "Restart=on-failure" "$unit"
  grep -qxF "WantedBy=default.target" "$unit"
  [[ $output == *"listening on 127.0.0.2:7780"* ]]
  [[ $output == *"running — see README"* ]]
}

@test "install.sh removes an existing agentlead.service when LEAD_LISTEN is no longer set" {
  mkdir -p "$HOME/.config/systemd/user"
  printf '[Unit]\nDescription=stale lead\n' > "$HOME/.config/systemd/user/agentlead.service"
  install_here
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/.config/systemd/user/agentlead.service" ]
  [[ $output == *"removed agentlead.service"* ]]
}
