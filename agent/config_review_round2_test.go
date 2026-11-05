package main

import (
	"context"
	"os"
	"path/filepath"
	"syscall"
	"testing"
	"time"
)

// configOldShell simulates the unsupported shopt result on Bash 5.0/5.1.
type configOldShell struct{}

func (configOldShell) Execute(ctx context.Context, cmd Command) (CommandResult, *ExecuteError) {
	cmd.Args[3] = `shopt() { return 1; }; ` + cmd.Args[3]
	return (OSExecutor{}).Execute(ctx, cmd)
}

func TestConfigReviewRound2Values(t *testing.T) {
	for _, tc := range []struct {
		script, want string
		args         []string
		executor     Executor
	}{
		{`values=(Polish); declare -n AGENT_LANGUAGE='values[0]'`, "Polish", nil, OSExecutor{}},
		{`declare -A values=([lang]=Polish); declare -n AGENT_LANGUAGE='values[lang]'`, "Polish", nil, OSExecutor{}},
		{`AGENT_LANGUAGE="${1-unset}:$#:$AGENT_SUBCOMMAND:${RUNNER_ID+x}:$RUNNER_ID"`, "unset:0::x:", nil, OSExecutor{}},
		{`AGENT_LANGUAGE="$1:$2:$#:$AGENT_SUBCOMMAND:${RUNNER_ID+x}:$RUNNER_ID"`, "run:two words:2:run:x:", []string{"run", "two words"}, OSExecutor{}},
		{`AGENT_LANGUAGE="$1:$AGENT_SUBCOMMAND:${RUNNER_ID+x}:$RUNNER_ID"`, "_runner:_runner:x:", []string{"_runner"}, OSExecutor{}},
		{`AGENT_LANGUAGE=Polish`, "Polish", nil, configOldShell{}},
	} {
		t.Run(tc.want+tc.script, func(t *testing.T) {
			file := filepath.Join(t.TempDir(), "config")
			writeContextFixture(t, file, tc.script)
			if tc.want == "Polish" {
				result, err := (OSExecutor{}).Execute(context.Background(), Command{Name: "bash", Args: []string{"-euc", tc.script + `; [[ $AGENT_LANGUAGE == Polish ]]`}})
				if err != nil || result.ExitCode != 0 {
					t.Fatalf("direct Bash value assertion: exit=%d error=%v", result.ExitCode, err)
				}
			}
			cfg, err := newApp(tc.executor, nil, nil).loadConfig(context.Background(), ConfigRequest{File: file, Args: tc.args, Env: []string{"PATH=/usr/bin:/bin"}, Context: ConfigContext{Home: t.TempDir()}})
			if got := cfg.Scalars["AGENT_LANGUAGE"]; err != nil || got != tc.want {
				t.Fatalf("want %q, got %q, error=%v", tc.want, got, err)
			}
		})
	}
}

func TestConfigReviewBackgroundChild(t *testing.T) {
	fifo := filepath.Join(t.TempDir(), "release")
	if err := syscall.Mkfifo(fifo, 0600); err != nil {
		t.Fatal(err)
	}
	release, err := os.OpenFile(fifo, os.O_RDWR, 0600) // #nosec G304 -- test-owned FIFO under t.TempDir; no external path input
	if err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(t.TempDir(), "config")
	ready := filepath.Join(t.TempDir(), "ready")
	writeContextFixture(t, file, `(exec 4< '`+fifo+`'; : > '`+ready+`'; read -r _ <&4) & while [[ ! -f '`+ready+`' ]]; do :; done; AGENT_LANGUAGE=Polish`)
	done := make(chan *LoadConfigError, 1)
	returned := false
	go func() {
		cfg, loadErr := newApp(OSExecutor{}, nil, nil).loadConfig(context.Background(), ConfigRequest{File: file, Env: []string{"PATH=/usr/bin:/bin"}, Context: ConfigContext{Home: t.TempDir()}})
		if loadErr == nil && cfg.Scalars["AGENT_LANGUAGE"] != "Polish" {
			loadErr = opErr(LoadConfigProtocol, "missing configuration value")
		}
		done <- loadErr
	}()
	// Release the child even on failure; the watchdog detects a deadlock, not a speed threshold.
	defer func() {
		if _, err := release.WriteString("release\n"); err != nil {
			t.Error(err)
		}
		if err := release.Close(); err != nil {
			t.Error(err)
		}
		if !returned {
			<-done
		}
	}()
	select {
	case err := <-done:
		returned = true
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("configuration loading waited for a background child")
	}
}
