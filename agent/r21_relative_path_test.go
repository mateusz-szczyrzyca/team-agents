package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestConflictingParentPathRelativeChild verifies that child PATH=. and PATH=:
// select the executable in the process CWD rather than a conflicting parent PATH executable.
func TestConflictingParentPathRelativeChild(t *testing.T) {
	cwdDir := t.TempDir()
	parentDir := t.TempDir()

	writeTestScript(t, filepath.Join(cwdDir, "mycmd"), "#!/bin/sh\necho child\n", 0o755)
	writeTestScript(t, filepath.Join(parentDir, "mycmd"), "#!/bin/sh\necho parent\n", 0o755)

	origWd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Chdir(cwdDir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := os.Chdir(origWd); err != nil {
			t.Errorf("restore wd: %v", err)
		}
	})

	t.Setenv("PATH", parentDir)

	tests := []struct {
		name     string
		childEnv []string
	}{
		{
			name:     "dot",
			childEnv: []string{"PATH=."},
		},
		{
			name:     "empty_component",
			childEnv: []string{"PATH=:"},
		},
	}

	exec := OSExecutor{}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			var out bytes.Buffer
			cmd := Command{
				Name:   "mycmd",
				Env:    tc.childEnv,
				Stdout: &out,
			}
			res, execErr := exec.Execute(context.Background(), cmd)
			if execErr != nil {
				t.Fatalf("unexpected execute error: %v", execErr)
			}
			if res.ExitCode != 0 {
				t.Fatalf("unexpected exit code: %d", res.ExitCode)
			}
			got := strings.TrimSpace(out.String())
			if got != "child" {
				t.Errorf("expected child execution, got %q", got)
			}
		})
	}
}
