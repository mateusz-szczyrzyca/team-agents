package main

import (
	"strings"
	"testing"
)

func TestIdentityFromRunBasicMapping(t *testing.T) {
	// RED: basic mapping from metadata and config
	meta := MetaFile{
		Lines: []MetaLine{
			{Kind: MetaAssignment, Key: "AGENT", Value: "claude-haiku"},
			{Kind: MetaAssignment, Key: "TYPE", Value: "claude"},
			{Kind: MetaAssignment, Key: "MODEL", Value: "haiku"},
			{Kind: MetaAssignment, Key: "TEAM", Value: "20261107-test-team"},
			{Kind: MetaAssignment, Key: "INV_ID", Value: ""},
		},
	}
	cfg := Config{
		Scalars: map[string]string{
			"GIT_NAME_FORMAT":  "%a (team-agents)",
			"GIT_EMAIL":        "%a@team-agents.invalid",
			"GIT_SIGNING_KEY":  "",
		},
	}
	runnerID := "20261107-test-run"

	id := IdentityFromRun(meta, cfg, runnerID)

	if id == nil {
		t.Fatal("IdentityFromRun returned nil")
	}
	if id.Agent != "claude-haiku" {
		t.Errorf("Agent: got %q, want %q", id.Agent, "claude-haiku")
	}
	if id.Provider != "claude" {
		t.Errorf("Provider: got %q, want %q", id.Provider, "claude")
	}
	if id.Model != "haiku" {
		t.Errorf("Model: got %q, want %q", id.Model, "haiku")
	}
	if id.Team != "20261107-test-team" {
		t.Errorf("Team: got %q, want %q", id.Team, "20261107-test-team")
	}
	if id.Task != runnerID {
		t.Errorf("Task: got %q, want %q", id.Task, runnerID)
	}
}

func TestIdentityFromRunEmptyConfig(t *testing.T) {
	// RED: empty config uses defaults
	meta := MetaFile{
		Lines: []MetaLine{
			{Kind: MetaAssignment, Key: "AGENT", Value: "codex-1"},
			{Kind: MetaAssignment, Key: "TYPE", Value: "codex"},
			{Kind: MetaAssignment, Key: "MODEL", Value: "gpt-6.1-sol"},
		},
	}
	cfg := Config{
		Scalars: map[string]string{},
	}
	runnerID := "test-run-2"

	id := IdentityFromRun(meta, cfg, runnerID)

	if id == nil {
		t.Fatal("IdentityFromRun returned nil")
	}
	if id.Agent != "codex-1" {
		t.Errorf("Agent: got %q, want %q", id.Agent, "codex-1")
	}
	if id.EmailConfig != "" {
		t.Errorf("EmailConfig should preserve empty string, got %q", id.EmailConfig)
	}
}

func TestIdentityFromRunEmailExpansion(t *testing.T) {
	// RED: %a in email config is preserved, not expanded
	meta := MetaFile{
		Lines: []MetaLine{
			{Kind: MetaAssignment, Key: "AGENT", Value: "test-agent"},
			{Kind: MetaAssignment, Key: "TYPE", Value: "test"},
			{Kind: MetaAssignment, Key: "MODEL", Value: "v1"},
		},
	}
	cfg := Config{
		Scalars: map[string]string{
			"GIT_EMAIL": "agent-%a@example.com",
		},
	}

	id := IdentityFromRun(meta, cfg, "run1")

	if id == nil {
		t.Fatal("IdentityFromRun returned nil")
	}
	// EmailConfig should be exactly what's in config, not expanded
	if id.EmailConfig != "agent-%a@example.com" {
		t.Errorf("EmailConfig: got %q, want %q", id.EmailConfig, "agent-%a@example.com")
	}
}

func TestIdentityFromRunOptionalFields(t *testing.T) {
	// RED: INV_ID and TEAM optional
	meta := MetaFile{
		Lines: []MetaLine{
			{Kind: MetaAssignment, Key: "AGENT", Value: "solo-agent"},
			{Kind: MetaAssignment, Key: "TYPE", Value: "claude"},
			{Kind: MetaAssignment, Key: "MODEL", Value: ""},
		},
	}
	cfg := Config{
		Scalars: map[string]string{},
	}

	id := IdentityFromRun(meta, cfg, "solo-run")

	if id == nil {
		t.Fatal("IdentityFromRun returned nil")
	}
	if id.Team != "" {
		t.Errorf("Team should be empty when not in metadata, got %q", id.Team)
	}
	if id.InvID != "" {
		t.Errorf("InvID should be empty when not in metadata, got %q", id.InvID)
	}
	if id.Model != "" {
		t.Errorf("Model should be empty when empty in metadata, got %q", id.Model)
	}
}

