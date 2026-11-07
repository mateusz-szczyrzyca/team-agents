package main

import (
	"context"
	"testing"
)

func TestIdentityFromRunExtractsNameFormat(t *testing.T) {
	// RED: IdentityFromRun should extract GIT_NAME_FORMAT from config
	meta := MetaFile{
		Lines: []MetaLine{
			{Kind: MetaAssignment, Key: "AGENT", Value: "claude-haiku"},
			{Kind: MetaAssignment, Key: "TYPE", Value: "claude"},
		},
	}
	cfg := Config{
		Scalars: map[string]string{
			"GIT_NAME_FORMAT": "%a (team-agents)",
		},
	}

	id := IdentityFromRun(meta, cfg, "task-123")

	if id.NameFormat != "%a (team-agents)" {
		t.Errorf("NameFormat: got %q, want %q", id.NameFormat, "%a (team-agents)")
	}
}

func TestIdentityFromRunEmptyNameFormat(t *testing.T) {
	// RED: IdentityFromRun should preserve empty NameFormat
	meta := MetaFile{
		Lines: []MetaLine{
			{Kind: MetaAssignment, Key: "AGENT", Value: "codex-1"},
		},
	}
	cfg := Config{
		Scalars: map[string]string{},
	}

	id := IdentityFromRun(meta, cfg, "task-456")

	if id.NameFormat != "" {
		t.Errorf("NameFormat: got %q, want %q", id.NameFormat, "")
	}
}

func TestSetupUsesIdentityNameFormat(t *testing.T) {
	// RED: Setup should use NameFormat field when provided
	id := &Identity{
		Agent:       "claude-haiku",
		NameFormat:  "%a: custom format",
		EmailConfig: "%a@team-agents.invalid",
	}

	env, err := id.Setup(context.Background(), "", nil)
	if err != nil {
		t.Fatalf("Setup failed: %v", err)
	}

	expectedName := "claude-haiku: custom format"
	if env["GIT_AUTHOR_NAME"] != expectedName {
		t.Errorf("GIT_AUTHOR_NAME: got %q, want %q", env["GIT_AUTHOR_NAME"], expectedName)
	}
	if env["GIT_COMMITTER_NAME"] != expectedName {
		t.Errorf("GIT_COMMITTER_NAME: got %q, want %q", env["GIT_COMMITTER_NAME"], expectedName)
	}
}

func TestSetupNameFormatPriority(t *testing.T) {
	// RED: When Setup receives both format param and NameFormat field,
	// format param should take precedence (like current Setup behavior)
	id := &Identity{
		Agent:       "gemini-1",
		NameFormat:  "%a (field format)",
		EmailConfig: "%a@team-agents.invalid",
	}

	env, err := id.Setup(context.Background(), "%a (param format)", nil)
	if err != nil {
		t.Fatalf("Setup failed: %v", err)
	}

	// Setup parameter takes precedence
	expectedName := "gemini-1 (param format)"
	if env["GIT_AUTHOR_NAME"] != expectedName {
		t.Errorf("GIT_AUTHOR_NAME: got %q, want %q", env["GIT_AUTHOR_NAME"], expectedName)
	}
}

func TestSetupDefaultsNameFormatWhenEmpty(t *testing.T) {
	// RED: Setup should use default format when NameFormat field is empty
	id := &Identity{
		Agent:       "codex-1",
		NameFormat:  "",
		EmailConfig: "%a@team-agents.invalid",
	}

	env, err := id.Setup(context.Background(), "", nil)
	if err != nil {
		t.Fatalf("Setup failed: %v", err)
	}

	expectedName := "codex-1 (team-agents)"
	if env["GIT_AUTHOR_NAME"] != expectedName {
		t.Errorf("GIT_AUTHOR_NAME: got %q, want %q", env["GIT_AUTHOR_NAME"], expectedName)
	}
}
