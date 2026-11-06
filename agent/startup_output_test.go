package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestStartupBinaryDiagnostics exercises the CLI streams of the actual executable.
func TestStartupBinaryDiagnostics(t *testing.T) {
	binary := filepath.Join(t.TempDir(), "agent-go")
	var build bytes.Buffer
	result, err := (OSExecutor{}).Execute(context.Background(), Command{Name: "go", Args: []string{"build", "-o", binary, "."}, Stdout: &build, Stderr: &build})
	if err != nil || result.ExitCode != 0 {
		t.Fatalf("build: %v: %s", err, build.String())
	}
	for _, missing := range []bool{false, true} {
		t.Run(map[bool]string{false: "configuration streams", true: "missing configuration"}[missing], func(t *testing.T) {
			home := t.TempDir()
			if !missing {
				writeConfigFixture(t, filepath.Join(home, "agents.conf"), "printf 'config stdout marker\\n'\nprintf 'config stderr marker\\n' >&2\n")
			}
			var stdout, stderr bytes.Buffer
			result, err := (OSExecutor{}).Execute(context.Background(), Command{Name: binary, Args: []string{"status"}, Env: []string{"HOME=" + home, "AGENTS_HOME=" + home, "PATH=" + os.Getenv("PATH")}, Stdout: &stdout, Stderr: &stderr})
			if err != nil {
				t.Fatal(err)
			}
			if missing {
				if result.ExitCode != 1 || !strings.Contains(stderr.String(), "agent: ") || stdout.Len() != 0 {
					t.Fatalf("missing startup diagnostic: exit=%d stdout=%q stderr=%q", result.ExitCode, stdout.String(), stderr.String())
				}
				return
			}
			if result.ExitCode != 0 || !strings.HasPrefix(stdout.String(), "config stdout marker\n") || stderr.String() != "config stderr marker\n" || strings.ContainsRune(stdout.String(), '\x00') {
				t.Fatalf("configuration streams lost or snapshot leaked: exit=%d stdout=%q stderr=%q", result.ExitCode, stdout.String(), stderr.String())
			}
		})
	}
}
