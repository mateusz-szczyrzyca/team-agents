package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// instancesFixture is a registry file with one active and one expired instance, in the storage layout.
const instancesFixture = `{
  "version": 1,
  "instances": {
    "pi5": {
      "peer": "token:pi5",
      "lease": {"version": 1, "lease_token": "x", "generation": 2, "expires_at": "2099-01-01 00:00:00"},
      "sequence": 4,
      "manifest": {
        "instance_name": "pi5",
        "detected": {"os": "linux", "arch": "arm64"},
        "declared": ["build:c", "hw:rp2350-probe"],
        "current_state": {"slots": {"total": 4, "free": 3}, "tasks": {"t1": {"task_id": "t1"}, "t2": {"task_id": "t2"}}}
      }
    },
    "mac": {
      "peer": "token:mac",
      "lease": {"version": 1, "lease_token": "y", "generation": 1, "expires_at": "2000-01-01 00:00:00"},
      "sequence": 9,
      "manifest": {
        "instance_name": "mac",
        "detected": {"os": "darwin", "arch": "arm64"},
        "declared": [],
        "current_state": {"slots": {"total": 2, "free": 0}}
      }
    }
  }
}`

// writeInstances stores content as bus/instances.json in a fresh bus directory and returns the directory.
func writeInstances(t *testing.T, content string) string {
	t.Helper()
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "instances.json"), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	return dir
}

func TestLoadInstancesReadsLeaseSlotsAndCapabilities(t *testing.T) {
	now := time.Date(2026, 10, 22, 12, 0, 0, 0, time.UTC)
	got := loadInstances(writeInstances(t, instancesFixture), now)
	if len(got) != 2 || got[0].Name != "mac" || got[1].Name != "pi5" {
		t.Fatalf("instances not sorted by name: %+v", got)
	}
	mac, pi := got[0], got[1]
	if mac.Active || mac.OS != "darwin" || mac.Arch != "arm64" || mac.SlotsTotal != 2 || mac.SlotsFree != 0 {
		t.Fatalf("expired instance read wrong: %+v", mac)
	}
	if !pi.Active || pi.OS != "linux" || pi.SlotsFree != 3 || pi.SlotsTotal != 4 || pi.Tasks != 2 {
		t.Fatalf("active instance read wrong: %+v", pi)
	}
	if strings.Join(pi.Capabilities, ",") != "build:c,hw:rp2350-probe" {
		t.Fatalf("capabilities: %v", pi.Capabilities)
	}
	if !pi.ExpiresAt.Equal(time.Date(2099, 1, 1, 0, 0, 0, 0, time.UTC)) {
		t.Fatalf("expiry: %v", pi.ExpiresAt)
	}
}

func TestLoadInstancesAcceptsRFC3339Expiry(t *testing.T) {
	content := `{"instances": {"x": {"lease": {"expires_at": "2099-01-01T00:00:00Z"}, "manifest": {}}}}`
	got := loadInstances(writeInstances(t, content), time.Now())
	if len(got) != 1 || !got[0].Active {
		t.Fatalf("RFC 3339 expiry rejected: %+v", got)
	}
}

func TestLoadInstancesWithoutFileIsEmpty(t *testing.T) {
	logs := captureLogs(t)
	if got := loadInstances(t.TempDir(), time.Now()); got != nil {
		t.Fatalf("single-instance installation shows instances: %+v", got)
	}
	if logs.Len() != 0 {
		t.Fatalf("a missing registry file is not a problem: %s", logs)
	}
}

func TestLoadInstancesLogsUndecodableFile(t *testing.T) {
	logs := captureLogs(t)
	if got := loadInstances(writeInstances(t, "{not json"), time.Now()); got != nil {
		t.Fatalf("undecodable file gave instances: %+v", got)
	}
	if !strings.Contains(logs.String(), `"event":"bus_state_unreadable"`) {
		t.Fatalf("no warning for an undecodable registry: %s", logs)
	}
}

func TestLoadInstancesLogsUnreadableExpiry(t *testing.T) {
	logs := captureLogs(t)
	content := `{"instances": {"x": {"lease": {"expires_at": "soon"}, "manifest": {}}}}`
	got := loadInstances(writeInstances(t, content), time.Now())
	if len(got) != 1 || got[0].Active {
		t.Fatalf("an unreadable expiry must not count as an active lease: %+v", got)
	}
	if !strings.Contains(logs.String(), `"instance":"x"`) {
		t.Fatalf("no warning for the unreadable expiry: %s", logs)
	}
}

func TestInstanceLinesRenderLeaseSlotsAndCapabilities(t *testing.T) {
	now := time.Date(2026, 10, 22, 12, 0, 0, 0, time.UTC)
	lines := instanceLines(loadInstances(writeInstances(t, instancesFixture), now), 200)
	if len(lines) != 3 {
		t.Fatalf("want header and two rows, got %d: %q", len(lines), lines)
	}
	if !strings.Contains(lines[0], "INSTANCE") || !strings.Contains(lines[0], "CAPABILITIES") {
		t.Fatalf("header: %q", lines[0])
	}
	text := strings.Join(lines, "\n")
	for _, want := range []string{"mac", "expired", "darwin/arm64", "0/2", "pi5", "active", "linux/arm64", "3/4", "build:c, hw:rp2350-probe"} {
		if !strings.Contains(text, want) {
			t.Fatalf("panel misses %q:\n%s", want, text)
		}
	}
}

func TestInstanceLinesAreEmptyWithoutInstances(t *testing.T) {
	if lines := instanceLines(nil, 80); lines != nil {
		t.Fatalf("instances shown without any: %q", lines)
	}
}

func TestInstanceLinesBoundTheTableAndSayHowManyRemain(t *testing.T) {
	var many []InstanceInfo
	for _, n := range []string{"a", "b", "c", "d", "e", "f"} {
		many = append(many, InstanceInfo{Name: n, Active: true, OS: "linux", Arch: "amd64"})
	}
	lines := instanceLines(many, 120)
	if len(lines) != 1+maxInstanceRows+1 {
		t.Fatalf("want header, %d rows and a summary, got %d lines", maxInstanceRows, len(lines))
	}
	if last := lines[len(lines)-1]; !strings.Contains(last, "2 more instances") {
		t.Fatalf("summary line: %q", last)
	}
}

func TestInstanceLinesShowUnknownPlatformAndNoCapabilities(t *testing.T) {
	lines := instanceLines([]InstanceInfo{{Name: "bare", Active: true}}, 120)
	text := strings.Join(lines, "\n")
	if !strings.Contains(text, "unknown") || !strings.Contains(text, " -") {
		t.Fatalf("missing platform or capabilities placeholder:\n%s", text)
	}
}
