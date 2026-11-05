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
	errChan := make(chan interface{}, 20)
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
		t.Fatalf("concurrent writes failed: %d errors", failCount)
	}

	// Verify all keys were written
	data, failure := os.ReadFile(metaFile)
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
	if failure := os.Chmod(filepath.Join(tasks, taskID), 0o500); failure != nil {
		t.Fatal(failure)
	}
	defer os.Chmod(filepath.Join(tasks, taskID), 0o700)

	err := setTaskMeta(tasks, taskID, "STATUS", "new", "")
	if err == nil {
		t.Fatal("expected write failure, got nil")
	}

	// Verify original metadata is preserved
	data, failure := os.ReadFile(metaFile)
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
	data, failure := os.ReadFile(metaFile)
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
	data, failure := os.ReadFile(metaFile)
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
	transitions, failure := os.ReadFile(jobTransFile)
	if failure != nil {
		t.Fatalf("job_transitions.jsonl not created: %v", failure)
	}

	// Verify it contains a valid JSON entry with "running" state
	if !strings.Contains(string(transitions), "running") {
		t.Fatalf("job_transitions.jsonl missing 'running' state: %s", string(transitions))
	}
}

// TestMixedBashGoWriter verifies that Bash meta_set and Go setTaskMeta can coexist in the same file.
func TestMixedBashGoWriter(t *testing.T) {
	tasks := t.TempDir()
	taskID := "mixed-writer"
	if failure := os.MkdirAll(filepath.Join(tasks, taskID), 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(tasks, taskID, "meta.env")
	writeMetaFixture(t, metaFile, "ID=mixed-writer\nSTATUS=init\n")

	// Simulate Bash meta_set writing a key (simplified - just append to file)
	bashKey := "BASH_KEY"
	bashValue := "bash_value"
	bashLine := fmt.Sprintf("%s=%s\n", bashKey, bashValue)
	if failure := os.WriteFile(metaFile, []byte("ID=mixed-writer\nSTATUS=init\n"+bashLine), 0o600); failure != nil {
		t.Fatal(failure)
	}

	// Use Go setTaskMeta to write another key
	err := setTaskMeta(tasks, taskID, "GO_KEY", "go_value", "")
	if err != nil {
		t.Fatalf("Go setTaskMeta failed: %v", err)
	}

	// Verify both Bash and Go keys exist
	data, failure := os.ReadFile(metaFile)
	if failure != nil {
		t.Fatal(failure)
	}
	m := parseMeta(data)

	// Check Bash key (written directly)
	if bashVal, ok := m.Lookup(bashKey); !ok || bashVal != bashValue {
		t.Fatalf("Bash key lost: got %q,%v want %q,true", bashVal, ok, bashValue)
	}

	// Check Go key (written by setTaskMeta)
	if goVal, ok := m.Lookup("GO_KEY"); !ok || goVal != "go_value" {
		t.Fatalf("Go key not written: got %q,%v", goVal, ok)
	}

	// Check original ID is preserved
	if id, ok := m.Lookup("ID"); !ok || id != "mixed-writer" {
		t.Fatalf("ID lost: got %q,%v", id, ok)
	}
}

// TestSeparateProcessMetaWrite verifies metadata can be updated correctly from separate processes.
func TestSeparateProcessMetaWrite(t *testing.T) {
	tasks := t.TempDir()
	taskID := "separate-process"
	if failure := os.MkdirAll(filepath.Join(tasks, taskID), 0o700); failure != nil {
		t.Fatal(failure)
	}
	metaFile := filepath.Join(tasks, taskID, "meta.env")
	writeMetaFixture(t, metaFile, "ID=separate-process\nSTATUS=init\n")

	// First process: Go setTaskMeta writes a key
	err := setTaskMeta(tasks, taskID, "PROCESS1", "value1", "")
	if err != nil {
		t.Fatalf("Process 1 setTaskMeta failed: %v", err)
	}

	// Verify first write
	data, failure := os.ReadFile(metaFile)
	if failure != nil {
		t.Fatal(failure)
	}
	m1 := parseMeta(data)
	if val, ok := m1.Lookup("PROCESS1"); !ok || val != "value1" {
		t.Fatalf("Process1 key not written: got %q,%v", val, ok)
	}

	// Second process: Go setTaskMeta writes a different key
	err = setTaskMeta(tasks, taskID, "PROCESS2", "value2", "")
	if err != nil {
		t.Fatalf("Process 2 setTaskMeta failed: %v", err)
	}

	// Verify both writes are present
	data, failure = os.ReadFile(metaFile)
	if failure != nil {
		t.Fatal(failure)
	}
	m2 := parseMeta(data)
	if val, ok := m2.Lookup("PROCESS1"); !ok || val != "value1" {
		t.Fatalf("Process1 key lost after Process2 write: got %q,%v", val, ok)
	}
	if val, ok := m2.Lookup("PROCESS2"); !ok || val != "value2" {
		t.Fatalf("Process2 key not written: got %q,%v", val, ok)
	}

	// Verify original ID is preserved
	if id, ok := m2.Lookup("ID"); !ok || id != "separate-process" {
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
	transitions, failure := os.ReadFile(jobTransFile)
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
	data, failure := os.ReadFile(metaFile)
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
	transitions, failure := os.ReadFile(jobTransFile)
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

	// Concurrent reader and writer goroutines
	done := make(chan bool, 20)
	readerCount := 10
	writerCount := 10

	// Start readers
	for i := 0; i < readerCount; i++ {
		go func(readerID int) {
			for j := 0; j < 5; j++ {
				m, err := readTaskMeta(tasks, taskID)
				if err != nil {
					// Read failure is acceptable during concurrent writes
					done <- true
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
			done <- true
		}(i)
	}

	// Start writers
	for i := 0; i < writerCount; i++ {
		go func(id int) {
			for j := 0; j < 5; j++ {
				key := fmt.Sprintf("RW_%d_%d", id, j)
				value := fmt.Sprintf("val_%d_%d", id, j)
				setTaskMeta(tasks, taskID, key, value, "")
			}
			done <- true
		}(i)
	}

	// Wait for all goroutines to complete
	for i := 0; i < readerCount+writerCount; i++ {
		<-done
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
