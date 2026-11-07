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
// Oracle pattern: GIT_CONFIG_COUNT with indexed GIT_CONFIG_KEY_*/GIT_CONFIG_VALUE_*.
func TestIdentitySigningConfig(t *testing.T) {
	// Test case 1: No signing key provided - signing disabled
	t.Run("no signing key", func(t *testing.T) {
		id := &Identity{Agent: "test", SigningKey: ""}
		env := id.SigningEnv()
		if env == nil {
			t.Fatal("Expected non-nil env")
		}
		if env["GIT_CONFIG_COUNT"] != "2" {
			t.Errorf("Expected GIT_CONFIG_COUNT=2, got %q", env["GIT_CONFIG_COUNT"])
		}
		if env["GIT_CONFIG_KEY_0"] != "commit.gpgsign" || env["GIT_CONFIG_VALUE_0"] != "false" {
			t.Errorf("Expected commit.gpgsign=false, got KEY_0=%q VALUE_0=%q",
				env["GIT_CONFIG_KEY_0"], env["GIT_CONFIG_VALUE_0"])
		}
		if env["GIT_CONFIG_KEY_1"] != "tag.gpgsign" || env["GIT_CONFIG_VALUE_1"] != "false" {
			t.Errorf("Expected tag.gpgsign=false, got KEY_1=%q VALUE_1=%q",
				env["GIT_CONFIG_KEY_1"], env["GIT_CONFIG_VALUE_1"])
		}
	})

	// Test case 2: Signing key file missing - signing disabled with warning
	t.Run("missing signing key file", func(t *testing.T) {
		id := &Identity{Agent: "test", SigningKey: "/nonexistent/key.pub"}
		env := id.SigningEnv()
		if env == nil {
			t.Fatal("Expected non-nil env")
		}
		if env["GIT_CONFIG_COUNT"] != "2" {
			t.Errorf("Expected GIT_CONFIG_COUNT=2, got %q", env["GIT_CONFIG_COUNT"])
		}
		if env["GIT_CONFIG_VALUE_0"] != "false" {
			t.Errorf("Expected signing disabled when key doesn't exist, got %q", env["GIT_CONFIG_VALUE_0"])
		}
	})

	// Test case 3: Signing key exists - signing enabled with full config
	t.Run("signing key exists", func(t *testing.T) {
		tmpfile, err := os.CreateTemp("", "test-key-*")
		if err != nil {
			t.Fatalf("Failed to create temp key file: %v", err)
		}
		defer func() {
			if err := os.Remove(tmpfile.Name()); err != nil {
				t.Logf("cleanup: could not remove temp file: %v", err)
			}
		}()
		if err := tmpfile.Close(); err != nil {
			t.Logf("cleanup: could not close temp file: %v", err)
		}

		id := &Identity{Agent: "test", SigningKey: tmpfile.Name()}
		env := id.SigningEnv()
		if env == nil {
			t.Fatal("Expected non-nil env")
		}
		if env["GIT_CONFIG_COUNT"] != "4" {
			t.Errorf("Expected GIT_CONFIG_COUNT=4 when signing, got %q", env["GIT_CONFIG_COUNT"])
		}
		if env["GIT_CONFIG_KEY_0"] != "gpg.format" || env["GIT_CONFIG_VALUE_0"] != "ssh" {
			t.Errorf("Expected gpg.format=ssh, got KEY_0=%q VALUE_0=%q",
				env["GIT_CONFIG_KEY_0"], env["GIT_CONFIG_VALUE_0"])
		}
		if env["GIT_CONFIG_KEY_1"] != "user.signingkey" || env["GIT_CONFIG_VALUE_1"] != tmpfile.Name() {
			t.Errorf("Expected user.signingkey=%q, got VALUE_1=%q", tmpfile.Name(), env["GIT_CONFIG_VALUE_1"])
		}
		if env["GIT_CONFIG_KEY_2"] != "commit.gpgsign" || env["GIT_CONFIG_VALUE_2"] != "true" {
			t.Errorf("Expected commit.gpgsign=true")
		}
		if env["GIT_CONFIG_KEY_3"] != "tag.gpgsign" || env["GIT_CONFIG_VALUE_3"] != "true" {
			t.Errorf("Expected tag.gpgsign=true")
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

	// Check signing is disabled by default (no key) - oracle uses GIT_CONFIG_COUNT pattern
	if env["GIT_CONFIG_COUNT"] != "2" {
		t.Errorf("GIT_CONFIG_COUNT = %q, want %q (signing disabled)", env["GIT_CONFIG_COUNT"], "2")
	}
	if env["GIT_CONFIG_KEY_0"] != "commit.gpgsign" || env["GIT_CONFIG_VALUE_0"] != "false" {
		t.Errorf("Expected commit.gpgsign=false in config")
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
