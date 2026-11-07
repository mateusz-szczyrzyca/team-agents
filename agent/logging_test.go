package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

// logFailWriter simulates a failed destination without filesystem privilege assumptions.
type logFailWriter struct{}

// Write implements io.Writer with a deterministic failure.
func (logFailWriter) Write([]byte) (int, error) { return 0, io.ErrClosedPipe }

// TestAgentLogSchema checks the Bash record shape and all runner destinations.
func TestAgentLogSchema(t *testing.T) {
	var runner, diagnostic bytes.Buffer
	logger := &Logger{LogDir: t.TempDir(), Subcommand: "_runner", Task: "task-1", PID: 42, Runner: &runner, Stderr: &diagnostic}
	if err := logger.Log(context.Background(), LevelWarn, LogNoticeFailed, "exit_code=7", "detail=a=b", "exit_code=8"); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(logger.LogDir, "agent.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	var record map[string]any
	if err := json.Unmarshal(data, &record); err != nil {
		t.Fatal(err)
	}
	for key, want := range map[string]any{"component": "agent", "subcommand": "_runner", "task": "task-1", "pid": float64(42), "level": "warn", "event": "notice_failed", "exit_code": "8", "detail": "a=b"} {
		if record[key] != want {
			t.Errorf("%s=%v want %v", key, record[key], want)
		}
	}
	if record["ts"] == nil || !bytes.Equal(data, runner.Bytes()) || diagnostic.Len() != 0 {
		t.Fatalf("destinations: %s %s %s", data, runner.Bytes(), diagnostic.Bytes())
	}
}

// TestAgentLogWriteFailure requires diagnostics plus the typed storage cause.
func TestAgentLogWriteFailure(t *testing.T) {
	var diagnostic bytes.Buffer
	path := filepath.Join(t.TempDir(), "file")
	if err := os.WriteFile(path, []byte("occupied"), 0o600); err != nil {
		t.Fatal(err)
	}
	logger := &Logger{LogDir: path, Stderr: &diagnostic}
	failure := logger.Log(context.Background(), LevelInfo, LogCommandFailed)
	if failure == nil || failure.Cause != LogWriteStorage {
		t.Fatalf("failure=%v", failure)
	}
	if !strings.Contains(diagnostic.String(), "agent: cannot write") {
		t.Fatalf("diagnostic=%q", diagnostic.String())
	}
	logger = &Logger{LogDir: t.TempDir(), Task: "task", Runner: logFailWriter{}, Stderr: &diagnostic}
	if failure := logger.Log(context.Background(), LevelError, LogCommandFailed); failure == nil || !errors.Is(failure, io.ErrClosedPipe) {
		t.Fatalf("runner failure=%v", failure)
	}
}

// TestAgentTry captures subprocess stderr without failing for the command's exit status.
func TestAgentTry(t *testing.T) {
	var stdout, diagnostic bytes.Buffer
	logger := &Logger{LogDir: t.TempDir(), Stderr: &diagnostic}
	app := newApp(&OSExecutor{}, nil, nil)
	cmd := Command{Name: "bash", Args: []string{"-c", "printf ok; printf '%2100s' x >&2; exit 7"}, Stdout: &stdout}
	if err := Try(context.Background(), app, logger, LogNoticeFailed, cmd); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(logger.LogDir, "agent.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	var record map[string]any
	if err := json.Unmarshal(data, &record); err != nil {
		t.Fatal(err)
	}
	if stdout.String() != "ok" || record["exit_code"] != "7" || len(record["stderr"].(string)) != 2000 {
		t.Fatalf("stdout=%q record=%v", stdout.String(), record)
	}
	if err := Try(context.Background(), newApp(nil, nil, nil), logger, LogNoticeFailed, Command{Name: "denied"}); err != nil {
		t.Fatal(err)
	}
}

// TestAgentLogHelpers checks jq soft extraction, last-event filtering and activity files.
func TestAgentLogHelpers(t *testing.T) {
	ctx := context.Background()
	logger := &Logger{LogDir: t.TempDir(), Stderr: &bytes.Buffer{}}
	app := newApp(&OSExecutor{}, nil, nil)
	out, err := JQSoft(ctx, app, logger, LogEventScanFailed, strings.NewReader("broken"), "-r", ".x")
	if err != nil || out != "" {
		t.Fatalf("jq=%q %v", out, err)
	}
	task := t.TempDir()
	if err := Act(task, "status", "hello", nil); err != nil {
		t.Fatal(err)
	}
	a, e := fs.ReadFile(os.DirFS(task), "activity.jsonl")
	if e != nil {
		t.Fatal(e)
	}
	b, e := fs.ReadFile(os.DirFS(task), "activity-full.jsonl")
	if e != nil {
		t.Fatal(e)
	}
	if !bytes.Equal(a, b) {
		t.Fatal("activity destinations differ")
	}
	out, err = LastEvent(ctx, app, logger, filepath.Join(task, "activity.jsonl"), `.k == "status"`)
	if err != nil || !strings.Contains(out, `"t":"hello"`) {
		t.Fatalf("last=%q %v", out, err)
	}
	out, err = LastEvent(ctx, app, logger, filepath.Join(task, "missing"), "true")
	if err != nil || out != "" {
		t.Fatalf("missing=%q %v", out, err)
	}
}

// TestAgentRenderLog checks readable output, pass-through and output-write errors.
func TestAgentRenderLog(t *testing.T) {
	var out bytes.Buffer
	input := "plain output\n" + `{"ts":"2026-11-05T12:34:56Z","level":"warn","component":"agent","pid":42,"event":"notice_failed","detail":"two words","empty":""}` + "\n"
	if err := RenderLog(strings.NewReader(input), &out); err != nil {
		t.Fatal(err)
	}
	want := "plain output\n12:34:56 WARN notice_failed detail=\"two words\"\n"
	if out.String() != want {
		t.Fatalf("render=%q want %q", out.String(), want)
	}
	if err := RenderLog(strings.NewReader(input), logFailWriter{}); err == nil || err.Cause != RenderLogStorage {
		t.Fatalf("write=%v", err)
	}
}

// TestAgentLogConcurrentAndJournalFailure verifies atomic records and journal diagnostics.
func TestAgentLogConcurrentAndJournalFailure(t *testing.T) {
	logger := &Logger{LogDir: t.TempDir(), Stderr: &bytes.Buffer{}}
	var wg sync.WaitGroup
	for range 20 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if err := logger.Log(context.Background(), LevelInfo, LogCommandFailed); err != nil {
				t.Error(err)
			}
		}()
	}
	wg.Wait()
	data, err := os.ReadFile(filepath.Join(logger.LogDir, "agent.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	lines := bytes.Split(bytes.TrimSpace(data), []byte("\n"))
	if len(lines) != 20 {
		t.Fatalf("records=%d", len(lines))
	}
	for _, line := range lines {
		var r map[string]any
		if err := json.Unmarshal(line, &r); err != nil {
			t.Fatal(err)
		}
		if _, ok := r["task"]; ok {
			t.Fatal("empty task emitted")
		}
	}
	diagnostic := &bytes.Buffer{}
	logger.Stderr = diagnostic
	logger.Journal = true
	logger.App = newApp(nil, nil, nil)
	if err := logger.Log(context.Background(), LevelWarn, LogNoticeFailed); err == nil || err.Cause != LogWriteStorage {
		t.Fatalf("journal error=%v", err)
	}
	if !strings.Contains(diagnostic.String(), "cannot write to the system journal") {
		t.Fatalf("journal diagnostic=%q", diagnostic.String())
	}
}

// TestAgentLogEnums checks severity and event validation at JSON boundaries.
func TestAgentLogEnums(t *testing.T) {
	if !LevelInfo.Valid() || !LevelWarn.Valid() || !LevelError.Valid() || LogLevel("bad").Valid() {
		t.Fatal("invalid severity validation")
	}
	if !LogNoticeFailed.Valid() || !LogEventScanFailed.Valid() || !LogCommandFailed.Valid() || LogEvent("bad").Valid() {
		t.Fatal("invalid event validation")
	}
	var level LogLevel
	var event LogEvent
	if err := json.Unmarshal([]byte(`"bad"`), &level); !errors.Is(err, ErrInvalidArgument) {
		t.Fatalf("severity error=%v", err)
	}
	if err := json.Unmarshal([]byte(`"bad"`), &event); !errors.Is(err, ErrInvalidArgument) {
		t.Fatalf("event error=%v", err)
	}
}

// logShortWriter models a broken writer that fails to report a short write.
type logShortWriter struct{}

// Write deliberately violates the writer contract to verify output checks.
func (logShortWriter) Write(data []byte) (int, error) { return len(data) - 1, nil }

// TestAgentLogShortWrite requires a typed error even when the writer returns no error.
func TestAgentLogShortWrite(t *testing.T) {
	logger := &Logger{LogDir: t.TempDir(), Task: "task", Runner: logShortWriter{}, Stderr: &bytes.Buffer{}}
	if err := logger.Log(context.Background(), LevelInfo, LogCommandFailed); err == nil || !errors.Is(err, io.ErrShortWrite) {
		t.Fatalf("short write=%v", err)
	}
}

// TestAgentSoftPartialOutput preserves jq output before malformed input and skips corrupt events.
func TestAgentSoftPartialOutput(t *testing.T) {
	ctx := context.Background()
	app := newApp(&OSExecutor{}, nil, nil)
	logger := &Logger{LogDir: t.TempDir(), Stderr: &bytes.Buffer{}}
	out, err := JQSoft(ctx, app, logger, LogEventScanFailed, strings.NewReader("{\"x\":\"one\"}\nbroken"), "-r", ".x")
	if err != nil || out != "one" {
		t.Fatalf("partial output=%q %v", out, err)
	}
	path := filepath.Join(t.TempDir(), "events")
	if err := os.WriteFile(path, []byte("bad\n{\"k\":\"status\",\"t\":\"first\"}\n{\"k\":\"other\"}\n{\"k\":\"status\",\"t\":\"last\"}\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	out, err = LastEvent(ctx, app, logger, path, `.k == "status"`)
	if err != nil || out != `{"k":"status","t":"last"}` {
		t.Fatalf("last valid=%q %v", out, err)
	}
}

// TestLogEventValidRoundTrip checks validity and JSON unmarshaling for all defined log events.
func TestLogEventValidRoundTrip(t *testing.T) {
	events := []LogEvent{
		LogNoticeFailed,
		LogNoticeSent,
		LogEventScanFailed,
		LogCommandFailed,
		LogGitIdentity,
		LogSigningKeyUnreadable,
		LogAgentsGHMissing,
	}
	for _, ev := range events {
		t.Run(string(ev), func(t *testing.T) {
			if !ev.Valid() {
				t.Fatalf("expected event %q to be valid", ev)
			}
			data, err := json.Marshal(ev)
			if err != nil {
				t.Fatalf("json.Marshal(%q) failed: %v", ev, err)
			}
			var decoded LogEvent
			if err := json.Unmarshal(data, &decoded); err != nil {
				t.Fatalf("json.Unmarshal(%s) failed: %v", data, err)
			}
			if decoded != ev {
				t.Fatalf("round trip mismatch: got %q, want %q", decoded, ev)
			}
		})
	}

	for _, invalid := range []string{`""`, `"unknown_event"`, `"bad"`} {
		var decoded LogEvent
		if err := json.Unmarshal([]byte(invalid), &decoded); !errors.Is(err, ErrInvalidArgument) {
			t.Fatalf("expected ErrInvalidArgument for %s", invalid)
		}
	}
}

// TestLogGitHubColorsFailedValidRoundTrip tests validity and JSON round-trip for LogGitHubColorsFailed.
func TestLogGitHubColorsFailedValidRoundTrip(t *testing.T) {
	ev := LogGitHubColorsFailed
	if !ev.Valid() {
		t.Fatalf("expected event %q to be valid", ev)
	}
	data, err := json.Marshal(ev)
	if err != nil {
		t.Fatalf("json.Marshal(%q) failed: %v", ev, err)
	}
	var decoded LogEvent
	if err := json.Unmarshal(data, &decoded); err != nil || decoded != ev {
		t.Fatalf("decoded = %q (err %v), want %q", decoded, err, ev)
	}
}
