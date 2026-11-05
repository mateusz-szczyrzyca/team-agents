package main

import (
	"strconv"
	"strings"
)

// Config owns a Bash configuration snapshot; absent names differ from empty values.
// Array keys retain their Bash indices, including sparse indexed arrays.
type Config struct {
	Scalars     map[string]string            // scalar settings, including explicitly empty strings
	Indexed     map[string]map[string]string // indexed arrays, keyed by decimal Bash index
	Associative map[string]map[string]string // associative arrays, including declared empty maps
}

// ConfigContext supplies startup paths and host values without reading the installation.
type ConfigContext struct {
	Home       string // HOME before sourcing configuration
	AgentsHome string // AGENTS_HOME before sourcing configuration
	Hostname   string // HOSTNAME used for the default instance name
	UID        string // decimal user ID used for the default runtime directory
	Self       string // executable path available to configuration as SELF
}

// ConfigRequest describes an isolated configuration evaluation, never a secrets load.
type ConfigRequest struct {
	Args    []string      // caller arguments visible while sourcing configuration
	File    string        // executable Bash configuration file
	Env     []string      // complete subprocess environment; nil inherits the executor's environment
	Context ConfigContext // explicit startup values overriding the environment
}

// newConfig allocates independent containers for the three Bash declaration shapes.
func newConfig() Config {
	return Config{Scalars: make(map[string]string), Indexed: make(map[string]map[string]string), Associative: make(map[string]map[string]string)}
}

// applyConfigDefaults consumes a snapshot and applies the oracle's post-source defaults.
func applyConfigDefaults(c Config, ctx ConfigContext) Config {
	if c.Scalars == nil {
		c.Scalars = make(map[string]string)
	}
	if c.Indexed == nil {
		c.Indexed = make(map[string]map[string]string)
	}
	if c.Associative == nil {
		c.Associative = make(map[string]map[string]string)
	}
	if ctx.AgentsHome == "" {
		ctx.AgentsHome = ctx.Home + "/agents"
	}
	// Configuration may change these before the oracle evaluates its path defaults.
	if value, ok := configScalar(c, "HOME"); ok {
		ctx.Home = value
	}
	if value, ok := configScalar(c, "AGENTS_HOME"); ok {
		ctx.AgentsHome = value
	}
	if value, ok := configScalar(c, "HOSTNAME"); ok {
		ctx.Hostname = value
	}
	for _, setting := range configSettings(ctx) {
		// AUTO_MERGE_HUMAN_PATHS alone uses ${VAR-default}, retaining explicit emptiness.
		value, present := configScalar(c, setting.name)
		if (present && value != "") || (present && setting.name == "AUTO_MERGE_HUMAN_PATHS") {
			continue
		}
		setConfigScalar(c, setting.name, setting.value)
	}
	// These are unconditional assignments, not configuration overrides, in the oracle.
	setConfigScalar(c, "REVIEW_HISTORY_MAX", "100")
	setConfigScalar(c, "ROUND_ALWAYS_EXCLUDE", ".agents/reports/")
	for _, name := range []string{"NORMAL_AGENTS", "WORKER_AGENTS", "INSTANCE_CAPABILITIES"} {
		if !configDeclared(c, name) {
			c.Indexed[name] = make(map[string]string)
		}
	}
	for _, name := range []string{"AGENT_EXTRA_ARGS", "AGENT_MODEL", "INVESTIGATORS_TYPE", "INVESTIGATORS_MODEL", "INVESTIGATORS_EXTRA_ARGS", "ROUND_MAX_LINES_REPO", "NODE_REPOS"} {
		if !configDeclared(c, name) {
			c.Associative[name] = make(map[string]string)
		}
	}
	if configCount(c, "INVESTIGATORS_TYPE") == 0 {
		// Legacy values are folded only when the list-format type map is empty.
		name, _ := configScalar(c, "INVESTIGATOR_NAME")
		for _, suffix := range []string{"TYPE", "MODEL", "EXTRA_ARGS"} {
			value, _ := configScalar(c, "INVESTIGATOR_"+suffix)
			setConfigElement(c, "INVESTIGATORS_"+suffix, name, value)
		}
	}
	setConfigScalar(c, "INV_SLOTS", strconv.Itoa(configCount(c, "INVESTIGATORS_TYPE")))
	return c
}

