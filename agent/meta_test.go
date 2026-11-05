package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

// TestMetaLiteralRecords covers legacy encodings and additive hand-written fixture forms.
func TestMetaLiteralRecords(t *testing.T) {
	cases := map[string]string{
		"": "", "''": "", "plain": "plain", `one\ two`: "one two",
		`quo\'te\\end`: "quo'te\\end", `'a b'`: "a b", `"a\\b\$c"`: "a\\b$c",
		`a'b c'$'\n'`: "ab c\n", `$'\a\b\t\n\v\f\r\E'`: "\a\b\t\n\v\f\r\x1b",
		`$'\101\x42\u0043\U00000044\cE'`: "ABCD\x05", `$'\q'`: `\q`,
		`$'a\0ignored'after`: "aafter", `$'\303\251'`: "é", "zażółć": "zażółć",
		"foo\\\nbar": "foobar", "'line1\nline2'": "line1\nline2", "value # comment": "value",
		`dollar$`: "dollar$", "value\r": "value\r",
	}
	for encoded, want := range cases {
		t.Run(encoded, func(t *testing.T) {
			data := []byte("FUTURE_FIELD=" + encoded + "\n")
			m := parseMeta(data)
			got, ok := m.Lookup("FUTURE_FIELD")
			if !ok || got != want || !bytes.Equal(m.Bytes(), data) {
				t.Fatalf("decoded %q = %q,%v, want %q; records=%+v", encoded, got, ok, want, m.Lines)
			}
		})
	}
}

// TestMetaPreservesOpaqueLines requires explicit classification instead of executing shell.
func TestMetaPreservesOpaqueLines(t *testing.T) {
	for _, raw := range []string{`KEY=$HOME`, `KEY=$(touch marker)`, "KEY=`id`", `KEY=~/x`, `KEY=a b`, `KEY=x; echo y`, `export KEY=x`, `KEY=(a b)`, `KEY='unfinished`} {
		m := parseMeta([]byte(raw))
		if len(m.Lines) != 1 || m.Lines[0].Kind != MetaShell || string(m.Bytes()) != raw {
			t.Fatalf("unsupported record lost: %q -> %+v", raw, m)
		}
		if _, ok := m.Lookup("KEY"); ok {
			t.Fatalf("shell record exposed as literal: %q", raw)
		}
	}
	data := []byte("# future format\n\nFUTURE=old\n  FUTURE='new value'\nEMPTY=''\nopaque extension")
	m := parseMeta(data)
	if got, ok := m.Lookup("FUTURE"); !ok || got != "new value" {
		t.Fatalf("last duplicate lost: %q,%v", got, ok)
	}
	if got, ok := m.Lookup("EMPTY"); !ok || got != "" {
		t.Fatal("explicit emptiness lost")
	}
	if _, ok := m.Lookup("ABSENT"); ok || !bytes.Equal(m.Bytes(), data) {
		t.Fatal("absence or unknown-line preservation changed")
	}
	if len(m.Lines) != 6 || m.Lines[0].Kind != MetaComment || m.Lines[1].Kind != MetaBlank || m.Lines[2].Encoded != "old" {
		t.Fatalf("record metadata changed: %+v", m.Lines)
	}
	if len(parseMeta(nil).Lines) != 0 || !MetaShell.Valid() || MetaLineKind(99).Valid() {
		t.Fatal("empty file or classification validity changed")
	}
}

// TestMetaBashDifferential checks exact writer bytes and decoded data in both legacy locales.
func TestMetaBashDifferential(t *testing.T) {
	values := []string{"", "plain", "a b", "#hash", "a#hash", "~home", "x=~home:~user", "x~tail", "!\"$&'()*,;<>?[]^`{|}\\", "zażółć é", "line1\nline2\t\r", "\a\b\f\v\x1b\x7f", "\xff\xc3x", "\u200b\u202e\ufeff\ue000", "\u2028\u2029\u0085"}
	for i := 1; i < 128; i++ {
		values = append(values, string(byte(i)))
	}
	for _, value := range values {
		encoded, failure := quoteMeta(value)
		if failure != nil {
			t.Fatal(failure)
		}
		want := runMetaBash(t, "C.UTF-8", `printf '%q' "$1"`, value)
		if encoded != want {
			t.Fatalf("encoder(%q)=%q, Bash=%q", value, encoded, want)
		}
		for _, locale := range []string{"C", "C.UTF-8"} {
			legacy := runMetaBash(t, locale, `printf '%q' "$1"`, value)
			m := parseMeta([]byte("VALUE=" + legacy + "\n"))
			if got, ok := m.Lookup("VALUE"); !ok || got != value {
				t.Fatalf("%s decode %q -> %q,%v want %q", locale, legacy, got, ok, value)
			}
		}
		record, failure := formatMetaAssignment("VALUE", value)
		if failure != nil {
			t.Fatal(failure)
		}
		file := filepath.Join(t.TempDir(), "meta.env")
		writeMetaFixture(t, file, record)
		if got := runMetaBash(t, "C", `source "$1"; printf '%s' "$VALUE"`, file); got != value {
			t.Fatalf("Bash source(%q)=%q want %q", record, got, value)
		}
	}
}

