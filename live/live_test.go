package main

import (
	"bytes"
	"errors"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestReadMetaUnquotesBashValues(t *testing.T) {
	dir := t.TempDir()
	p := filepath.Join(dir, "meta.env")
	content := "ID=20261020-120000-claude-1\nAGENT=claude-1\nSTATUS=running\nEXTRA_ARGS=-c\\ model_reasoning_effort=\\\"high\\\"\nNOTE=$'line1\\nline2'\nEMPTY=''\n"
	if err := os.WriteFile(p, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	m := readMeta(p)
	cases := map[string]string{
		"AGENT":      "claude-1",
		"STATUS":     "running",
		"EXTRA_ARGS": `-c model_reasoning_effort="high"`,
		"NOTE":       "line1\nline2",
		"EMPTY":      "",
	}
	for k, want := range cases {
		if got := m[k]; got != want {
			t.Errorf("%s = %q, want %q", k, got, want)
		}
	}
}

func TestShortTS(t *testing.T) {
	if got := shortTS("2026-10-20 15:04:05"); got != "15:04:05" {
		t.Fatalf("got %q", got)
	}
	if got := shortTS("15:04:05"); got != "15:04:05" {
		t.Fatalf("old format should stay, got %q", got)
	}
}

func TestHuman(t *testing.T) {
	cases := map[time.Duration]string{42 * time.Second: "42s", 7 * time.Minute: "7m", 65 * time.Minute: "1h05m"}
	for d, want := range cases {
		if got := human(d); got != want {
			t.Errorf("human(%v) = %q, want %q", d, got, want)
		}
	}
}

func TestQueueLineWithSeveralInvestigators(t *testing.T) {
	old := InvSlots
	InvSlots = 2
	defer func() { InvSlots = old }()
	s := &Snapshot{Invs: []*Investigation{
		{ID: 1, Status: "running"}, {ID: 2, Status: "queued"}, {ID: 3, Status: "queued"}, {ID: 4, Status: "waitroom"},
	}}
	// one running + one free slot: one queued request starts right away, one is waiting
	want := "investigations: in progress 1/2, waiting 1/3, waiting room 1"
	if got := s.QueueLine(3); got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

// captureLogs replaces the logger for one test and returns the buffer with its JSON records.
func captureLogs(t *testing.T) *bytes.Buffer {
	t.Helper()
	var buf bytes.Buffer
	old := logger
	logger = slog.New(newJSONHandler(&buf))
	t.Cleanup(func() { logger = old })
	return &buf
}

func TestMalformedMetaValuesAreLoggedNotIgnored(t *testing.T) {
	logs := captureLogs(t)
	if got := parseCount("/x/meta.env", "RUN", "seven"); got != 0 {
		t.Fatalf("parseCount = %d", got)
	}
	if got := parseTime("not-a-time-value"); !got.IsZero() {
		t.Fatalf("parseTime = %v", got)
	}
	if n := strings.Count(logs.String(), `"event":"meta_malformed"`); n != 2 {
		t.Errorf("want 2 meta_malformed records, got %d:\n%s", n, logs)
	}
	// empty values are "not set", not errors
	logs.Reset()
	if parseCount("/x/meta.env", "ROUND", "") != 0 || !parseTime("").IsZero() || logs.Len() != 0 {
		t.Fatalf("empty values must not be logged: %s", logs)
	}
}

func TestMissingFilesAreNotErrors(t *testing.T) {
	logs := captureLogs(t)
	dir := t.TempDir()
	if m := readMeta(filepath.Join(dir, "meta.env")); len(m) != 0 {
		t.Fatalf("readMeta = %v", m)
	}
	if l := tailLines(filepath.Join(dir, "activity.jsonl"), 5, 1024); l != nil {
		t.Fatalf("tailLines = %v", l)
	}
	if logs.Len() != 0 {
		t.Fatalf("a missing optional file must not be logged: %s", logs)
	}
}

func TestCorruptActivityLineIsSkippedAndLogged(t *testing.T) {
	logs := captureLogs(t)
	dir := t.TempDir()
	content := `{"ts":"2026-10-20 10:00:00","k":"cmd","t":"ls"}` + "\n" + `{broken` + "\n"
	if err := os.WriteFile(filepath.Join(dir, "activity.jsonl"), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	got := readActivity(dir, 5)
	if len(got) != 1 || got[0].Text != "ls" {
		t.Fatalf("readActivity = %+v", got)
	}
	if !strings.Contains(logs.String(), `"event":"corrupt_activity_line"`) {
		t.Fatalf("no warning logged: %s", logs)
	}
}

func TestReadFileDistinguishesMissingFromUnreadable(t *testing.T) {
	logs := captureLogs(t)
	dir := t.TempDir()
	if got := readFile(filepath.Join(dir, "missing.md")); got != "" || logs.Len() != 0 {
		t.Fatalf("a missing file is not an error: %q, logs %s", got, logs)
	}
	// a directory cannot be read as a file: that is logged as a Storage problem
	if got := readFile(dir); got != "" {
		t.Fatalf("got %q", got)
	}
	if !strings.Contains(logs.String(), `"event":"file_read_failed"`) || !strings.Contains(logs.String(), `"class":"Storage"`) {
		t.Fatalf("no typed warning: %s", logs)
	}
}

func TestTypedErrors(t *testing.T) {
	serr := storageErr(os.ErrPermission, "cannot read %s", "meta.env")
	if !errors.Is(serr, ErrStorage) || !errors.Is(serr, os.ErrPermission) || serr.Class() != ClassStorage {
		t.Fatalf("StorageError: %v", serr)
	}
	verr := valueWrap(errors.New("bad digit"), "malformed RUN %q", "x")
	if !errors.Is(verr, ErrInvalidValue) || verr.Class() != ClassInvalidValue {
		t.Fatalf("ValueError: %v", verr)
	}
	var nilErr *StorageError
	if nilErr.asError() != nil {
		t.Fatal("asError of nil must be nil")
	}
}
