package main

import (
	"bytes"
	"os"
	"testing"
)

// TestIdentityAuthorFormat tests that identity correctly formats author name with %a substitution.
func TestIdentityAuthorFormat(t *testing.T) {
	cases := []struct {
		agent       string
		format      string
		want        string
		wantErr     bool
		description string
	}{
		{
			agent:       "claude-haiku",
			format:      "%a (team-agents)",
			want:        "claude-haiku (team-agents)",
			wantErr:     false,
			description: "basic %a substitution",
		},
		{
			agent:       "my-agent",
			format:      "%a",
			want:        "my-agent",
			wantErr:     false,
			description: "%a without suffix",
		},
		{
			agent:       "test-bot",
			format:      "Agent: %a",
			want:        "Agent: test-bot",
			wantErr:     false,
			description: "%a with prefix",
		},
	}

	for _, tc := range cases {
		t.Run(tc.description, func(t *testing.T) {
			id := &Identity{Agent: tc.agent}
			got := id.FormatAuthor(tc.format)
			if tc.wantErr && got == "" {
				// expected error case (no error handling yet in stub)
				return
			}
			if got != tc.want {
				t.Errorf("FormatAuthor(%q) = %q, want %q", tc.format, got, tc.want)
			}
		})
	}
}

// TestIdentityEmail tests email generation with proper defaults and %a substitution.
func TestIdentityEmail(t *testing.T) {
	cases := []struct {
		agent       string
		emailConfig string
		want        string
		description string
	}{
		{
			agent:       "claude-haiku",
			emailConfig: "",
			want:        "claude-haiku@team-agents.invalid",
			description: "default .invalid address when empty",
		},
		{
			agent:       "my-bot",
			emailConfig: "%a@example.com",
			want:        "my-bot@example.com",
			description: "%a substitution in email config",
		},
		{
			agent:       "bot123",
			emailConfig: "agent@test.invalid",
			want:        "agent@test.invalid",
			description: "fixed email without %a",
		},
	}

	for _, tc := range cases {
		t.Run(tc.description, func(t *testing.T) {
			id := &Identity{Agent: tc.agent, EmailConfig: tc.emailConfig}
			got := id.Email()
			if got != tc.want {
				t.Errorf("Email() with agent=%q, config=%q = %q, want %q", tc.agent, tc.emailConfig, got, tc.want)
			}
		})
	}
}

// TestIdentitySigningConfig tests signing environment variables based on key availability.
func TestIdentitySigningConfig(t *testing.T) {
	// Test case 1: No signing key provided
	t.Run("no signing key", func(t *testing.T) {
		id := &Identity{Agent: "test", SigningKey: ""}
		env := id.SigningEnv()
		if env == nil || env["GIT_COMMIT_GPGSIGN"] != "false" {
			t.Error("Expected signing disabled when no key provided")
		}
	})

	// Test case 2: Signing key file missing (should disable signing)
	t.Run("missing signing key file", func(t *testing.T) {
		id := &Identity{Agent: "test", SigningKey: "/nonexistent/key.pub"}
		env := id.SigningEnv()
		if env == nil || env["GIT_COMMIT_GPGSIGN"] != "false" {
			t.Error("Expected signing disabled when key file doesn't exist")
		}
	})

	// Test case 3: Signing key exists
	t.Run("signing key exists", func(t *testing.T) {
		tmpfile, err := os.CreateTemp("", "test-key-*")
		if err != nil {
			t.Fatalf("Failed to create temp key file: %v", err)
		}
		defer func() {
			_ = os.Remove(tmpfile.Name())
		}()
		_ = tmpfile.Close()

		id := &Identity{Agent: "test", SigningKey: tmpfile.Name()}
		env := id.SigningEnv()
		if env == nil {
			t.Error("Expected non-nil env when signing key exists")
		}
		if env["GIT_COMMIT_GPGSIGN"] != "true" {
			t.Error("Expected signing enabled when key exists")
		}
		if env["GIT_GPG_FORMAT"] != "ssh" {
			t.Error("Expected GPG format to be ssh")
		}
	})
}

