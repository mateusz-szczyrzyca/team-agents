#!/usr/bin/env bats
# install.sh stopped with "gpt-6.1-sol: arithmetic syntax error" once agents.conf priced models: it reads settings
# by sourcing agents.conf in a bash of its own, and MODEL_PRICES — declared by bin/agent only — was no associative
# array there. install.sh declares it before reading agents.conf, and the template declares it like its other arrays.

load helpers

install_here() {
  mkdir -p "$HOME/stubs"
  printf '#!/usr/bin/env bash\nexit 0\n' > "$HOME/stubs/go"
  chmod +x "$HOME/stubs/go"
  ln -sf "$REPO_ROOT/tests/unit/stubs/systemctl" "$HOME/stubs/systemctl"
  ln -sf "$REPO_ROOT/tests/unit/stubs/systemd-run" "$HOME/stubs/systemd-run"
  run env PATH="$HOME/stubs:$PATH" XDG_CONFIG_HOME="$HOME/.config" bash "$REPO_ROOT/install.sh"
}

@test "install.sh reads an agents.conf whose models are priced in MODEL_PRICES, with no declaration of it" {
  printf 'MODEL_PRICES[gpt-6.1-sol]="2 0.10 10"\nMODEL_PRICES[deepinfra/Qwen/Qwen3-Coder-480B-A35B-Instruct]="0.40 0.40 1.60"\nLEAD_URL="http://127.0.0.2:7780"\n' >> "$AGENTS_HOME/agents.conf"
  install_here
  [ "$status" -eq 0 ]
  [[ $output != *"arithmetic syntax error"* ]]
  [ -f "$HOME/.config/systemd/user/agentnode.service" ]   # LEAD_URL, after the prices, was read
}

@test "the configuration template declares MODEL_PRICES: prices added to it load in a plain bash" {
  cp "$REPO_ROOT/config/agents.conf" "$BATS_TEST_TMPDIR/agents.conf"
  printf 'MODEL_PRICES[gpt-6.1-sol]="2 0.10 10"\n' >> "$BATS_TEST_TMPDIR/agents.conf"
  run bash -c 'set -eu; HOME=/nonexistent; source "$1"; printf %s "${MODEL_PRICES[gpt-6.1-sol]}"' _ "$BATS_TEST_TMPDIR/agents.conf"
  [ "$status" -eq 0 ]
  [ "$output" = "2 0.10 10" ]
}
