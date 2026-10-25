#!/usr/bin/env bash
# Installs everything that does not need root: agentbus and agentlive (Go, also the web interface), the `agent` script,
# the prompts and the configuration templates.
# Run it from the project directory as the user the agents will run as:  ./install.sh
# Re-running it updates the binaries, the script and the prompts; it never overwrites
# agents.conf, hardware.md or secrets.env.
set -euo pipefail

SRC=$(cd "$(dirname "$0")" && pwd)
AH="${AGENTS_HOME:-$HOME/agents}"
BIN="$HOME/.local/bin"

# ------------------------------------------------------------------ tools
# Required to build and run the orchestration: missing ones stop the installation.
REQUIRED_TOOLS=(go git jq systemd-run flock)
# Used by the orchestration for some features: pull requests (gh), archives (7z), journal logs (logger),
# detached wake-ups (setsid), command time limits (timeout), account checks (curl).
FEATURE_TOOLS=(gh 7z logger setsid timeout curl)
# What agents typically reach for while working on code: searching, reading, editing, comparing, unpacking,
# building and scripting. Missing ones do not stop the installation, but agents waste time working around them.
AGENT_TOOLS=(rg grep sed awk find xargs diff patch file tar gzip unzip make python3 curl)

# package_for tool manager — the package that provides a command (the command itself when it is the same)
package_for() {
  local tool=$1 pm=$2
  case "$pm:$tool" in
    apt:rg | dnf:rg | pacman:rg | apk:rg | zypper:rg) echo ripgrep ;;
    apt:7z) echo p7zip-full ;;
    dnf:7z) echo p7zip p7zip-plugins ;;
    pacman:7z | apk:7z | zypper:7z) echo p7zip ;;
    apt:logger) echo bsdutils ;;
    *:logger | *:setsid | *:flock) echo util-linux ;;
    *:timeout) echo coreutils ;;
    *:find | *:xargs) echo findutils ;;
    *:diff) echo diffutils ;;
    *:awk) echo gawk ;;
    pacman:gh) echo github-cli ;;
    apt:go) echo golang-go ;;
    *:go) echo go ;;
    *) echo "$tool" ;;
  esac
}

# package_manager — the system package manager ("" when none is recognised)
package_manager() {
  local pm
  for pm in apt-get dnf pacman apk zypper; do
    if command -v "$pm" >/dev/null; then
      echo "${pm%-get}"
      return
    fi
  done
}

# install_hint tool... — the command that installs the given tools with the system package manager
install_hint() {
  local pm pkgs=() t
  pm=$(package_manager)
  for t in "$@"; do
    read -ra p <<< "$(package_for "$t" "$pm")"
    pkgs+=("${p[@]}")
  done
  mapfile -t pkgs < <(printf '%s\n' "${pkgs[@]}" | sort -u)
  case $pm in
    apt) echo "sudo apt install -y ${pkgs[*]}" ;;
    dnf) echo "sudo dnf install -y ${pkgs[*]}" ;;
    pacman) echo "sudo pacman -S --needed ${pkgs[*]}" ;;
    apk) echo "sudo apk add ${pkgs[*]}" ;;
    zypper) echo "sudo zypper install -y ${pkgs[*]}" ;;
    *) echo "install with your package manager: ${pkgs[*]}" ;;
  esac
}

# missing_from tool... — prints the tools that are not on PATH (a tool may appear in several lists once)
missing_from() {
  local t
  for t in "$@"; do
    command -v "$t" >/dev/null || echo "$t"
  done
}

# has_passwordless_sudo — whether this user (the agents' user) can run sudo without a password
has_passwordless_sudo() {
  command -v sudo >/dev/null && sudo -n true 2>/dev/null
}

