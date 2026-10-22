package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestStatusComparesSerializedHubLeaseInstants reproduces same-day RFC3339 expiry using actual hub encoding.
func TestStatusComparesSerializedHubLeaseInstants(t *testing.T) {
	home := t.TempDir()
	bus := &Bus{dir: filepath.Join(home, "bus")}
	if err := os.MkdirAll(bus.dir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(home, "agents.conf"), []byte("declare -A AGENT_TYPE=()\nQUESTIONS_EDITOR=off\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	// Freeze now at midday so same-day past and future leases are deterministic even around midnight.
	now := time.Date(2026, 10, 22, 12, 0, 0, 0, time.UTC)
	registry := &HubInstanceRegistry{Bus: bus, Now: func() time.Time { return now }}
	if _, err := registry.Register("mac", "token:mac", instanceRegisterFixture("mac", "session")); err != nil {
		t.Fatal(err)
	}
	bin := t.TempDir()
	date := fmt.Sprintf("#!/bin/sh\nif [ \"$1\" = '+%%s' ]; then printf '%%s\\n' '%d'; else /bin/date \"$@\"; fi\n", now.Unix())
	if err := os.WriteFile(filepath.Join(bin, "date"), []byte(date), 0o700); err != nil { //nolint:gosec // G306: private fixture command needs execute permission, never installed on the host
		t.Fatal(err)
	}
	for _, delta := range []time.Duration{-time.Minute, time.Minute} {
		if serr := registry.withState(func(state *hubInstanceState) *StorageError {
			state.Instances["mac"].Lease.ExpiresAt = Timestamp{Time: now.Add(delta + 500*time.Millisecond)}
			return registry.save(state)
		}); serr != nil {
			t.Fatal(serr)
		}
		data, err := os.ReadFile(bus.path("instances.json"))
		if err != nil {
			t.Fatal(err)
		}
		if !strings.Contains(string(data), now.Format("2006-01-02")+"T") {
			t.Fatal("test no longer uses the hub's actual RFC3339 lease encoding")
		}
		script, err := filepath.Abs(filepath.Join("..", "bin", "agent"))
		if err != nil {
			t.Fatal(err)
		}
		cmd := exec.Command(script, "status") //nolint:gosec // G204: repository status script only, temporary AGENTS_HOME, no tasks or real agent CLI
		cmd.Env = append(os.Environ(), "AGENTS_HOME="+home, "PATH="+bin+string(os.PathListSeparator)+os.Getenv("PATH"))
		out, err := cmd.CombinedOutput()
		if err != nil {
			t.Fatalf("status failed: %v: %s", err, out)
		}
		want := "expired"
		if delta > 0 {
			want = "active"
		}
		if !strings.Contains(string(out), want) {
			t.Fatalf("same-day lease should be %s: %s", want, out)
		}
	}
}
