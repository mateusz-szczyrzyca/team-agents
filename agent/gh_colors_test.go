package main

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestRegisterGitHubColors(t *testing.T) {
	tmp := t.TempDir()
	colorsPath := filepath.Join(tmp, "github-colors")
	palette := []string{"#0969DA", "#BC4C00", "#8250DF"}

	names1 := []string{"claude-haiku", "codex-1"}
	err := RegisterGitHubColors(context.Background(), colorsPath, palette, names1, nil)
	if err != nil {
		t.Fatalf("RegisterGitHubColors() failed: %v", err)
	}

	data, errRead := os.ReadFile(filepath.Clean(colorsPath))
	if errRead != nil {
		t.Fatalf("ReadFile() failed: %v", errRead)
	}
	content := string(data)
	if !strings.Contains(content, "claude-haiku #0969DA\n") {
		t.Errorf("expected claude-haiku #0969DA in %q", content)
	}
	if !strings.Contains(content, "codex-1 #BC4C00\n") {
		t.Errorf("expected codex-1 #BC4C00 in %q", content)
	}

	// Register another name; should pick the least-used color (#8250DF)
	names2 := []string{"gemini-1"}
	if err := RegisterGitHubColors(context.Background(), colorsPath, palette, names2, nil); err != nil {
		t.Fatalf("RegisterGitHubColors() failed: %v", err)
	}
	data2, errRead2 := os.ReadFile(filepath.Clean(colorsPath))
	if errRead2 != nil {
		t.Fatalf("second ReadFile() failed: %v", errRead2)
	}
	if !strings.Contains(string(data2), "gemini-1 #8250DF\n") {
		t.Errorf("expected gemini-1 #8250DF in %q", string(data2))
	}
}

func TestFormatGitHubNameColors(t *testing.T) {
	tmp := t.TempDir()
	colorsPath := filepath.Join(tmp, "github-colors")
	palette := []string{"#0969DA", "#BC4C00"}
	if err := os.WriteFile(colorsPath, []byte("agent-a #0969DA\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	cfg := newConfig()
	got := FormatGitHubNameColors(colorsPath, palette, cfg)
	if !strings.Contains(got, "agent-a=#0969DA") {
		t.Errorf("FormatGitHubNameColors() = %q, want containing agent-a=#0969DA", got)
	}
}

func TestLegacyGitHubColorsLoads(t *testing.T) {
	fixturePath := filepath.Join("testdata", "github-colors-legacy")
	palette := []string{"#0969DA", "#BC4C00", "#8250DF"}
	cfg := newConfig()

	got := FormatGitHubNameColors(fixturePath, palette, cfg)
	if !strings.Contains(got, "claude-haiku=#0969DA") {
		t.Errorf("FormatGitHubNameColors() missing claude-haiku: %q", got)
	}
	if !strings.Contains(got, "codex-1=#BC4C00") {
		t.Errorf("FormatGitHubNameColors() missing codex-1: %q", got)
	}
	if !strings.Contains(got, "gemini-1=#8250DF") {
		t.Errorf("FormatGitHubNameColors() missing gemini-1: %q", got)
	}
}
