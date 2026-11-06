package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"testing"
)

// TestRound2ProductionJSON loads configuration through the compiled entry point.
func TestRound2ProductionJSON(t *testing.T) {
	binary := filepath.Join(t.TempDir(), "agent-go")
	var buildOutput bytes.Buffer
	result, buildError := (OSExecutor{}).Execute(context.Background(), Command{Name: "go", Args: []string{"build", "-o", binary, "."}, Stdout: &buildOutput, Stderr: &buildOutput})
	if buildError != nil || result.ExitCode != 0 {
		t.Fatalf("build: %v: %s", buildError, buildOutput.String())
	}
	for _, customHome := range []bool{false, true} {
		t.Run(map[bool]string{false: "HOME", true: "AGENTS_HOME"}[customHome], func(t *testing.T) {
			home := t.TempDir()
			agentsHome := filepath.Join(home, "agents")
			if customHome {
				agentsHome = filepath.Join(home, "custom")
			}
			if err := os.MkdirAll(agentsHome, 0o700); err != nil {
				t.Fatal(err)
			}
			writeConfigFixture(t, filepath.Join(agentsHome, "agents.conf"), `declare -A AGENT_TYPE=([fixture]=codex)
declare -A AGENT_MODEL=([fixture]=example-model)
declare -A INVESTIGATORS_TYPE=([reviewer]=claude)
declare -A INVESTIGATORS_MODEL=([reviewer]=review-model)
`)
			var output, diagnostics bytes.Buffer
			command := Command{Name: binary, Args: []string{"_agents-json"}, Env: []string{"HOME=" + home, "PATH=" + os.Getenv("PATH")}, Stdout: &output, Stderr: &diagnostics}
			if customHome {
				command.Env = append(command.Env, "AGENTS_HOME="+agentsHome)
			}
			result, executionError := (OSExecutor{}).Execute(context.Background(), command)
			if executionError != nil || result.ExitCode != 0 {
				t.Fatalf("JSON listing: %v (exit %d): %s", executionError, result.ExitCode, diagnostics.String())
			}
			const expected = "{\"name\":\"fixture\",\"type\":\"codex\",\"model\":\"example-model\",\"tier\":\"normal\",\"provider\":\"openai\"}\n" +
				"{\"name\":\"reviewer\",\"type\":\"claude\",\"model\":\"review-model\",\"tier\":\"investigator\",\"provider\":\"\"}\n"
			if output.String() != expected || diagnostics.Len() != 0 {
				t.Fatalf("JSON output=%q, stderr=%q; want %q and empty stderr", output.String(), diagnostics.String(), expected)
			}
		})
	}
}
