package main

import (
	"bytes"
	"context"
	"io"
	"os"
	"strconv"
	"strings"
)

// configShell sources trusted Bash once; FD3 carries data independently of ordinary output.
const configShell = `exec 3>&1
exec 1>"$7" 2>"$8"
set -euo pipefail
shopt -u patsub_replacement 2>/dev/null || : # Expected on Bash before 5.2, matching production.
HOME=$1 AGENTS_HOME=$2 HOSTNAME=$3 SELF=$4 CONF=$5
TASKS=$AGENTS_HOME/tasks PROMPTS=$AGENTS_HOME/prompts BUS_DIR=$AGENTS_HOME/bus LOG_DIR=$AGENTS_HOME/logs
readonly _agent_cfg_defaults=$6
readonly _agent_cfg_env=$9
shift 9
readonly -a _agent_cfg_names=("${@:2:$1}")
shift "$(($1 + 1))"
AGENT_SUBCOMMAND=${1:-} RUNNER_ID=""
source "$CONF"
builtin eval "$_agent_cfg_defaults"
for _agent_cfg_name in "${_agent_cfg_names[@]}"; do
    declare -n _agent_cfg_ref="$_agent_cfg_name"
    _agent_cfg_flags=
    # An undeclared allowlisted name is an expected absence, not a source failure.
    if _agent_cfg_decl=$(declare -p "${!_agent_cfg_ref}" 2>/dev/null); then
        _agent_cfg_flags=${_agent_cfg_decl#declare -}
        _agent_cfg_flags=${_agent_cfg_flags%% *}
    fi
    if [[ $_agent_cfg_flags == *A* || $_agent_cfg_flags == *a* ]]; then
        _agent_cfg_tag=I
        _agent_cfg_element=i
        if [[ $_agent_cfg_flags == *A* ]]; then
            _agent_cfg_tag=A
            _agent_cfg_element=a
        fi
        printf '%s\0%s\0\0\0' "$_agent_cfg_tag" "$_agent_cfg_name" >&3
        for _agent_cfg_key in "${!_agent_cfg_ref[@]}"; do
            printf '%s\0%s\0%s\0%s\0' "$_agent_cfg_element" "$_agent_cfg_name" "$_agent_cfg_key" "${_agent_cfg_ref[$_agent_cfg_key]}" >&3
        done
    elif [[ ${_agent_cfg_ref+set} ]]; then
        printf 'S\0%s\0\0%s\0' "$_agent_cfg_name" "${_agent_cfg_ref}" >&3
    fi
    unset -n _agent_cfg_ref
done
printf 'Z\0\0\0\0' >&3
env -0 >"$_agent_cfg_env"
`

// loadConfig evaluates executable Bash settings and startup defaults in one shell.
func (a *App) loadConfig(ctx context.Context, req ConfigRequest) (Config, *LoadConfigError) {
	return a.evaluateConfigShell(ctx, req, configDefaultShell(req.Context))
}

// loadShellConfig isolates configuration output from its typed NUL snapshot.
func (a *App) loadShellConfig(ctx context.Context, req ConfigRequest) (Config, *LoadConfigError) {
	return a.evaluateConfigShell(ctx, req, "")
}

// evaluateConfigShell retains Bash declarations through default assignments before snapshotting.
func (a *App) evaluateConfigShell(ctx context.Context, req ConfigRequest, defaults string) (cfg Config, failure *LoadConfigError) {
	// Regular files keep background configuration children from holding copy pipes open.
	streams, streamsErr := os.MkdirTemp("", "agent-config-streams-*")
	if streamsErr != nil {
		return Config{}, fromStorage(LoadConfigShell, storageErr(streamsErr, "create configuration output directory"))
	}
	defer func() {
		if removeErr := os.RemoveAll(streams); removeErr != nil {
			cfg, failure = Config{}, fromStorage(LoadConfigShell, storageErr(removeErr, "remove configuration output directory"))
		}
	}()
	stdoutPath, stderrPath, envPath := streams+"/stdout", streams+"/stderr", streams+"/env"
	home := req.Context.AgentsHome
	if home == "" {
		home = req.Context.Home + "/agents"
	}
	args := []string{"--noprofile", "--norc", "-c", configShell, "agent-config", req.Context.Home, home, req.Context.Hostname, req.Context.Self, req.File, defaults, stdoutPath, stderrPath, envPath}
	names := configNames()
	args = append(args, strconv.Itoa(len(names)))
	args = append(append(args, names...), req.Args...)
	// The snapshot has its own channel, separate from ordinary configuration output.
	output, fileErr := os.CreateTemp("", "agent-config-*.nul")
	if fileErr != nil {
		return Config{}, opErr(LoadConfigShell, "cannot create configuration snapshot: %v", fileErr)
	}
	defer func() {
		closeErr, removeErr := output.Close(), os.Remove(output.Name())
		if closeErr != nil || removeErr != nil {
			cfg, failure = Config{}, opErr(LoadConfigShell, "cannot clean up configuration snapshot: close=%v remove=%v", closeErr, removeErr)
		}
	}()
	result, err := a.execute(ctx, Command{Name: "bash", Args: args, Env: configEnvironment(req.Env), Stdout: output})
	if streamErr := replayConfigOutput(stdoutPath, a.Stdout); streamErr != nil {
		return Config{}, streamErr
	}
	if streamErr := replayConfigOutput(stderrPath, a.Stderr); streamErr != nil {
		return Config{}, streamErr
	}
	if err != nil {
		// Executor diagnostics can contain caller-controlled values; retain cause, redact text.
		return Config{}, &LoadConfigError{Cause: LoadConfigShell, Message: "cannot evaluate Bash configuration", Err: err}
	}
	if result.ExitCode != 0 {
		return Config{}, opErr(LoadConfigShell, "Bash configuration exited with status %d", result.ExitCode)
	}
	if _, seekErr := output.Seek(0, io.SeekStart); seekErr != nil {
		return Config{}, opErr(LoadConfigShell, "cannot rewind configuration snapshot: %v", seekErr)
	}
	data, readErr := io.ReadAll(output)
	if readErr != nil {
		return Config{}, opErr(LoadConfigShell, "cannot read configuration snapshot: %v", readErr)
	}
	end := []byte("Z\x00\x00\x00\x00")
	if !bytes.HasSuffix(data, end) {
		return Config{}, opErr(LoadConfigProtocol, "configuration snapshot is incomplete")
	}
	cfg, failure = decodeConfigNUL(bytes.TrimSuffix(data, end))
	if failure != nil {
		return Config{}, failure
	}
	// M1 FIX: Check for non-ENOENT read errors when loading environment exports
	if envData, rErr := os.ReadFile(envPath); rErr != nil { // #nosec G304 -- fixed filename in private temporary directory
		if !os.IsNotExist(rErr) {
			return Config{}, fromStorage(LoadConfigShell, storageErr(rErr, "read configuration environment"))
		}
	} else if len(envData) > 0 {
		for _, e := range bytes.Split(bytes.TrimSuffix(envData, []byte{0}), []byte{0}) {
			if len(e) > 0 {
				cfg.Env = append(cfg.Env, string(e))
			}
		}
	}
	return cfg, nil
}