// configSetting describes a scalar default; values stay strings to retain shell arithmetic.
type configSetting struct {
	name  string // documented Bash variable name
	value string // expanded default when unset or empty (except the unset-only override)
}

// configSettings returns independent definitions in oracle order, without mutable global state.
func configSettings(ctx ConfigContext) []configSetting {
	return []configSetting{
		{"CLAUDE_BIN", ctx.Home + "/.local/bin/claude"},
		{"CODEX_BIN", ctx.Home + "/.local/bin/codex"},
		{"AGENTBUS_BIN", ctx.Home + "/.local/bin/agentbus"},
		{"RUN_PATH", ctx.Home + "/.local/bin:/usr/local/bin:/usr/bin:/bin"},
		{"NOTIFY_CMD", ""}, {"QUESTIONS_EDITOR", "vscode"},
		{"VSCODE_SERVER_DIRS", ctx.Home + "/.vscode-server " + ctx.Home + "/.vscode-server-insiders " + ctx.Home + "/.cursor-server"},
		{"AGENT_CONSULT_MIN", "5"}, {"NOTIFY_DIR", ctx.AgentsHome + "/notifications"},
		{"NOTIFY_SCRIPT", ""}, {"NOTIFY_MAX_CHARS", "1500"},
		{"AGENT_LANGUAGE", "English"}, {"REPO_LANGUAGE", "English"},
		{"GIT_NAME_FORMAT", "%a (team-agents)"}, {"GIT_EMAIL", ""}, {"GIT_SIGNING_KEY", ""},
		{"ENV_NOTES_FILE", ctx.AgentsHome + "/hardware.md"}, {"BRANCH_PREFIX", "agents"},
		{"CLAUDE_MAX_TURNS", "300"}, {"CLAUDE_TOOL_SEARCH", "false"}, {"CLAUDE_BASH_TIMEOUT_MIN", "20"},
		{"AGY_BIN", ctx.Home + "/.local/bin/agy"}, {"OPENCODE_BIN", ctx.Home + "/.opencode/bin/opencode"},
		{"OPENCODE_BASH_TIMEOUT_MS", "600000"}, {"AGY_PRINT_TIMEOUT", "8h"},
		{"INVESTIGATOR_NAME", "investigator"}, {"INVESTIGATOR_TYPE", "claude"},
		{"INVESTIGATOR_MODEL", ""}, {"INVESTIGATOR_EXTRA_ARGS", ""}, {"INVESTIGATION_QUEUE_MAX", "3"},
		{"AGENT_INVESTIGATION_SEED", ""}, {"AGENT_HANG_MIN", "30"}, {"AGENT_HANG_STRIKES", "3"},
		{"AGENT_INVESTIGATION_EXCLUSIVE", "no"}, {"AGENT_IDLE_RUN_SEC", "120"},
		{"AGENT_WAKE_LOOP_MAX", "4"}, {"AGENT_WAKE_RATE_MAX", "30"}, {"AGENT_WAKE_LOOP_PAUSE_MIN", "15"},
		{"AGENT_QUOTA_PAUSE_MIN", "30"}, {"AGENT_RELEASE_PAUSE_MIN", "20"},
		{"AGENT_SUPERVISE_GRACE_SEC", "120"}, {"AGENT_RESTART_MAX", "2"},
		{"AGENT_RESTART_GRACE_MIN", "10"}, {"AGENT_TEAM_ABANDON_MIN", "60"},
		{"AGENT_RATE_LIMIT_PAUSE_MIN", "5"}, {"AGENT_NETWORK_PAUSE_MIN", "15"}, {"AGENT_NETWORK_MAX_PAUSES", "12"},
		{"AGENT_GITHUB_COLORS", "#0969DA #BC4C00 #8250DF #1B7C83 #BF3989 #9A6700 #57606A #953800 #3192AA #A475F9"},
		{"AGENT_REVIEW_DRAW", "random"}, {"AGENT_REVIEW_ROTATE_WAIT_MIN", "20"}, {"REVIEW_HISTORY_MAX", "100"},
		{"AGENT_SERENA", ""}, {"AGENT_PROMPT_CHECKPOINTS", "2"},
		{"AGENT_FRESH_SESSION_TOKENS", "80000"}, {"AGENT_FRESH_SESSION_RATIO", "2"},
		{"AGENT_LONG_RUN_STEPS", "120"}, {"AGENT_LONG_RUN_HARD", "200"},
		{"AGENT_PROMPT_ARG_MAX", "100000"}, {"AGENT_LIMIT_TAIL_LINES", "30"},
		{"AGENTLIVE_BIN", ctx.Home + "/.local/bin/agentlive"}, {"LIVE_REMIND_MIN", "1"},
		{"AGENT_WEB_LISTEN", "127.0.0.1:8787"}, {"AGENT_WEB_USERS", ""}, {"AGENT_WEB_DEVICES", ""}, {"AGENT_WEB_URL", ""},
		{"LEAD_LISTEN", ""}, {"LEAD_NODES", ""}, {"INSTANCE_NAME", strings.SplitN(ctx.Hostname, ".", 2)[0]},
		{"NODE_TTL_SEC", "90"}, {"ROUND_SOFT_LINES", "2000"}, {"ROUND_ALWAYS_EXCLUDE", ".agents/reports/"},
		{"VERIFY_MAX_FIX_PASSES", "2"}, {"AGENT_REVIEW_GATE", "auto"}, {"AGENT_REVIEW_GATE_MIN", "30"},
		{"AGENT_STAGE_POLL_SEC", "2"}, {"AGENT_PRECHECK_MAX", "2"},
		{"AUTO_MERGE_HUMAN_PATHS", "install.sh .github/ systemd/ AGENTS.md"}, {"AUTO_MERGE_HOLD_TEST_CHANGES", "yes"},
		{"AGENT_REVIEW_WAIT_MIN", "30"}, {"AGENT_HUMAN_GITHUB", ""},
		{"ROUND_MAX_LINES", "3000"}, {"ROUND_EXCLUDE", "*_gen.go go.sum package-lock.json vendor/"}, {"ROUND_BOUNCES", "2"},
		{"LEAD_URL", ""}, {"NODE_POLL_SEC", "5"},
		{"AUTH_MODEL_ANTHROPIC", "haiku"}, {"AUTH_MODEL_OPENAI", "gpt-6-luna"}, {"AUTH_MODEL_GEMINI", ""},
		{"AUTH_MODEL_OPENROUTER", ""}, {"AUTH_MODEL_DEEPINFRA", ""},
		{"AUTH_OPENROUTER_URL", "https://openrouter.ai/api/v1"}, {"AUTH_DEEPINFRA_URL", "https://api.deepinfra.com/v1/openai"},
		{"XDG_RUNTIME_DIR", "/run/user/" + ctx.UID},
	}
}