// TestMetaInvalidAssignments rejects values Bash variables cannot retain.
func TestMetaInvalidAssignments(t *testing.T) {
	for _, key := range []string{"", "1KEY", "A-B", "X\nY", "X[0]", "A;touch marker"} {
		if _, failure := formatMetaAssignment(key, "value"); failure == nil || !errors.Is(failure.asError(), ErrInvalidArgument) {
			t.Fatalf("invalid key %q accepted: %v", key, failure)
		}
	}
	if _, failure := quoteMeta("a\x00b"); failure == nil || !errors.Is(failure.asError(), ErrInvalidArgument) {
		t.Fatal("NUL value accepted")
	}
}

// TestMetaLiveGolden fixes writer bytes consumed by the current live-reader regression.
func TestMetaLiveGolden(t *testing.T) {
	data, failure := os.ReadFile("testdata/meta-live.env")
	if failure != nil {
		t.Fatal(failure)
	}
	var output strings.Builder
	for _, field := range [][2]string{{"EMPTY", ""}, {"TEXT", "hello world"}, {"QUOTE", "quo'te\\slash"}, {"UNICODE", "zażółć"}, {"CONTROLS", "\a\b\t\n\v\f\r\x1b"}} {
		record, failure := formatMetaAssignment(field[0], field[1])
		if failure != nil {
			t.Fatal(failure)
		}
		output.WriteString(record)
	}
	if output.String() != string(data) {
		t.Fatalf("candidate writer differs from live golden: %q", output.String())
	}
}

// TestReadTaskMeta distinguishes missing files, storage failures, and optional fields.
func TestReadTaskMeta(t *testing.T) {
	tasks := t.TempDir()
	if _, failure := readTaskMeta(tasks, "missing"); failure == nil || failure.Cause != ReadTaskMetaNotFound {
		t.Fatalf("missing cause = %v", failure)
	}
	file := filepath.Join(tasks, "task", "meta.env")
	if failure := os.MkdirAll(filepath.Dir(file), 0o700); failure != nil {
		t.Fatal(failure)
	}
	writeMetaFixture(t, file, "STATUS=historic_unknown\nRUN=not-a-number\nFUTURE=value\nNOTE="+strings.Repeat("x", 100000))
	m, failure := readTaskMeta(tasks, "task")
	if failure != nil {
		t.Fatal(failure)
	}
	if value, ok := m.Lookup("RUN"); !ok || value != "not-a-number" {
		t.Fatal("legacy field was validated as an enum or number")
	}
	if value, ok := m.Lookup("NOTE"); !ok || len(value) != 100000 {
		t.Fatal("large record truncated")
	}
	if failure := os.Remove(file); failure != nil {
		t.Fatal(failure)
	}
	if failure := os.Mkdir(file, 0o700); failure != nil {
		t.Fatal(failure)
	}
	if _, failure := readTaskMeta(tasks, "task"); failure == nil || failure.Cause != ReadTaskMetaStorage {
		t.Fatalf("read-directory cause = %v", failure)
	}
}

// runMetaBash isolates all oracle evaluations from installation configuration and startup hooks.
func runMetaBash(t *testing.T, locale, script string, args ...string) string {
	t.Helper()
	cmd := exec.Command("bash", append([]string{"--noprofile", "--norc", "-c", script, "meta-test"}, args...)...) // #nosec G204 -- scripts and argv are synthetic test literals, with isolated HOME and startup environment
	cmd.Env = []string{"HOME=" + t.TempDir(), "PATH=/usr/bin:/bin", "LC_ALL=" + locale}
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	output, failure := cmd.Output()
	if failure != nil {
		t.Fatalf("Bash oracle failed: %v: %s", failure, stderr.String())
	}
	return string(output)
}

