package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// operatorHome is the operator's agents home as it was before the sandbox replaced HOME and AGENTS_HOME. Set by TestMain.
var operatorHome string

// TestMain runs the whole test binary in a sandbox: HOME and AGENTS_HOME point to a temporary directory, so no test
// writes logs, state or notebooks into the operator's real agents home; wake-ups and agent commands are not spawned
// unless a test captures them (captureWakes, agentCmdFn replacement). After the run it fails the binary when records
// written by this process reached the operator's real agents home anyway (the guard reads only the log file, so writes of state files are not detected; the sandbox is what prevents them).
func TestMain(m *testing.M) {
	os.Exit(runSandboxed(m))
}

// runSandboxed sets up the sandbox, runs the tests and returns the exit code. It returns instead of exiting so that
// the deferred cleanup of the sandbox directory runs.
func runSandboxed(m *testing.M) int {
	operator, err := operatorAgentsHome()
	if err != nil {
		fmt.Fprintln(os.Stderr, "agentbus tests: cannot locate the operator's real agents home:", err)
		return 2
	}
	operatorHome = operator
	sandbox, err := os.MkdirTemp("", "agentbus-test-")
	if err != nil {
		fmt.Fprintln(os.Stderr, "agentbus tests: cannot create the sandbox:", err)
		return 2
	}
	defer func() {
		// a leftover sandbox only wastes disk space; report it so it is noticed
		if rerr := os.RemoveAll(sandbox); rerr != nil {
			fmt.Fprintln(os.Stderr, "agentbus tests: cannot remove the sandbox:", rerr)
		}
	}()
	if err := os.Setenv("HOME", sandbox); err != nil {
		fmt.Fprintln(os.Stderr, "agentbus tests: cannot set HOME:", err)
		return 2
	}
	if err := os.Setenv("AGENTS_HOME", filepath.Join(sandbox, "agents")); err != nil {
		fmt.Fprintln(os.Stderr, "agentbus tests: cannot set AGENTS_HOME:", err)
		return 2
	}
	wakeFn = noopWake
	agentCmdFn = noopAgentCmd

	logFile := filepath.Join(operator, "logs", "agentbus.jsonl")
	before, err := fileSize(logFile)
	if err != nil {
		fmt.Fprintln(os.Stderr, "agentbus tests: cannot read the real log file:", err)
		return 2
	}
	code := m.Run()
	leaked, err := leakedRecords(logFile, before, os.Getpid())
	if err != nil {
		fmt.Fprintln(os.Stderr, "agentbus tests: cannot check the real log file:", err)
		return 1
	}
	if leaked > 0 {
		fmt.Fprintf(os.Stderr, "agentbus tests: %d log record(s) of this test run reached the real agents home %s; a test must not touch it\n", leaked, operator)
		return 1
	}
	return code
}

// noopWake is the default wakeFn during tests: it neither spawns `agent _wake` nor records anything. A test that
// needs the wake-ups uses captureWakes.
func noopWake(*Team, string) {}

// noopAgentCmd is the default agentCmdFn during tests: it spawns no agent subcommand. A test that needs them
// replaces agentCmdFn itself.
func noopAgentCmd(_, _ string, _ ...string) {}

// operatorAgentsHome is the agents home the operator's account would use outside the tests: $AGENTS_HOME or
// $HOME/agents. It is computed before the sandbox overrides both variables.
func operatorAgentsHome() (string, error) {
	if d := os.Getenv("AGENTS_HOME"); d != "" {
		return d, nil
	}
	h, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(h, "agents"), nil
}

// fileSize returns the size of path, or 0 when it does not exist. The size is the offset from which leakedRecords
// starts looking, so records written before the tests started are not counted.
func fileSize(path string) (int64, error) {
	info, err := os.Stat(path)
	if errors.Is(err, os.ErrNotExist) {
		return 0, nil
	}
	if err != nil {
		return 0, err
	}
	return info.Size(), nil
}

// leakedRecords counts the JSON log records at or after offset in path whose "pid" equals pid, i.e. records this test
// process wrote. A missing file has no records. Lines that are not JSON objects are not log records and are skipped.
func leakedRecords(path string, offset int64, pid int) (leaked int, err error) {
	f, err := os.Open(path) //nolint:gosec // path is the operator's real log file; the guard only reads it
	if errors.Is(err, os.ErrNotExist) {
		return 0, nil
	}
	if err != nil {
		return 0, err
	}
	defer func() {
		// a failed Close on a file opened read-only loses nothing, but it must not be silently dropped
		if cerr := f.Close(); cerr != nil && err == nil {
			err = cerr
		}
	}()
	if _, err := f.Seek(offset, io.SeekStart); err != nil {
		return 0, err
	}
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 0, 64*1024), 4*1024*1024)
	for sc.Scan() {
		var rec map[string]any
		if json.Unmarshal(sc.Bytes(), &rec) != nil {
			continue
		}
		if p, ok := rec["pid"].(float64); ok && int(p) == pid {
			leaked++
		}
	}
	return leaked, sc.Err()
}

// TestLeakedRecordsCountsOnlyThisProcess checks the guard itself: it counts records of the given pid after the offset
// and ignores other processes, records before the offset and lines that are not JSON.
func TestLeakedRecordsCountsOnlyThisProcess(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "agentbus.jsonl")
	before := `{"event":"old","pid":42}` + "\n"
	lines := strings.Join([]string{
		`{"event":"mine","pid":42}`,
		`{"event":"other","pid":7}`,
		`not json`,
		`{"event":"mine_again","pid":42}`,
	}, "\n") + "\n"
	if err := os.WriteFile(path, []byte(before+lines), 0o600); err != nil {
		t.Fatal(err)
	}
	got, err := leakedRecords(path, int64(len(before)), 42)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got != 2 {
		t.Fatalf("leakedRecords = %d, want 2", got)
	}
}

// TestLeakedRecordsMissingFile checks that a missing log file has no leaked records.
func TestLeakedRecordsMissingFile(t *testing.T) {
	got, err := leakedRecords(filepath.Join(t.TempDir(), "absent.jsonl"), 0, 42)
	if err != nil || got != 0 {
		t.Fatalf("leakedRecords = %d, %v; want 0, nil", got, err)
	}
}

// TestSandboxIsolatesHome checks that the tests run with the sandbox agents home, not the operator's.
func TestSandboxIsolatesHome(t *testing.T) {
	if operatorHome == "" {
		t.Fatal("TestMain did not record the operator's agents home")
	}
	if got := agentsHome(); got == operatorHome {
		t.Fatalf("agentsHome() = %s, which is the operator's agents home", got)
	}
}
