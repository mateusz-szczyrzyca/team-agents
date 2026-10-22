package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestHubQualifiesBoardAssignee keeps unqualified board routing local to the calling instance.
func TestHubQualifiesBoardAssignee(t *testing.T) {
	for _, assignee := range []string{"codex-1", "codex-1@other", ""} {
		args := map[string]any{"for": assignee, "title": "Review backend"}
		got := qualifyHubArgs(args, "mac")
		want := assignee
		if assignee == "codex-1" {
			want = "codex-1@mac"
		}
		if got["for"] != want || got["title"] != args["title"] || args["for"] != assignee {
			t.Fatalf("assignee qualification: %v, original %v", got, args)
		}
	}
}

// TestNewEnumJSONRejectsUnknownValues requires concrete value failures and preserves the receiver on failure.
func TestNewEnumJSONRejectsUnknownValues(t *testing.T) {
	for _, data := range []string{`"future"`, `3`, `null`, `[]`} {
		capability := Capability("build:go")
		mode := HubAuthModeToken
		for _, target := range []any{&capability, &mode} {
			err := json.Unmarshal([]byte(data), target)
			var verr *ValueError
			if !errors.As(err, &verr) || !errors.Is(err, ErrInvalidArgument) {
				t.Fatalf("%s into %T: %T %v", data, target, err, err)
			}
		}
		if capability != Capability("build:go") || mode != HubAuthModeToken {
			t.Fatal("invalid JSON changed an existing enum value")
		}
	}
	for value := range RecognizedCapabilities {
		var decoded Capability
		data, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(data, &decoded); err != nil || decoded != value {
			t.Fatalf("supported capability no longer loads: %s %v", value, err)
		}
	}
	for _, value := range []HubAuthMode{HubAuthModeToken, HubAuthModeTailscale} {
		var decoded HubAuthMode
		data, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(data, &decoded); err != nil || decoded != value {
			t.Fatalf("supported authentication mode no longer loads: %s %v", value, err)
		}
	}
}

// TestCopyFileReportsConcreteStorageFailures checks both IO boundaries and successful owner-only copies.
func TestCopyFileReportsConcreteStorageFailures(t *testing.T) {
	dir := t.TempDir()
	source, target := filepath.Join(dir, "source"), filepath.Join(dir, "target")
	if err := copyFile(source, target); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("missing source lost its cause: %v", err)
	}
	if err := os.WriteFile(source, []byte("lease credentials"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := copyFile(source, filepath.Join(source, "invalid-child")); err == nil || !errors.Is(err, ErrStorage) {
		t.Fatalf("destination failure was not typed Storage: %v", err)
	}
	if err := copyFile(source, target); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(target)
	if err != nil || info.Mode().Perm() != 0o600 {
		t.Fatalf("backup permissions: %v %v", info, err)
	}
}

// TestInstancesHasItsOwnStorageCause covers registry and stdout failures under the new operation name.
func TestInstancesHasItsOwnStorageCause(t *testing.T) {
	b := newBus(t)
	if err := os.WriteFile(filepath.Join(b.dir, "instances.json"), []byte("invalid JSON"), 0o600); err != nil {
		t.Fatal(err)
	}
	err := runInstances([]string{"--dir", b.dir})
	expectCause(t, err, CmdInstancesStorage)
	if err.Operation() != "instances" {
		t.Fatalf("wrong failed operation: %s", err.Operation())
	}
	b = newBus(t)
	registry := &HubInstanceRegistry{Bus: b}
	if _, err := registry.Register("mac", "token:mac", instanceRegisterFixture("mac", "session")); err != nil {
		t.Fatal(err)
	}
	old := os.Stdout
	file, errOpen := os.CreateTemp(t.TempDir(), "closed-stdout")
	if errOpen != nil {
		t.Fatal(errOpen)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
	os.Stdout = file
	t.Cleanup(func() { os.Stdout = old })
	expectCause(t, runInstances([]string{"--dir", b.dir}), CmdInstancesStorage)
	os.Stdout = old
	if err := runInstances([]string{"--dir", t.TempDir()}); err != nil {
		t.Fatal(err)
	}
}

// TestSocketCleanupLogsFailuresOnly verifies that a primary failure is not hidden by a cleanup failure.
func TestSocketCleanupLogsFailuresOnly(t *testing.T) {
	logs := captureLogs(t)
	dir := t.TempDir()
	removeHubSocket(filepath.Join(dir, "already-removed.sock"))
	if logs.Len() != 0 {
		t.Fatal("an already-unlinked Unix socket produced a warning")
	}
	if err := os.WriteFile(filepath.Join(dir, "keep"), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	removeHubSocket(dir)
	if !strings.Contains(logs.String(), `"effect":"remove hub unix socket"`) {
		t.Fatal("socket cleanup error was not reported")
	}
}

// TestPreEnumCapabilityRegistryLoads preserves the registered vocabulary in the pre-validation file format.
func TestPreEnumCapabilityRegistryLoads(t *testing.T) {
	data, err := os.ReadFile("testdata/instances-before-capability-enum.json")
	if err != nil {
		t.Fatal(err)
	}
	b := newBus(t)
	if err := os.WriteFile(b.path("instances.json"), data, 0o600); err != nil { //nolint:gosec // G703: fixed fixture filename under newBus(t), which owns t.TempDir()
		t.Fatal(err)
	}
	registry := &HubInstanceRegistry{Bus: b}
	state, serr := registry.load()
	if serr != nil {
		t.Fatal(serr)
	}
	if len(state.Instances["mac"].Manifest.Declared) != 6 {
		t.Fatal("pre-validation capability declarations were lost")
	}
}
