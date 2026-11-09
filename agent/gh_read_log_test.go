package main

import (
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestR11ReadLog preserves the Bash wrapper's append-only warning schema and string fields.
func TestR11ReadLog(t *testing.T) {
	root := t.TempDir()
	if err := os.Mkdir(filepath.Join(root, "logs"), 0o700); err != nil {
		t.Fatal(err)
	}
	for range 2 {
		if err := ghReadLog([]string{"AGENTS_HOME=" + root}, EventGitHubCommentIgnored, "ignored", map[string]string{"author": "outsider", "id": "17"}); err != nil {
			t.Fatal(err)
		}
	}
	raw, err := fs.ReadFile(os.DirFS(root), "logs/agent.jsonl")
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(strings.TrimSpace(string(raw)), "\n")
	if len(lines) != 2 {
		t.Fatalf("expected two appended records, got %q", raw)
	}
	for _, line := range lines {
		var record map[string]string // record checks string-valued Bash JSON compatibility.
		if err := json.Unmarshal([]byte(line), &record); err != nil {
			t.Fatal(err)
		}
		if record["component"] != "agent-gh" || record["level"] != "warn" || record["event"] != "github_comment_ignored" || record["author"] != "outsider" || record["id"] != "17" || record["message"] != "ignored" || record["ts"] == "" {
			t.Fatalf("wrong warning schema: %v", record)
		}
	}
}

// TestR11ReadLogMissingDirectory keeps logging failures visible without creating installation paths.
func TestR11ReadLogMissingDirectory(t *testing.T) {
	err := ghReadLog([]string{"HOME=" + t.TempDir()}, EventGitHubGraphQLRefused, "refused", nil)
	if err == nil || !errors.Is(err, ErrStorage) {
		t.Fatalf("expected storage failure, got %v", err)
	}
}
