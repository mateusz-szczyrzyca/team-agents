package main

import "testing"

// TestConfigDefaultFamilies checks startup paths and defaults across independent families.
func TestConfigDefaultFamilies(t *testing.T) {
	// ctx makes path and host expansion independent of the host running the test.
	ctx := ConfigContext{Home: "/tmp/config-home", AgentsHome: "/tmp/config-home/agents", Hostname: "invented.example.com", UID: "1234"}
	// got represents an executable configuration that declared no settings.
	got := applyConfigDefaults(newConfig(), ctx)
	// want spans executable paths, human settings, scheduling, review, node, and auth defaults.
	want := map[string]string{
		"CLAUDE_BIN": "/tmp/config-home/.local/bin/claude", "CODEX_BIN": "/tmp/config-home/.local/bin/codex",
		"RUN_PATH": "/tmp/config-home/.local/bin:/usr/local/bin:/usr/bin:/bin", "NOTIFY_DIR": "/tmp/config-home/agents/notifications",
		"ENV_NOTES_FILE": "/tmp/config-home/agents/hardware.md", "AGENT_WEB_LISTEN": "127.0.0.1:8787", "INSTANCE_NAME": "invented",
		"XDG_RUNTIME_DIR": "/run/user/1234", "AGENT_LANGUAGE": "English", "AGENT_HANG_MIN": "30", "AGENT_NETWORK_MAX_PAUSES": "12",
		"AGENT_RESTART_MAX": "2", "AGENT_REVIEW_DRAW": "random", "ROUND_MAX_LINES": "3000", "ROUND_BOUNCES": "2",
		"CLAUDE_TOOL_SEARCH": "false", "AUTH_MODEL_OPENAI": "gpt-6-luna", "AUTH_OPENROUTER_URL": "https://openrouter.ai/api/v1",
		"INV_SLOTS": "1", "NOTIFY_SCRIPT": "", "ROUND_ALWAYS_EXCLUDE": ".agents/reports/", "REVIEW_HISTORY_MAX": "100",
	}
	for name, value := range want {
		if actual, ok := got.Scalars[name]; !ok || actual != value {
			t.Errorf("%s: got %q (present %t), want %q", name, actual, ok, value)
		}
	}
	if got.Associative["INVESTIGATORS_TYPE"]["investigator"] != "claude" {
		t.Error("missing default legacy investigator")
	}
}

// TestConfigDefaultEmptyAndShapes preserves the oracle's unset/empty and array assignments.
func TestConfigDefaultEmptyAndShapes(t *testing.T) {
	// input includes an unset-only override, a numeric expression, and sparse arrays.
	input := newConfig()
	input.Scalars["AUTO_MERGE_HUMAN_PATHS"] = ""
	input.Scalars["AGENT_HANG_MIN"] = "5+2"
	input.Scalars["QUESTIONS_EDITOR"] = ""
	input.Scalars["HOME"] = "/changed"
	input.Scalars["AGENTS_HOME"] = "/changed/agents"
	input.Indexed["CODEX_BIN"] = map[string]string{"0": "", "7": "retained"}
	input.Indexed["NORMAL_AGENTS"] = map[string]string{"3": "normal with spaces"}
	input.Associative["AGENT_EXTRA_ARGS"] = map[string]string{"normal with spaces": "--flag 'two words'"}
	// got must preserve declared shapes while filling the effective scalar at index zero.
	got := applyConfigDefaults(input, ConfigContext{Home: "/original"})
	if got.Scalars["AUTO_MERGE_HUMAN_PATHS"] != "" || got.Scalars["AGENT_HANG_MIN"] != "5+2" || got.Scalars["QUESTIONS_EDITOR"] != "vscode" {
		t.Error("unset-only, shell arithmetic, or empty-value default changed")
	}
	if got.Indexed["CODEX_BIN"]["0"] != "/changed/.local/bin/codex" || got.Indexed["CODEX_BIN"]["7"] != "retained" {
		t.Error("scalar assignment did not preserve indexed shape and sparse elements")
	}
	if _, exists := got.Scalars["CODEX_BIN"]; exists {
		t.Error("an indexed setting also appeared as a scalar")
	}
	if got.Indexed["NORMAL_AGENTS"]["3"] != "normal with spaces" || got.Associative["AGENT_EXTRA_ARGS"]["normal with spaces"] != "--flag 'two words'" {
		t.Error("agent arrays changed while applying defaults")
	}
	if got.Scalars["NOTIFY_DIR"] != "/changed/agents/notifications" {
		t.Error("post-source context was ignored")
	}
}

// TestConfigLegacyInvestigators folds legacy scalars only when no list was configured.
func TestConfigLegacyInvestigators(t *testing.T) {
	// input uses the older single-investigator contract.
	input := newConfig()
	input.Scalars["INVESTIGATOR_NAME"] = "legacy"
	input.Scalars["INVESTIGATOR_TYPE"] = "opencode"
	input.Scalars["INVESTIGATOR_MODEL"] = "example/model"
	input.Scalars["INVESTIGATOR_EXTRA_ARGS"] = "--flag"
	// legacy receives all four values without treating extra arguments as shell syntax.
	legacy := applyConfigDefaults(input, ConfigContext{})
	if legacy.Associative["INVESTIGATORS_TYPE"]["legacy"] != "opencode" || legacy.Associative["INVESTIGATORS_MODEL"]["legacy"] != "example/model" || legacy.Associative["INVESTIGATORS_EXTRA_ARGS"]["legacy"] != "--flag" {
		t.Error("legacy investigator values were not folded into maps")
	}
	// explicit suppresses legacy conversion, even if its model map is empty.
	explicit := newConfig()
	explicit.Associative["INVESTIGATORS_TYPE"] = map[string]string{"chosen": "codex"}
	// got must retain intentionally absent models rather than impose new validation.
	got := applyConfigDefaults(explicit, ConfigContext{})
	if len(got.Associative["INVESTIGATORS_TYPE"]) != 1 || len(got.Associative["INVESTIGATORS_MODEL"]) != 0 || got.Scalars["INV_SLOTS"] != "1" {
		t.Error("explicit investigator list was backfilled or miscounted")
	}
}