func TestIdentityFromRunSigningKeyPreserved(t *testing.T) {
	// RED: signing key path is preserved
	meta := MetaFile{
		Lines: []MetaLine{
			{Kind: MetaAssignment, Key: "AGENT", Value: "signer"},
			{Kind: MetaAssignment, Key: "TYPE", Value: "claude"},
			{Kind: MetaAssignment, Key: "MODEL", Value: "test"},
		},
	}
	cfg := Config{
		Scalars: map[string]string{
			"GIT_SIGNING_KEY": "/home/user/.ssh/id_ed25519",
		},
	}

	id := IdentityFromRun(meta, cfg, "sign-run")

	if id == nil {
		t.Fatal("IdentityFromRun returned nil")
	}
	if id.SigningKey != "/home/user/.ssh/id_ed25519" {
		t.Errorf("SigningKey: got %q, want %q", id.SigningKey, "/home/user/.ssh/id_ed25519")
	}
}

func TestIdentityFromRunTrailersFormat(t *testing.T) {
	// RED: trailers format matches oracle
	meta := MetaFile{
		Lines: []MetaLine{
			{Kind: MetaAssignment, Key: "AGENT", Value: "haiku-agent"},
			{Kind: MetaAssignment, Key: "TYPE", Value: "claude"},
			{Kind: MetaAssignment, Key: "MODEL", Value: "haiku"},
			{Kind: MetaAssignment, Key: "TEAM", Value: "team-1"},
		},
	}
	cfg := Config{
		Scalars: map[string]string{},
	}

	id := IdentityFromRun(meta, cfg, "task-123")

	if id == nil {
		t.Fatal("IdentityFromRun returned nil")
	}

	trailers := id.Trailers()
	expected := []string{
		"Agent: haiku-agent",
		"Agent-Model: claude/haiku",
		"Task: task-123",
		"Team: team-1",
	}

	if len(trailers) != len(expected) {
		t.Fatalf("trailers count: got %d, want %d", len(trailers), len(expected))
	}
	for i, exp := range expected {
		if trailers[i] != exp {
			t.Errorf("trailer %d: got %q, want %q", i, trailers[i], exp)
		}
	}
}

func TestIdentityFromRunInvestigationTrailer(t *testing.T) {
	// RED: investigation trailer present when INV_ID set
	meta := MetaFile{
		Lines: []MetaLine{
			{Kind: MetaAssignment, Key: "AGENT", Value: "inv-agent"},
			{Kind: MetaAssignment, Key: "TYPE", Value: "claude"},
			{Kind: MetaAssignment, Key: "MODEL", Value: "test"},
			{Kind: MetaAssignment, Key: "INV_ID", Value: "42"},
		},
	}
	cfg := Config{
		Scalars: map[string]string{},
	}

	id := IdentityFromRun(meta, cfg, "task-inv")

	if id == nil {
		t.Fatal("IdentityFromRun returned nil")
	}

	trailers := id.Trailers()
	found := false
	for _, tr := range trailers {
		if tr == "Investigation: #42" {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("Investigation trailer not found. Got: %v", trailers)
	}
}

func TestIdentityFromRunAuthorFormatExpansion(t *testing.T) {
	// RED: Identity values are extracted and preserved correctly
	meta := MetaFile{
		Lines: []MetaLine{
			{Kind: MetaAssignment, Key: "AGENT", Value: "my-agent"},
			{Kind: MetaAssignment, Key: "TYPE", Value: "test"},
			{Kind: MetaAssignment, Key: "MODEL", Value: ""},
		},
	}
	cfg := Config{
		Scalars: map[string]string{
			"GIT_NAME_FORMAT": "%a (team-agents)",
		},
	}

	id := IdentityFromRun(meta, cfg, "run")

	if id == nil {
		t.Fatal("IdentityFromRun returned nil")
	}

	// FormatAuthor expands %a when given the format
	author := id.FormatAuthor("%a (team-agents)")
	if !strings.Contains(author, "my-agent") {
		t.Errorf("FormatAuthor should expand %%a to agent name, got %q", author)
	}
}
