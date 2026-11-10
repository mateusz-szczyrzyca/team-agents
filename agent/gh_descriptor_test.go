package main

import (
	"testing"
)

// TestFormatAgentDescriptor tests agent descriptor formatting matching Bash agent_desc (bin/agent:1440-1443).
// Tests cover AGENT_TYPE/AGENT_MODEL and INVESTIGATORS_TYPE/INVESTIGATORS_MODEL lookups with independent fallbacks.
func TestFormatAgentDescriptor(t *testing.T) {
	tests := []struct {
		agentName string
		cfg       Config
		want      string
		testCase  string
	}{
		{
			agentName: "inv-x",
			cfg: Config{
				Associative: map[string]map[string]string{
					"INVESTIGATORS_TYPE":  {"inv-x": "codex"},
					"INVESTIGATORS_MODEL": {"inv-x": "gpt-6-astra"},
					"AGENT_TYPE":          {},
					"AGENT_MODEL":         {},
				},
			},
			want:     "`inv-x` (codex, model `gpt-6-astra`)",
			testCase: "Investigator with model from INVESTIGATORS maps",
		},
		{
			agentName: "codex-1",
			cfg: Config{
				Associative: map[string]map[string]string{
					"AGENT_TYPE":          {"codex-1": "codex"},
					"AGENT_MODEL":         {"codex-1": "gpt-6.1-sol"},
					"INVESTIGATORS_TYPE":  {},
					"INVESTIGATORS_MODEL": {},
				},
			},
			want:     "`codex-1` (codex, model `gpt-6.1-sol`)",
			testCase: "Team agent with model from AGENT maps",
		},
		{
			agentName: "claude-haiku",
			cfg: Config{
				Associative: map[string]map[string]string{
					"AGENT_TYPE":          {"claude-haiku": "claude"},
					"AGENT_MODEL":         {},
					"INVESTIGATORS_TYPE":  {},
					"INVESTIGATORS_MODEL": {},
				},
			},
			want:     "`claude-haiku` (claude)",
			testCase: "Agent with type but no model",
		},
		{
			agentName: "inv-xyz",
			cfg: Config{
				Associative: map[string]map[string]string{
					"INVESTIGATORS_TYPE":  {"inv-xyz": "opencode"},
					"INVESTIGATORS_MODEL": {},
					"AGENT_TYPE":          {},
					"AGENT_MODEL":         {},
				},
			},
			want:     "`inv-xyz` (opencode)",
			testCase: "Investigator with type but no model",
		},
		{
			agentName: "unknown-agent",
			cfg: Config{
				Associative: map[string]map[string]string{
					"AGENT_TYPE":          {},
					"AGENT_MODEL":         {},
					"INVESTIGATORS_TYPE":  {},
					"INVESTIGATORS_MODEL": {},
				},
			},
			want:     "`unknown-agent` (?)",
			testCase: "Missing type falls back to ? like Bash oracle",
		},
		{
			agentName: "shared-name",
			cfg: Config{
				Associative: map[string]map[string]string{
					"AGENT_TYPE":          {"shared-name": "claude"},
					"AGENT_MODEL":         {},
					"INVESTIGATORS_TYPE":  {"shared-name": "codex"},
					"INVESTIGATORS_MODEL": {},
				},
			},
			want:     "`shared-name` (claude)",
			testCase: "AGENT_TYPE takes precedence when both exist",
		},
		{
			agentName: "worker-1",
			cfg: Config{
				Associative: map[string]map[string]string{
					"AGENT_TYPE":          {"worker-1": "claude"},
					"AGENT_MODEL":         {"worker-1": "sonnet-5"},
					"INVESTIGATORS_TYPE":  {},
					"INVESTIGATORS_MODEL": {"worker-1": "gpt-6"},
				},
			},
			want:     "`worker-1` (claude, model `sonnet-5`)",
			testCase: "AGENT_MODEL takes precedence when both exist",
		},
	}

	for _, tt := range tests {
		t.Run(tt.testCase, func(t *testing.T) {
			got := FormatAgentDescriptor(tt.agentName, &tt.cfg)
			if got != tt.want {
				t.Errorf("FormatAgentDescriptor(%q) = %q, want %q", tt.agentName, got, tt.want)
			}
		})
	}
}

// TestFormatAgentDescriptorRegressionModelWithoutType verifies model lookup is independent of type.
// This regression case ensures both AGENT_MODEL and INVESTIGATORS_MODEL are checked regardless of type value.
func TestFormatAgentDescriptorRegressionModelWithoutType(t *testing.T) {
	// Even with "?" type, model lookup should still check both AGENT_MODEL and INVESTIGATORS_MODEL
	cfg := Config{
		Associative: map[string]map[string]string{
			"AGENT_TYPE":          {},
			"AGENT_MODEL":         {"unknown": "agent-model"},
			"INVESTIGATORS_TYPE":  {},
			"INVESTIGATORS_MODEL": {},
		},
	}
	got := FormatAgentDescriptor("unknown", &cfg)
	// Model is found in AGENT_MODEL even though type is "?"
	want := "`unknown` (?, model `agent-model`)"
	if got != want {
		t.Errorf("FormatAgentDescriptor with unknown type but model set = %q, want %q", got, want)
	}
}