// replayConfigOutput forwards a completed shell stream; absence means the shell never started.
func replayConfigOutput(path string, out io.Writer) *LoadConfigError {
	if out == nil {
		return nil
	}
	data, err := os.ReadFile(path) // #nosec G304 -- path is a fixed filename in the evaluator's private temporary directory.
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return fromStorage(LoadConfigShell, storageErr(err, "read configuration output"))
	}
	if _, err := io.Copy(out, bytes.NewReader(data)); err != nil {
		return fromStorage(LoadConfigShell, storageErr(err, "write configuration output"))
	}
	return nil
}

// configDefaultShell emits only trusted assignments; Bash retains attributes and evaluates indices.
func configDefaultShell(ctx ConfigContext) string {
	script := `declare -p AGENT_EXTRA_ARGS &>/dev/null || declare -A AGENT_EXTRA_ARGS=()
declare -p AGENT_MODEL &>/dev/null || declare -A AGENT_MODEL=()
declare -p NORMAL_AGENTS &>/dev/null || NORMAL_AGENTS=()
declare -p WORKER_AGENTS &>/dev/null || WORKER_AGENTS=()
`
	for _, setting := range configSettings(ConfigContext{Home: "@HOME@", AgentsHome: "@AGENTS_HOME@", UID: ctx.UID}) {
		switch setting.name {
		case "AGENTLIVE_BIN":
			script += `declare -p INVESTIGATORS_TYPE &>/dev/null || declare -A INVESTIGATORS_TYPE=()
declare -p INVESTIGATORS_MODEL &>/dev/null || declare -A INVESTIGATORS_MODEL=()
declare -p INVESTIGATORS_EXTRA_ARGS &>/dev/null || declare -A INVESTIGATORS_EXTRA_ARGS=()
if ((${#INVESTIGATORS_TYPE[@]} == 0)); then
    INVESTIGATORS_TYPE[$INVESTIGATOR_NAME]=$INVESTIGATOR_TYPE
    INVESTIGATORS_MODEL[$INVESTIGATOR_NAME]=$INVESTIGATOR_MODEL
    INVESTIGATORS_EXTRA_ARGS[$INVESTIGATOR_NAME]=$INVESTIGATOR_EXTRA_ARGS
fi
INV_SLOTS=${#INVESTIGATORS_TYPE[@]}
`
		case "LEAD_URL":
			script += "declare -p ROUND_MAX_LINES_REPO &>/dev/null || declare -A ROUND_MAX_LINES_REPO=()\ndeclare -p NODE_REPOS &>/dev/null || declare -A NODE_REPOS=()\n"
		case "AUTH_MODEL_ANTHROPIC":
			script += "declare -p INSTANCE_CAPABILITIES &>/dev/null || declare -a INSTANCE_CAPABILITIES=()\n"
		}
		value := "'" + strings.ReplaceAll(setting.value, "'", "'\\''") + "'"
		value = strings.NewReplacer("@HOME@", "'\"${HOME}\"'", "@AGENTS_HOME@", "'\"${AGENTS_HOME}\"'").Replace(value)
		if setting.name == "INSTANCE_NAME" {
			value = "${HOSTNAME%%.*}"
		}
		op := ":-"
		if setting.name == "AUTO_MERGE_HUMAN_PATHS" {
			op = "-"
		}
		if setting.name == "REVIEW_HISTORY_MAX" || setting.name == "ROUND_ALWAYS_EXCLUDE" {
			script += setting.name + "=" + value + "\n"
		} else {
			script += setting.name + "=${" + setting.name + op + value + "}\n"
		}
	}
	return script
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
