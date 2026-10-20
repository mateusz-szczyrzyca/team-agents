package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"strings"
	"testing"
	"time"
)

// captureLogs replaces the logger for one test and returns the buffer with its JSON records.
func captureLogs(t *testing.T) *bytes.Buffer {
	t.Helper()
	var buf bytes.Buffer
	old := logger
	logger = slog.New(newJSONHandler(&buf))
	t.Cleanup(func() { logger = old })
	return &buf
}

// records decodes the captured JSON log records.
func records(t *testing.T, buf *bytes.Buffer) []map[string]any {
	t.Helper()
	var out []map[string]any
	for _, line := range strings.Split(strings.TrimSpace(buf.String()), "\n") {
		if line == "" {
			continue
		}
		var r map[string]any
		if err := json.Unmarshal([]byte(line), &r); err != nil {
			t.Fatalf("log line is not JSON: %q", line)
		}
		out = append(out, r)
	}
	return out
}

func TestSideEffectFailureIsLoggedAndDoesNotFailTheOperation(t *testing.T) {
	logs := captureLogs(t)
	b := newBus(t)
	// the investigation record cannot be written (its directory is a file), but the request itself must succeed
	s := server(b, "a", func(s *Server) { s.tasksDir = "/dev/null" })
	out, err := toolResult(s.requestInvestigation("why does it hang?", "", false))
	if err != nil || !strings.HasPrefix(out, "ACCEPTED") {
		t.Fatalf("request failed because of a side effect: %q, %v", out, err)
	}
	var warned bool
	for _, r := range records(t, logs) {
		if r["event"] == string(LogSideEffectFailed) && r["level"] == "warn" && r["effect"] == "write investigation record" &&
			r["class"] == "Storage" {
			warned = true
		}
	}
	if !warned {
		t.Fatalf("no structured warning about the record file:\n%s", logs)
	}
}

func TestCheckpointFileFailureFailsTheTool(t *testing.T) {
	b := newBus(t)
	captureLogs(t)
	// the checkpoint file is the purpose of the tool, so failing to write it is an error, not a warning
	if err := writeFile(t, b.dir+"/checkpoints", "not a directory"); err != nil {
		t.Fatal(err)
	}
	failsWith(t, CheckpointStorage)(server(b, "a").checkpoint("done", "next"))
}

func TestWarnOnceReportsEachKeyOnce(t *testing.T) {
	logs := captureLogs(t)
	err := storageErr(errors.New("disk on fire"), "broken")
	for i := 0; i < 3; i++ {
		warnOnce("test-key-once", LogCorruptMessageLine, err)
	}
	warnOnce("test-key-other", LogCorruptMessageLine, err)
	if n := len(records(t, logs)); n != 2 {
		t.Fatalf("want 2 records (one per key), got %d:\n%s", n, logs)
	}
}

func TestFanoutWritesEveryDestination(t *testing.T) {
	var a, b bytes.Buffer
	l := slog.New(fanout([]slog.Handler{slog.NewJSONHandler(&a, nil), slog.NewJSONHandler(&b, nil)}))
	l.Info("hello", "k", "v")
	if !strings.Contains(a.String(), `"k":"v"`) || !strings.Contains(b.String(), `"k":"v"`) {
		t.Fatalf("a=%q b=%q", a.String(), b.String())
	}
	// without destinations records are discarded without an error
	r := slog.NewRecord(time.Now(), slog.LevelError, "dropped", 0)
	if err := fanout(nil).Handle(context.Background(), r); err != nil {
		t.Fatalf("discarding a record failed: %v", err)
	}
}

func TestRecordsUseTheCommonSchema(t *testing.T) {
	logs := captureLogs(t)
	logWarn(LogToolCallFailed, "tool", "board_claim")
	r := records(t, logs)
	if len(r) != 1 {
		t.Fatalf("got %d records", len(r))
	}
	if r[0]["event"] != "tool_call_failed" || r[0]["level"] != "warn" || r[0]["tool"] != "board_claim" {
		t.Fatalf("record %v", r[0])
	}
	if _, err := time.Parse(time.RFC3339, r[0]["ts"].(string)); err != nil {
		t.Fatalf("ts is not RFC 3339: %v", r[0]["ts"])
	}
	for _, k := range []string{"time", "msg"} {
		if _, ok := r[0][k]; ok {
			t.Fatalf("slog's %q key must be renamed: %v", k, r[0])
		}
	}
}

func TestJournalPriorities(t *testing.T) {
	cases := map[slog.Level]int{slog.LevelDebug: 7, slog.LevelInfo: 6, slog.LevelWarn: 4, slog.LevelError: 3}
	for l, want := range cases {
		if got := journalPriority(l); got != want {
			t.Errorf("%v → %d, want %d", l, got, want)
		}
	}
}

func TestClassOf(t *testing.T) {
	if class, cause := classOf(errors.Join(errors.New("x"), opErr(BoardClaimAlreadyTaken, "y"))); class != "Conflict" || cause != "AlreadyTaken" {
		t.Fatalf("got %q %q", class, cause)
	}
	if class, cause := classOf(errors.New("plain")); class != "" || cause != "" {
		t.Fatalf("got %q %q", class, cause)
	}
}

// writeFile creates a file (helper for tests that need a file where a directory is expected).
func writeFile(t *testing.T, path, content string) error {
	t.Helper()
	return osWriteFile(path, []byte(content))
}
