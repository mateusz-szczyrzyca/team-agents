package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"testing"
)

// TestR10GHBoundary keeps unfinished gh policy fail-closed without intercepting ordinary commands.
func TestR10GHBoundary(t *testing.T) {
	for _, name := range []string{"gh", "/tmp/tools/gh"} {
		if !ghFastPath(name) {
			t.Fatalf("ghFastPath(%q) did not select the native wrapper", name)
		}
	}
	for _, name := range []string{"agent", "/tmp/gh/agent", "gh-extra"} {
		if ghFastPath(name) {
			t.Fatalf("ordinary executable %q intercepted", name)
		}
	}
}

// TestR10GHBeforeConfiguration proves the real executable refuses gh before executing any configuration.
func TestR10GHBeforeConfiguration(t *testing.T) {
	root := t.TempDir()
	binary := filepath.Join(root, "agent-go")
	var build bytes.Buffer // build captures compiler diagnostics without accessing the live installation.
	result, buildErr := (OSExecutor{}).Execute(context.Background(), Command{Name: "go", Args: []string{"build", "-o", binary, "."}, Stdout: &build, Stderr: &build})
	if buildErr != nil || result.ExitCode != 0 {
		t.Fatalf("build candidate: %v: %s", buildErr, build.String())
	}
	alias := filepath.Join(root, "gh")
	if err := os.Symlink(binary, alias); err != nil {
		t.Fatal(err)
	}
	marker := filepath.Join(root, "config-ran")
	realGH := filepath.Join(root, "real-gh")
	if err := os.WriteFile(realGH, []byte("#!/bin/sh\nprintf called > \"$HOME/gh-ran\"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(realGH, 0o700); err != nil { // #nosec G302 -- the isolated fixture must execute to detect forbidden forwarding.
		t.Fatal(err)
	}
	config := "printf 'loaded' > \"$HOME/config-ran\"\nexit 73\n"
	if err := os.WriteFile(filepath.Join(root, "agents.conf"), []byte(config), 0o600); err != nil {
		t.Fatal(err)
	}
	for _, executable := range []string{alias, binary} {
		var output bytes.Buffer // output retains startup diagnostics from the isolated candidate process.
		result, err := (OSExecutor{}).Execute(context.Background(), Command{Name: executable, Args: []string{"--help"}, Env: []string{"HOME=" + root, "AGENTS_HOME=" + root, "PATH=" + os.Getenv("PATH"), "AGENT_REAL_GH=" + realGH}, Stdout: &output, Stderr: &output})
		if err != nil {
			t.Fatalf("%s: execution failed: %v: %s", executable, err, output.String())
		}
		_, statErr := os.Stat(marker)
		if executable == alias {
			if result.ExitCode != 0 {
				t.Fatalf("gh exit = %d; output: %s", result.ExitCode, output.String())
			}
			if !os.IsNotExist(statErr) {
				t.Fatalf("gh evaluated configuration: %v", statErr)
			}
			if _, realErr := os.Stat(filepath.Join(root, "gh-ran")); realErr != nil {
				t.Fatalf("native gh help did not reach the fixture: %v", realErr)
			}
		} else if statErr != nil || result.ExitCode == 0 {
			t.Fatalf("ordinary agent did not evaluate configuration: %v; output: %s", statErr, output.String())
		}
	}
}