// TestCommitTrailers tests trailer formatting with required and optional fields.
func TestCommitTrailers(t *testing.T) {
	cases := []struct {
		id          *Identity
		want        []string
		description string
	}{
		{
			id: &Identity{
				Agent:    "claude-haiku",
				Provider: "claude",
				Model:    "haiku",
				Task:     "task-123",
				Team:     "",
				InvID:    "",
			},
			want: []string{
				"Agent: claude-haiku",
				"Agent-Model: claude/haiku",
				"Task: task-123",
			},
			description: "required trailers only",
		},
		{
			id: &Identity{
				Agent:    "codex-1",
				Provider: "codex",
				Model:    "gpt-6.1-sol",
				Task:     "task-456",
				Team:     "team-001",
				InvID:    "123",
			},
			want: []string{
				"Agent: codex-1",
				"Agent-Model: codex/gpt-6.1-sol",
				"Task: task-456",
				"Team: team-001",
				"Investigation: #123",
			},
			description: "with team and investigation",
		},
	}

	for _, tc := range cases {
		t.Run(tc.description, func(t *testing.T) {
			got := tc.id.Trailers()
			if len(got) != len(tc.want) {
				t.Errorf("Trailers() returned %d lines, want %d", len(got), len(tc.want))
				t.Logf("Got: %v", got)
				t.Logf("Want: %v", tc.want)
				return
			}
			for i, line := range got {
				if line != tc.want[i] {
					t.Errorf("Trailer %d: got %q, want %q", i, line, tc.want[i])
				}
			}
		})
	}
}

// TestIdentityEnvironment tests complete environment setup for git operations.
func TestIdentityEnvironment(t *testing.T) {
	id := &Identity{
		Agent:       "test-agent",
		Provider:    "claude",
		Model:       "haiku",
		EmailConfig: "",
		SigningKey:  "",
		Task:        "task-123",
		Team:        "team-01",
	}

	env := id.Environment()

	if env == nil {
		t.Fatal("Environment() returned nil")
	}

	// Check required fields
	if env["GIT_AUTHOR_NAME"] != "test-agent (team-agents)" {
		t.Errorf("GIT_AUTHOR_NAME = %q, want %q", env["GIT_AUTHOR_NAME"], "test-agent (team-agents)")
	}
	if env["GIT_COMMITTER_NAME"] != "test-agent (team-agents)" {
		t.Errorf("GIT_COMMITTER_NAME = %q, want %q", env["GIT_COMMITTER_NAME"], "test-agent (team-agents)")
	}
	if env["GIT_AUTHOR_EMAIL"] != "test-agent@team-agents.invalid" {
		t.Errorf("GIT_AUTHOR_EMAIL = %q, want %q", env["GIT_AUTHOR_EMAIL"], "test-agent@team-agents.invalid")
	}
	if env["GIT_COMMITTER_EMAIL"] != "test-agent@team-agents.invalid" {
		t.Errorf("GIT_COMMITTER_EMAIL = %q, want %q", env["GIT_COMMITTER_EMAIL"], "test-agent@team-agents.invalid")
	}

	// Check signing is disabled by default (no key)
	if env["GIT_COMMIT_GPGSIGN"] != "false" {
		t.Errorf("GIT_COMMIT_GPGSIGN = %q, want %q (signing disabled)", env["GIT_COMMIT_GPGSIGN"], "false")
	}
}

// TestCommitWithTrailers tests that trailers can be formatted as git message suffix.
func TestCommitWithTrailers(t *testing.T) {
	id := &Identity{
		Agent: "bot",
		Model: "test",
		Task:  "id-001",
		Team:  "team-x",
		InvID: "",
	}

	// Test formatting message with trailers
	trailers := id.Trailers()
	if len(trailers) < 3 {
		t.Fatalf("Expected at least 3 trailers, got %d", len(trailers))
	}

	// Verify trailer format (lines with ": ")
	for _, trailer := range trailers {
		if !bytes.Contains([]byte(trailer), []byte(": ")) {
			t.Errorf("Trailer %q does not contain ': '", trailer)
		}
	}
}
