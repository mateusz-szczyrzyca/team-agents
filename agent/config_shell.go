package main

import (
	"bytes"
	"context"
	"io"
	"os"
	"strconv"
	"strings"
)

// configShell sources trusted Bash once; FD3 carries data while ordinary output is discarded.
const configShell = `exec 3>&1
exec 1>&2
set -euo pipefail
HOME=$1 AGENTS_HOME=$2 HOSTNAME=$3 SELF=$4 CONF=$5
TASKS=$AGENTS_HOME/tasks PROMPTS=$AGENTS_HOME/prompts BUS_DIR=$AGENTS_HOME/bus LOG_DIR=$AGENTS_HOME/logs
shift 5
source "$CONF"
for _agent_cfg_name in "$@"; do
    # An undeclared allowlisted name is an expected absence, not a source failure.
    if _agent_cfg_decl=$(declare -p "$_agent_cfg_name" 2>/dev/null); then
        _agent_cfg_flags=${_agent_cfg_decl#declare -}
        _agent_cfg_flags=${_agent_cfg_flags%% *}
        if [[ $_agent_cfg_flags == *A* || $_agent_cfg_flags == *a* ]]; then
            _agent_cfg_tag=I
            _agent_cfg_element=i
            if [[ $_agent_cfg_flags == *A* ]]; then
                _agent_cfg_tag=A
                _agent_cfg_element=a
            fi
            printf '%s\0%s\0\0\0' "$_agent_cfg_tag" "$_agent_cfg_name" >&3
            declare -n _agent_cfg_ref="$_agent_cfg_name"
            for _agent_cfg_key in "${!_agent_cfg_ref[@]}"; do
                printf '%s\0%s\0%s\0%s\0' "$_agent_cfg_element" "$_agent_cfg_name" "$_agent_cfg_key" "${_agent_cfg_ref[$_agent_cfg_key]}" >&3
            done
            unset -n _agent_cfg_ref
        elif [[ -v $_agent_cfg_name ]]; then
            printf 'S\0%s\0\0%s\0' "$_agent_cfg_name" "${!_agent_cfg_name}" >&3
        fi
    fi
done
printf 'Z\0\0\0\0' >&3
`

// loadConfig evaluates executable Bash settings before applying startup defaults.
func (a *App) loadConfig(ctx context.Context, req ConfigRequest) (Config, *LoadConfigError) {
	cfg, err := a.loadShellConfig(ctx, req)
	if err != nil {
		return Config{}, err
	}
	// Bash evaluates a fallback lazily: an unset context fails only when its default is used.
	for name, settings := range map[string][]string{
		"HOME":        {"CLAUDE_BIN", "CODEX_BIN", "AGENTBUS_BIN", "RUN_PATH", "VSCODE_SERVER_DIRS", "AGY_BIN", "OPENCODE_BIN", "AGENTLIVE_BIN"},
		"AGENTS_HOME": {"NOTIFY_DIR", "ENV_NOTES_FILE"},
		"HOSTNAME":    {"INSTANCE_NAME"},
	} {
		if _, present := configScalar(cfg, name); present {
			continue
		}
		for _, setting := range settings {
			if value, _ := configScalar(cfg, setting); value == "" {
				return Config{}, opErr(LoadConfigShell, "configuration unset %s required by a default", name)
			}
		}
	}
	return applyConfigDefaults(cfg, req.Context), nil
}

// loadShellConfig isolates configuration output from its typed NUL snapshot.
func (a *App) loadShellConfig(ctx context.Context, req ConfigRequest) (Config, *LoadConfigError) {
	home := req.Context.AgentsHome
	if home == "" {
		home = req.Context.Home + "/agents"
	}
	args := []string{"--noprofile", "--norc", "-c", configShell, "agent-config", req.Context.Home, home, req.Context.Hostname, req.Context.Self, req.File}
	args = append(args, configNames()...)
	var output bytes.Buffer
	result, err := a.execute(ctx, Command{Name: "bash", Args: args, Env: configEnvironment(req.Env), Stdout: &output, Stderr: io.Discard})
	if err != nil {
		// Executor diagnostics can contain caller-controlled values; retain cause, redact text.
		return Config{}, &LoadConfigError{Cause: LoadConfigShell, Message: "cannot evaluate Bash configuration", Err: err}
	}
	if result.ExitCode != 0 {
		return Config{}, opErr(LoadConfigShell, "Bash configuration exited with status %d", result.ExitCode)
	}
	data := output.Bytes()
	end := []byte("Z\x00\x00\x00\x00")
	if !bytes.HasSuffix(data, end) {
		return Config{}, opErr(LoadConfigProtocol, "configuration snapshot is incomplete")
	}
	return decodeConfigNUL(bytes.TrimSuffix(data, end))
}