check_tools() {
  local missing
  mapfile -t missing < <(missing_from "${REQUIRED_TOOLS[@]}")
  if ((${#missing[@]})); then
    echo "Missing required tools: ${missing[*]}"
    echo "  $(install_hint "${missing[@]}")"
    echo "  (see README, Requirements)"
    exit 1
  fi

  local feature agent
  mapfile -t feature < <(missing_from "${FEATURE_TOOLS[@]}")
  mapfile -t agent < <(missing_from "${AGENT_TOOLS[@]}" | grep -vxF -f <(printf '%s\n' "${feature[@]}" ""))
  ((${#feature[@]} + ${#agent[@]})) || return 0

  echo
  if ((${#feature[@]})); then
    echo "WARNING: tools used by some features are missing: ${feature[*]}"
    echo "  (gh: pull requests, 7z: agent archive, logger: journal logs, setsid: wake-ups, timeout, curl)"
  fi
  if ((${#agent[@]})); then
    echo "WARNING: tools that agents commonly use while working on code are missing: ${agent[*]}"
  fi
  echo "  Install them: $(install_hint "${feature[@]}" "${agent[@]}")"
  if has_passwordless_sudo; then
    echo "  The agents' user ($(id -un)) has passwordless sudo: when an agent needs a missing tool, it installs it"
    echo "  itself (the prompts allow that and ask the agent to mention it in its report)."
  else
    echo "  The agents' user ($(id -un)) has no passwordless sudo, so agents cannot install tools themselves —"
    echo "  install them now. (With passwordless sudo the agents would install missing tools on their own; that"
    echo "  also lets them run anything as root — see the security warning in the README.)"
  fi
  echo
}

check_tools

mkdir -p "$AH"/{tasks,worktrees,bus,prompts} "$BIN"
chmod 700 "$AH"

echo "» building agentbus"
(cd "$SRC/agentbus" && go build -trimpath -ldflags='-s -w' -o "$BIN/agentbus" .)

echo "» building agentlive (the first build downloads Go modules: Bubble Tea, Lip Gloss)"
(cd "$SRC/live" && go mod tidy && go build -trimpath -ldflags='-s -w' -o "$BIN/agentlive" .)

echo "» installing agent"
install -m 755 "$SRC/bin/agent" "$BIN/agent"

echo "» prompts"
for f in "$SRC"/prompts/*.md; do
  f=${f##*/}
  if [[ -f $AH/prompts/$f ]] && ! cmp -s "$SRC/prompts/$f" "$AH/prompts/$f"; then
    cp "$AH/prompts/$f" "$AH/prompts/$f.bak"
    echo "  $f was changed locally — previous version saved as $f.bak"
  fi
  install -m 644 "$SRC/prompts/$f" "$AH/prompts/$f"
done

echo "» configuration"
if [[ ! -f $AH/agents.conf ]]; then
  install -m 644 "$SRC/config/agents.conf" "$AH/agents.conf"
elif ! cmp -s "$SRC/config/agents.conf" "$AH/agents.conf"; then
  install -m 644 "$SRC/config/agents.conf" "$AH/agents.conf.new"
  echo "  agents.conf exists — the new template was saved as agents.conf.new (compare: diff -u $AH/agents.conf $AH/agents.conf.new)"
fi
[[ -f $AH/hardware.md ]] || install -m 644 "$SRC/config/hardware.md" "$AH/hardware.md"
[[ -f $AH/secrets.env ]] || install -m 600 "$SRC/config/secrets.env.example" "$AH/secrets.env"
mkdir -p "$AH/notify"
for f in "$SRC"/examples/notify/*.sh; do
  [[ -f $AH/notify/${f##*/} ]] || install -m 755 "$f" "$AH/notify/${f##*/}"
done

echo "» web interface (agentweb.service: agent web)"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
mkdir -p "$UNIT_DIR"
cat > "$UNIT_DIR/agentweb.service" <<UNIT
[Unit]
Description=team-agents web interface (agent web)

[Service]
ExecStart=$BIN/agent web
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNIT
if systemctl --user show-environment >/dev/null 2>&1; then   # a query: is there a systemd user session?
  systemctl --user daemon-reload
  if ! systemctl --user enable agentweb.service 2>/dev/null; then
    echo "  WARNING: agentweb.service could not be enabled (start it by hand: systemctl --user start agentweb)"
  fi
  if systemctl --user restart agentweb.service; then
    echo "  running — see README, \"Web interface\" (local address: AGENT_WEB_LISTEN in agents.conf, default 127.0.0.1:8787)"
  else
    echo "  WARNING: agentweb.service did not start — see: journalctl --user -u agentweb"
  fi
else
  echo "  no systemd user session here — run the web interface with: agent web"
fi

# The lead's job listener (agentlead.service: agent lead) is installed only when this instance is a lead: LEAD_LISTEN
# is set in agents.conf (README "Several instances"). Without it no lead unit is written.
lead_listen=$(bash -c 'source "$1"; printf %s "${LEAD_LISTEN:-}"' _ "$AH/agents.conf")
if [[ -n $lead_listen ]]; then
  echo "» lead (agentlead.service: agent lead, listening on $lead_listen)"
  cat > "$UNIT_DIR/agentlead.service" <<UNIT
[Unit]
Description=team-agents lead (agent lead)

[Service]
ExecStart=$BIN/agent lead
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNIT
  if systemctl --user show-environment >/dev/null 2>&1; then   # a query: is there a systemd user session?
    systemctl --user daemon-reload
    if ! systemctl --user enable agentlead.service; then
      echo "  WARNING: agentlead.service could not be enabled (start it by hand: systemctl --user start agentlead)"
    fi
    if systemctl --user restart agentlead.service; then
      echo "  running — see README, \"Several instances\""
    else
      echo "  WARNING: agentlead.service did not start — see: journalctl --user -u agentlead"
    fi
  else
    echo "  no systemd user session here — run the lead with: agent lead"
  fi
fi

case ":$PATH:" in *":$BIN:"*) ;; *) echo "WARNING: add $BIN to your PATH (e.g. in ~/.bashrc)";; esac

echo
echo "Done. Next: README — log in to the agent CLIs, edit agents.conf, hardware.md and secrets.env, run agent authenticated."
