package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestStartupHOSTNAMEPrecedence compares configuration-visible hostnames with Bash.
func TestStartupHOSTNAMEPrecedence(t *testing.T) {
	binary := filepath.Join(t.TempDir(), "agent-go")
	var build bytes.Buffer
	result, err := (OSExecutor{}).Execute(context.Background(), Command{Name: "go", Args: []string{"build", "-o", binary, "."}, Stdout: &build, Stderr: &build})
	if err != nil || result.ExitCode != 0 {
		t.Fatalf("build: %v: %s", err, build.String())
	}
	for _, program := range []string{filepath.Join("..", "bin", "agent"), binary} {
		home := t.TempDir()
		writeConfigFixture(t, filepath.Join(home, "agents.conf"), "printf 'hostname=%s\\n' \"$HOSTNAME\"\n")
		var stdout, stderr bytes.Buffer
		result, err := (OSExecutor{}).Execute(context.Background(), Command{Name: program, Args: []string{"_agents-json"}, Env: []string{"HOME=" + home, "AGENTS_HOME=" + home, "HOSTNAME=invented.example", "PATH=" + os.Getenv("PATH")}, Stdout: &stdout, Stderr: &stderr})
		if err != nil || result.ExitCode != 0 || !strings.HasPrefix(stdout.String(), "hostname=invented.example\n") {
			t.Fatalf("%s: exit=%d error=%v stdout=%q stderr=%q", program, result.ExitCode, err, stdout.String(), stderr.String())
		}
	}
}