// configScalar reads the scalar expansion of a declaration without flattening arrays.
func configScalar(c Config, name string) (string, bool) {
	if value, ok := c.Scalars[name]; ok {
		return value, true
	}
	if array, ok := c.Indexed[name]; ok {
		value, present := array["0"]
		return value, present
	}
	value, present := c.Associative[name]["0"]
	return value, present
}

// configDeclared distinguishes a declared empty array from an absent variable.
func configDeclared(c Config, name string) bool {
	if _, ok := c.Scalars[name]; ok {
		return true
	}
	if _, ok := c.Indexed[name]; ok {
		return true
	}
	_, ok := c.Associative[name]
	return ok
}

// setConfigScalar models Bash's scalar assignment, which retains an existing array type.
func setConfigScalar(c Config, name, value string) {
	if array, ok := c.Indexed[name]; ok {
		array["0"] = value
		return
	}
	if array, ok := c.Associative[name]; ok {
		array["0"] = value
		return
	}
	c.Scalars[name] = value
}

// setConfigElement updates a declared investigator array without changing its shape.
func setConfigElement(c Config, name, key, value string) {
	if array, ok := c.Associative[name]; ok {
		array[key] = value
		return
	}
	if array, ok := c.Indexed[name]; ok {
		array[key] = value
		return
	}
	setConfigScalar(c, name, value)
}

// configCount matches the element count used for legacy investigator conversion and slots.
func configCount(c Config, name string) int {
	if _, ok := c.Scalars[name]; ok {
		return 1
	}
	return len(c.Indexed[name]) + len(c.Associative[name])
}
