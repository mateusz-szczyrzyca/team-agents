package main

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestFormatGitHubNameColors_InvestigatorPaletteReset(t *testing.T) {
	colorsPath := filepath.Join(t.TempDir(), "nonexistent-github-colors")
	palette := []string{"#0969DA", "#BC4C00"}
	cfg := newConfig()
	cfg.Indexed["NORMAL_AGENTS"] = map[string]string{"0": "agent-normal"}
	cfg.Associative["INVESTIGATORS_TYPE"] = map[string]string{"agent-inv": "claude"}

	got := FormatGitHubNameColors(colorsPath, palette, cfg)
	if !strings.Contains(got, "agent-normal=#0969DA") {
		t.Errorf("FormatGitHubNameColors() = %q, want containing agent-normal=#0969DA", got)
	}
	if !strings.Contains(got, "agent-inv=#0969DA") {
		t.Errorf("FormatGitHubNameColors() = %q, want containing agent-inv=#0969DA", got)
	}
}