// configEnvironment prevents inherited startup hooks from executing before the fixed script.
func configEnvironment(env []string) []string {
	if env == nil {
		env = os.Environ()
	}
	result := make([]string, 0, len(env)+2)
	for _, entry := range env {
		name, _, _ := strings.Cut(entry, "=")
		if name != "BASH_ENV" && name != "ENV" {
			result = append(result, entry)
		}
	}
	return append(result, "BASH_ENV=", "ENV=")
}

// decodeConfigNUL validates declaration and element records from the allowlisted channel.
func decodeConfigNUL(data []byte) (Config, *LoadConfigError) {
	cfg := newConfig()
	if len(data) == 0 {
		return cfg, nil
	}
	fields := bytes.Split(data, []byte{0})
	if data[len(data)-1] != 0 || (len(fields)-1)%4 != 0 {
		return Config{}, opErr(LoadConfigProtocol, "malformed configuration record framing")
	}
	allowed := make(map[string]bool)
	for _, name := range configNames() {
		allowed[name] = true
	}
	for i := 0; i < len(fields)-1; i += 4 {
		tag, name, key, value := string(fields[i]), string(fields[i+1]), string(fields[i+2]), string(fields[i+3])
		if !allowed[name] {
			return Config{}, opErr(LoadConfigProtocol, "configuration record is outside the allowlist")
		}
		switch tag {
		case "S", "I", "A":
			if configDeclared(cfg, name) || key != "" || (tag != "S" && value != "") {
				return Config{}, opErr(LoadConfigProtocol, "invalid configuration declaration")
			}
			switch tag {
			case "S":
				cfg.Scalars[name] = value
			case "I":
				cfg.Indexed[name] = make(map[string]string)
			case "A":
				cfg.Associative[name] = make(map[string]string)
			}
		case "i", "a":
			array := cfg.Associative[name]
			if tag == "i" {
				array = cfg.Indexed[name]
				index, err := strconv.ParseUint(key, 10, 63)
				if err != nil || strconv.FormatUint(index, 10) != key {
					return Config{}, opErr(LoadConfigProtocol, "invalid indexed configuration key")
				}
			}
			if array == nil || key == "" {
				return Config{}, opErr(LoadConfigProtocol, "configuration element lacks its declaration")
			}
			if _, exists := array[key]; exists {
				return Config{}, opErr(LoadConfigProtocol, "duplicate configuration element")
			}
			array[key] = value
		default:
			return Config{}, opErr(LoadConfigProtocol, "unknown configuration record type")
		}
	}
	return cfg, nil
}

// configNames lists startup settings whose declarations may cross the Bash boundary.
func configNames() []string {
	names := []string{"HOME", "AGENTS_HOME", "HOSTNAME", "SELF", "CONF", "TASKS", "PROMPTS", "BUS_DIR", "LOG_DIR", "AGENT_TYPE", "AGENT_EXTRA_ARGS", "AGENT_MODEL", "NORMAL_AGENTS", "WORKER_AGENTS", "INSTANCE_CAPABILITIES", "INVESTIGATORS_TYPE", "INVESTIGATORS_MODEL", "INVESTIGATORS_EXTRA_ARGS", "INV_SLOTS", "ROUND_MAX_LINES_REPO", "NODE_REPOS"}
	for _, setting := range configSettings(ConfigContext{}) {
		names = append(names, setting.name)
	}
	return names
}