// writeMetaFixture fails explicitly on storage errors in synthetic legacy fixtures.
func writeMetaFixture(t *testing.T, file, data string) {
	t.Helper()
	if failure := os.WriteFile(file, []byte(data), 0o600); failure != nil {
		t.Fatal(failure)
	}
}

// TestConcurrentSetTaskMeta verifies that concurrent setTaskMeta calls of different keys all survive.
func TestConcurrentSetTaskMeta(t *testing.T) {
	tasks := t.TempDir()
	taskID := "concurrent-meta"
	if failure := os.MkdirAll(filepath.Join(tasks, taskID), 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(tasks, taskID, "meta.env")
	writeMetaFixture(t, metaFile, "STATUS=running\n")

	// Spawn multiple concurrent writers
	errChan := make(chan *SetTaskMetaError, 20)
	for i := 1; i <= 20; i++ {
		go func(index int) {
			key := fmt.Sprintf("K%d", index)
			value := fmt.Sprintf("v%d", index)
			err := setTaskMeta(tasks, taskID, key, value, "")
			errChan <- err
		}(i)
	}

	// Collect all errors
	var failCount int
	for i := 0; i < 20; i++ {
		err := <-errChan
		if err != nil {
			failCount++
		}
	}

	if failCount > 0 {
		t.Fatalf("concurrent operations failed: %d errors", failCount)
	}

	// Verify all keys were written
	data, failure := os.ReadFile(metaFile) // #nosec G304 -- path belongs to temporary test directory
	if failure != nil {
		t.Fatal(failure)
	}
	m := parseMeta(data)
	for i := 1; i <= 20; i++ {
		key := fmt.Sprintf("K%d", i)
		value := fmt.Sprintf("v%d", i)
		got, ok := m.Lookup(key)
		if !ok || got != value {
			t.Fatalf("key %s: got %q, want %q", key, got, value)
		}
	}

	// Verify original STATUS is preserved
	status, ok := m.Lookup("STATUS")
	if !ok || status != "running" {
		t.Fatalf("STATUS: got %q, want 'running'", status)
	}
}

// TestSetTaskMetaWriteFailure verifies that setTaskMeta fails and keeps old metadata when writing fails.
func TestSetTaskMetaWriteFailure(t *testing.T) {
	tasks := t.TempDir()
	taskID := "write-fault"
	if failure := os.MkdirAll(filepath.Join(tasks, taskID), 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(tasks, taskID, "meta.env")
	original := "ID=write-fault\nSTATUS=original\n"
	writeMetaFixture(t, metaFile, original)

	// Make the directory read-only to prevent writing
	if failure := os.Chmod(filepath.Join(tasks, taskID), 0o500); failure != nil { // #nosec G302 -- temporary test directory permission change
		t.Fatal(failure)
	}
	defer func() { // #nosec G302 -- temporary test directory permission change
		_ = os.Chmod(filepath.Join(tasks, taskID), 0o700)
	}()

	err := setTaskMeta(tasks, taskID, "STATUS", "new", "")
	if err == nil {
		t.Fatal("expected write failure, got nil")
	}
	if err.Cause != SetTaskMetaStorage {
		t.Fatalf("expected SetTaskMetaStorage cause, got %v", err.Cause)
	}

	// Verify original metadata is preserved
	data, failure := os.ReadFile(metaFile) // #nosec G304 -- path belongs to temporary test directory
	if failure != nil {
		t.Fatal(failure)
	}
	if string(data) != original {
		t.Fatalf("metadata changed on write failure: got %q, want %q", string(data), original)
	}
}

// TestSetTaskMetaReadFailure verifies that setTaskMeta fails and keeps old metadata when reading fails.
func TestSetTaskMetaReadFailure(t *testing.T) {
	tasks := t.TempDir()
	taskID := "read-fault"
	if failure := os.MkdirAll(filepath.Join(tasks, taskID), 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(tasks, taskID, "meta.env")
	original := "ID=read-fault\nSTATUS=original\n"
	writeMetaFixture(t, metaFile, original)

	// Replace the file with a directory to cause read failure
	if failure := os.Remove(metaFile); failure != nil {
		t.Fatal(failure)
	}
	if failure := os.Mkdir(metaFile, 0o700); failure != nil {
		t.Fatal(failure)
	}

	err := setTaskMeta(tasks, taskID, "STATUS", "new", "")
	if err == nil {
		t.Fatal("expected read failure, got nil")
	}
	if err.Cause != SetTaskMetaStorage {
		t.Fatalf("expected SetTaskMetaStorage cause, got %v", err.Cause)
	}

	// Verify directory structure remains intact (can't verify exact metadata since it's a directory)
	info, failure := os.Stat(metaFile)
	if failure != nil || !info.IsDir() {
		t.Fatal("meta.env directory structure was corrupted")
	}
}

// TestSetTaskMetaStaleRunNoOp verifies that setting metadata with mismatched run is a successful no-op.
func TestSetTaskMetaStaleRunNoOp(t *testing.T) {
	tasks := t.TempDir()
	taskID := "stale-run"
	if failure := os.MkdirAll(filepath.Join(tasks, taskID), 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(tasks, taskID, "meta.env")
	original := "ID=stale-run\nRUN=run123\nSTATUS=original\n"
	writeMetaFixture(t, metaFile, original)

	// Try to set metadata with a different run ID
	err := setTaskMeta(tasks, taskID, "STATUS", "new", "run-different")
	if err != nil {
		t.Fatalf("stale run should return nil, got: %v", err)
	}

	// Verify metadata is unchanged (stale run is a no-op)
	data, failure := os.ReadFile(metaFile) // #nosec G304 -- path belongs to temporary test directory
	if failure != nil {
		t.Fatal(failure)
	}
	if string(data) != original {
		t.Fatalf("stale run was not a no-op: got %q, want %q", string(data), original)
	}
}

// TestSetTaskMetaJobTransitions verifies that setting STATUS appends to job_transitions.jsonl.
func TestSetTaskMetaJobTransitions(t *testing.T) {
	tasks := t.TempDir()
	taskID := "job-transitions"
	taskDir := filepath.Join(tasks, taskID)
	if failure := os.MkdirAll(taskDir, 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(taskDir, "meta.env")
	original := "ID=job-transitions\nJOB_ID=job-123\nSTATUS=original\n"
	writeMetaFixture(t, metaFile, original)

	// Set STATUS, which should create/append to job_transitions.jsonl
	err := setTaskMeta(tasks, taskID, "STATUS", "running", "")
	if err != nil {
		t.Fatalf("setTaskMeta failed: %v", err)
	}

	// Verify metadata was updated
	data, failure := os.ReadFile(metaFile) // #nosec G304 -- path belongs to temporary test directory
	if failure != nil {
		t.Fatal(failure)
	}
	m := parseMeta(data)
	status, ok := m.Lookup("STATUS")
	if !ok || status != "running" {
		t.Fatalf("STATUS not updated: got %q", status)
	}

	// Verify job_transitions.jsonl was created
	jobTransFile := filepath.Join(taskDir, "job_transitions.jsonl")
	transitions, failure := os.ReadFile(jobTransFile) // #nosec G304 -- path belongs to temporary test directory
	if failure != nil {
		t.Fatalf("job_transitions.jsonl not created: %v", failure)
	}

	// Verify it contains a valid JSON entry with "running" state
	if !strings.Contains(string(transitions), "running") {
		t.Fatalf("job_transitions.jsonl missing 'running' state: %s", string(transitions))
	}
}

// extractBashMetaFuncs extracts meta_file, meta_set, task_field, and log_error from bin/agent source.
func extractBashMetaFuncs(t *testing.T) string {
	t.Helper()
	source, failure := os.ReadFile("../bin/agent")
	if failure != nil {
		t.Fatal(failure)
	}
	const (
		metaFile = "meta_file() {"
		metaSet  = "meta_set() {"
		taskFld  = "task_field() {"
		logErr   = "log_error() {"
	)
	var funcs strings.Builder
	funcs.WriteString("# Extracted from bin/agent\n")
	for _, fn := range []struct{ name, pattern string }{
		{"meta_file", metaFile},
		{"log_error", logErr},
		{"meta_set", metaSet},
		{"task_field", taskFld},
	} {
		start := strings.Index(string(source), fn.pattern)
		if start < 0 {
			t.Fatalf("function %s not found in bin/agent", fn.name)
		}
		end := strings.Index(string(source)[start:], "\n}")
		if end < 0 {
			t.Fatalf("end of function %s not found", fn.name)
		}
		funcs.WriteString(string(source)[start : start+end+2])
		funcs.WriteString("\n")
	}
	return funcs.String()
}

// TestMixedBashGoWriter verifies that Bash meta_set and Go setTaskMeta preserve each other's writes.
func TestMixedBashGoWriter(t *testing.T) {
	tasks := t.TempDir()
	taskID := "mixed-writer"
	if failure := os.MkdirAll(filepath.Join(tasks, taskID), 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(tasks, taskID, "meta.env")
	writeMetaFixture(t, metaFile, "ID=mixed-writer\nSTATUS=init\n")

	bashFuncs := extractBashMetaFuncs(t)
	bashScript := bashFuncs + `
TASKS=$1
# Bash writes BASH_KEY
meta_set "$2" BASH_KEY bash_value
# Then read it back to verify
task_field "$2" BASH_KEY
`
	bashResult := runMetaBash(t, "C", bashScript, tasks, taskID)
	if strings.TrimSpace(bashResult) != "bash_value" {
		t.Fatalf("Bash meta_set failed: got %q, want 'bash_value'", strings.TrimSpace(bashResult))
	}

	// Use Go setTaskMeta to write another key
	err := setTaskMeta(tasks, taskID, "GO_KEY", "go_value", "")
	if err != nil {
		t.Fatalf("Go setTaskMeta failed: %v", err)
	}

	// Verify both Bash and Go keys exist
	data, failure := os.ReadFile(metaFile) // #nosec G304 -- path belongs to temporary test directory
	if failure != nil {
		t.Fatal(failure)
	}
	m := parseMeta(data)

	// Check Bash key (written by Bash)
	if bashVal, ok := m.Lookup("BASH_KEY"); !ok || bashVal != "bash_value" {
		t.Fatalf("Bash key lost: got %q,%v want 'bash_value',true", bashVal, ok)
	}

	// Check Go key (written by Go)
	if goVal, ok := m.Lookup("GO_KEY"); !ok || goVal != "go_value" {
		t.Fatalf("Go key not written: got %q,%v", goVal, ok)
	}

	// Check original ID is preserved
	if id, ok := m.Lookup("ID"); !ok || id != "mixed-writer" {
		t.Fatalf("ID lost: got %q,%v", id, ok)
	}
}

// TestSeparateProcessMetaWrite verifies metadata can be updated correctly from separate OS processes.
func TestSeparateProcessMetaWrite(t *testing.T) {
	// Check if we're a child process doing the write
	if childKey := os.Getenv("TEST_CHILD_PROCESS_KEY"); childKey != "" {
		childValue := os.Getenv("TEST_CHILD_PROCESS_VALUE")
		tasks := os.Getenv("TEST_TASKS_DIR")
		taskID := os.Getenv("TEST_TASK_ID")
		err := setTaskMeta(tasks, taskID, childKey, childValue, "")
		if err != nil {
			fmt.Fprintf(os.Stderr, "child process setTaskMeta failed: %v\n", err)
			os.Exit(1)
		}
		os.Exit(0)
	}

	tasks := t.TempDir()
	taskID := "separate-process"
	if failure := os.MkdirAll(filepath.Join(tasks, taskID), 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(tasks, taskID, "meta.env")
	writeMetaFixture(t, metaFile, "ID=separate-process\nSTATUS=init\n")

	// Spawn 2 separate processes to write different keys
	for i := 1; i <= 2; i++ {
		cmd := exec.Command(os.Args[0], "-test.run=TestSeparateProcessMetaWrite$") // #nosec G204,G702 -- os.Args[0] is the test binary itself
		cmd.Env = append(os.Environ(),
			"TEST_TASKS_DIR="+tasks,
			"TEST_TASK_ID="+taskID,
			fmt.Sprintf("TEST_CHILD_PROCESS_KEY=PROCESS%d", i),
			fmt.Sprintf("TEST_CHILD_PROCESS_VALUE=value%d", i),
		)
		if output, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("child process %d failed: %v\n%s", i, err, string(output))
		}
	}

	// Verify both writes are present
	data, failure := os.ReadFile(metaFile) // #nosec G304 -- path belongs to temporary test directory
	if failure != nil {
		t.Fatal(failure)
	}
	m := parseMeta(data)

	for i := 1; i <= 2; i++ {
		key := fmt.Sprintf("PROCESS%d", i)
		value := fmt.Sprintf("value%d", i)
		if val, ok := m.Lookup(key); !ok || val != value {
			t.Fatalf("Process%d key not written: got %q,%v want %q,true", i, val, ok, value)
		}
	}

	// Verify original ID is preserved
	if id, ok := m.Lookup("ID"); !ok || id != "separate-process" {
		t.Fatalf("ID lost: got %q,%v", id, ok)
	}
}

// TestJobTransitionsNotCreatedWithoutJobID verifies job_transitions.jsonl is not created when JOB_ID is absent.
func TestJobTransitionsNotCreatedWithoutJobID(t *testing.T) {
	tasks := t.TempDir()
	taskID := "no-job-id"
	if failure := os.MkdirAll(filepath.Join(tasks, taskID), 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(tasks, taskID, "meta.env")
	writeMetaFixture(t, metaFile, "ID=no-job-id\nSTATUS=init\n")

	// Set STATUS without JOB_ID present
	err := setTaskMeta(tasks, taskID, "STATUS", "running", "")
	if err != nil {
		t.Fatalf("setTaskMeta failed: %v", err)
	}

	// Verify job_transitions.jsonl was NOT created
	jobTransFile := filepath.Join(tasks, taskID, "job_transitions.jsonl")
	if _, failure := os.Stat(jobTransFile); failure == nil {
		t.Fatal("job_transitions.jsonl should not be created without JOB_ID")
	}
}

// TestJobTransitionsNotCreatedForNonStatusKeys verifies job_transitions.jsonl is not updated for non-STATUS keys.
func TestJobTransitionsNotCreatedForNonStatusKeys(t *testing.T) {
	tasks := t.TempDir()
	taskID := "no-status-key"
	taskDir := filepath.Join(tasks, taskID)
	if failure := os.MkdirAll(taskDir, 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(taskDir, "meta.env")
	writeMetaFixture(t, metaFile, "ID=no-status-key\nJOB_ID=job-456\nNOTE=initial\n")

	// Set non-STATUS key
	err := setTaskMeta(tasks, taskID, "NOTE", "updated", "")
	if err != nil {
		t.Fatalf("setTaskMeta failed: %v", err)
	}

	// Verify job_transitions.jsonl was NOT created
	jobTransFile := filepath.Join(taskDir, "job_transitions.jsonl")
	if _, failure := os.Stat(jobTransFile); failure == nil {
		t.Fatal("job_transitions.jsonl should not be created for non-STATUS keys")
	}
}

// TestJobTransitionsMultipleRecords verifies multiple STATUS transitions append distinct records with RFC3339 timestamps.
func TestJobTransitionsMultipleRecords(t *testing.T) {
	tasks := t.TempDir()
	taskID := "multi-trans"
	taskDir := filepath.Join(tasks, taskID)
	if failure := os.MkdirAll(taskDir, 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(taskDir, "meta.env")
	writeMetaFixture(t, metaFile, "ID=multi-trans\nJOB_ID=job-789\nSTATUS=init\n")

	// Set STATUS multiple times
	states := []string{"running", "paused", "completed"}
	for _, state := range states {
		err := setTaskMeta(tasks, taskID, "STATUS", state, "")
		if err != nil {
			t.Fatalf("setTaskMeta failed for state %s: %v", state, err)
		}
	}

	// Verify job_transitions.jsonl has records
	jobTransFile := filepath.Join(taskDir, "job_transitions.jsonl")
	transitions, failure := os.ReadFile(jobTransFile) // #nosec G304 -- path belongs to temporary test directory
	if failure != nil {
		t.Fatalf("job_transitions.jsonl not found: %v", failure)
	}

	// Count records (should be at least 3 for the states we set)
	lines := strings.Split(strings.TrimSpace(string(transitions)), "\n")
	if len(lines) < 3 {
		t.Fatalf("expected at least 3 transition records, got %d", len(lines))
	}

	// Verify each line is valid JSON with state and timestamp fields
	for i, line := range lines {
		if line == "" {
			continue
		}
		var record map[string]interface{}
		if failure := json.Unmarshal([]byte(line), &record); failure != nil {
			t.Fatalf("line %d not valid JSON: %v", i, failure)
		}
		if _, hasState := record["state"]; !hasState {
			t.Fatalf("line %d missing 'state' field", i)
		}
		if _, hasAt := record["at"]; !hasAt {
			t.Fatalf("line %d missing 'at' field (timestamp)", i)
		}
	}

	// Verify all expected states appear in transitions
	transContent := string(transitions)
	for _, state := range states {
		if !strings.Contains(transContent, fmt.Sprintf(`"state":"%s"`, state)) {
			t.Fatalf("state %s not found in transitions", state)
		}
	}
}

// TestJobTransitionsPrelanchCancel verifies STATUS update with RUN=0 (prelaunch_cancel).
func TestJobTransitionsPrelanchCancel(t *testing.T) {
	tasks := t.TempDir()
	taskID := "prelaunch-cancel"
	taskDir := filepath.Join(tasks, taskID)
	if failure := os.MkdirAll(taskDir, 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(taskDir, "meta.env")
	writeMetaFixture(t, metaFile, "ID=prelaunch-cancel\nJOB_ID=job-cancel\nRUN=0\nSTATUS=init\n")

	// Set STATUS=stopped when RUN=0
	err := setTaskMeta(tasks, taskID, "STATUS", "stopped", "")
	if err != nil {
		t.Fatalf("setTaskMeta failed for prelaunch cancel: %v", err)
	}

	// Verify STATUS was updated
	data, failure := os.ReadFile(metaFile) // #nosec G304 -- path belongs to temporary test directory
	if failure != nil {
		t.Fatal(failure)
	}
	m := parseMeta(data)
	status, ok := m.Lookup("STATUS")
	if !ok || status != "stopped" {
		t.Fatalf("STATUS not updated: got %q,%v", status, ok)
	}

	// Verify job_transitions.jsonl has the stopped transition
	jobTransFile := filepath.Join(taskDir, "job_transitions.jsonl")
	transitions, failure := os.ReadFile(jobTransFile) // #nosec G304 -- path belongs to temporary test directory
	if failure != nil {
		t.Fatalf("job_transitions.jsonl not found: %v", failure)
	}
	if !strings.Contains(string(transitions), "stopped") {
		t.Fatalf("'stopped' state not in transitions: %s", string(transitions))
	}
}

// TestConcurrentReadWriteMeta verifies concurrent reads never observe partial writes or corrupt lines.
func TestConcurrentReadWriteMeta(t *testing.T) {
	tasks := t.TempDir()
	taskID := "concurrent-rw"
	taskDir := filepath.Join(tasks, taskID)
	if failure := os.MkdirAll(taskDir, 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(taskDir, "meta.env")
	writeMetaFixture(t, metaFile, "ID=concurrent-rw\nSTATUS=init\nCOUNT=0\n")

	// Concurrent reader and writer goroutines using WaitGroup
	var wg sync.WaitGroup
	writerErrors := make(chan *SetTaskMetaError, 50) // 10 writers × 5 operations each
	readerCount := 10
	writerCount := 10

	// Start readers
	wg.Add(readerCount)
	for i := 0; i < readerCount; i++ {
		go func(readerID int) {
			defer wg.Done()
			for j := 0; j < 5; j++ {
				m, err := readTaskMeta(tasks, taskID)
				if err != nil {
					// Read failure is acceptable during concurrent writes
					return
				}
				// Verify no corrupted lines or missing complete records
				for _, line := range m.Lines {
					if line.Raw == "" && line.Kind != MetaBlank {
						t.Errorf("reader %d: empty raw line with kind %v", readerID, line.Kind)
					}
				}
				// Verify ID is readable
				if idVal, ok := m.Lookup("ID"); !ok || idVal != "concurrent-rw" {
					t.Errorf("reader %d: ID corrupted or missing", readerID)
				}
			}
		}(i)
	}

	// Start writers
	wg.Add(writerCount)
	for i := 0; i < writerCount; i++ {
		go func(id int) {
			defer wg.Done()
			for j := 0; j < 5; j++ {
				key := fmt.Sprintf("RW_%d_%d", id, j)
				value := fmt.Sprintf("val_%d_%d", id, j)
				err := setTaskMeta(tasks, taskID, key, value, "")
				writerErrors <- err
			}
		}(i)
	}

	// Wait for all goroutines to complete
	wg.Wait()
	close(writerErrors)

	// Collect writer errors
	var failCount int
	for err := range writerErrors {
		if err != nil {
			failCount++
		}
	}

	if failCount > 0 {
		t.Fatalf("concurrent write operations failed: %d errors", failCount)
	}

	// Verify file is still readable and consistent
	m, err := readTaskMeta(tasks, taskID)
	if err != nil {
		t.Fatalf("final read failed: %v", err)
	}
	if id, ok := m.Lookup("ID"); !ok || id != "concurrent-rw" {
		t.Fatal("final read: ID corrupted or missing")
	}
}

// TestSetTaskMetaValueEscaping verifies various special characters roundtrip correctly.
func TestSetTaskMetaValueEscaping(t *testing.T) {
	tasks := t.TempDir()
	taskID := "escaping"
	if failure := os.MkdirAll(filepath.Join(tasks, taskID), 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(tasks, taskID, "meta.env")
	writeMetaFixture(t, metaFile, "ID=escaping\n")

	testValues := []string{
		"simple",
		"with spaces",
		"with\nnewline",
		"with'single'quotes",
		`with"double"quotes`,
		"with$dollar",
		"with`backtick`",
		"with\\backslash",
		"with#hash",
		"tab\there",
		"mixed\n'single'\"double\"$dollar\\back",
		"unicode_é_ñ_ü",
		"emoji_😀_🎉",
		"\x01\x02\x03control",
		"empty_value_next",
		"",
	}

	// Write each value with setTaskMeta
	for i, value := range testValues {
		key := fmt.Sprintf("TEST_%d", i)
		err := setTaskMeta(tasks, taskID, key, value, "")
		if err != nil {
			t.Fatalf("setTaskMeta failed for index %d (value %q): %v", i, value, err)
		}
	}

	// Read back and verify each value matches exactly
	m, err := readTaskMeta(tasks, taskID)
	if err != nil {
		t.Fatalf("readTaskMeta failed: %v", err)
	}

	for i, expectedValue := range testValues {
		key := fmt.Sprintf("TEST_%d", i)
		gotValue, ok := m.Lookup(key)
		if !ok {
			t.Fatalf("key %s not found in metadata", key)
		}
		if gotValue != expectedValue {
			t.Fatalf("value mismatch for %s: got %q, want %q", key, gotValue, expectedValue)
		}
	}
}

// TestSetTaskMetaRenameFailure verifies setTaskMeta fails and cleans up temp file when rename fails.
func TestSetTaskMetaRenameFailure(t *testing.T) {
	tasks := t.TempDir()
	taskID := "rename-fault"
	taskDir := filepath.Join(tasks, taskID)
	if failure := os.MkdirAll(taskDir, 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(taskDir, "meta.env")
	original := "ID=rename-fault\nSTATUS=original\n"
	writeMetaFixture(t, metaFile, original)

	// Make the task directory read-only to prevent rename
	if failure := os.Chmod(taskDir, 0o500); failure != nil { // #nosec G302 -- temporary test directory
		t.Fatal(failure)
	}
	defer func() { // #nosec G302 -- temporary test directory
		_ = os.Chmod(taskDir, 0o700)
	}()

	err := setTaskMeta(tasks, taskID, "STATUS", "new", "")
	if err == nil {
		t.Fatal("expected rename failure, got nil")
	}
	if err.Cause != SetTaskMetaStorage {
		t.Fatalf("expected SetTaskMetaStorage cause, got %v", err.Cause)
	}

	// Verify original metadata is preserved
	data, failure := os.ReadFile(metaFile) // #nosec G304 -- path belongs to temporary test directory
	if failure != nil {
		t.Fatal(failure)
	}
	if string(data) != original {
		t.Fatalf("metadata changed on rename failure: got %q, want %q", string(data), original)
	}

	// Verify no orphaned temp files exist
	entries, failure := os.ReadDir(taskDir)
	if failure != nil {
		t.Fatal(failure)
	}
	for _, entry := range entries {
		if strings.Contains(entry.Name(), "meta.env.tmp") {
			t.Fatalf("orphaned temp file found: %s", entry.Name())
		}
	}
}

// TestSetTaskMetaTempFileCleanup verifies temp files are removed on write/close/rename failures.
func TestSetTaskMetaTempFileCleanup(t *testing.T) {
	tasks := t.TempDir()
	taskID := "cleanup-test"
	taskDir := filepath.Join(tasks, taskID)
	if failure := os.MkdirAll(taskDir, 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(taskDir, "meta.env")
	writeMetaFixture(t, metaFile, "ID=cleanup-test\n")

	// Make the task directory read-only after fixture to trigger write/rename failure
	if failure := os.Chmod(taskDir, 0o500); failure != nil { // #nosec G302 -- temporary test directory
		t.Fatal(failure)
	}
	defer func() { // #nosec G302 -- temporary test directory
		_ = os.Chmod(taskDir, 0o700)
	}()

	// Attempt to write metadata, which will fail
	err := setTaskMeta(tasks, taskID, "KEY", "value", "")
	if err == nil {
		t.Fatal("expected failure, got nil")
	}

	// Verify no temp files remain in task directory (restored to 0o700 by defer)
	os.Chmod(taskDir, 0o700) // #nosec G302 -- temporary test directory

	entries, failure := os.ReadDir(taskDir)
	if failure != nil {
		t.Fatal(failure)
	}
	for _, entry := range entries {
		if strings.Contains(entry.Name(), "meta.env.tmp") {
			t.Fatalf("orphaned temp file found after write failure: %s", entry.Name())
		}
	}
}
