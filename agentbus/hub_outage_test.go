package main

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

// outageAt returns the record at AGENTS_HOME home, failing the test when it cannot be read.
func outageAt(t *testing.T, home string) hubOutage {
	t.Helper()
	state, serr := readHubOutage(home)
	if serr != nil {
		t.Fatal(serr)
	}
	return state
}

// TestHubOutageAccumulatesClosedOutages verifies that the total grows by each closed outage and that repeated
// failures inside one outage do not move its start.
func TestHubOutageAccumulatesClosedOutages(t *testing.T) {
	home := t.TempDir()
	start := time.Date(2026, 10, 22, 16, 0, 0, 0, time.UTC)
	must := func(reachable bool, at time.Time) {
		t.Helper()
		if serr := recordHubReachability(home, reachable, at); serr != nil {
			t.Fatal(serr)
		}
	}
	must(false, start)
	must(false, start.Add(10*time.Second))
	if got := outageAt(t, home); got.OpenSince != start.Unix() || got.TotalSeconds != 0 {
		t.Fatalf("repeated failure moved the outage start: %+v", got)
	}
	must(true, start.Add(30*time.Second))
	if got := outageAt(t, home); got.OpenSince != 0 || got.TotalSeconds != 30 {
		t.Fatalf("recovery did not close the outage: %+v", got)
	}
	must(false, start.Add(100*time.Second))
	must(true, start.Add(110*time.Second))
	if got := outageAt(t, home); got.TotalSeconds != 40 {
		t.Fatalf("second outage not added: %+v", got)
	}
}

// TestHubOutageReachableWithoutOutageWritesNothing verifies that normal operation leaves no file behind.
func TestHubOutageReachableWithoutOutageWritesNothing(t *testing.T) {
	home := t.TempDir()
	if serr := recordHubReachability(home, true, time.Now()); serr != nil {
		t.Fatal(serr)
	}
	if _, err := os.Stat(filepath.Join(home, hubOutageFile)); !os.IsNotExist(err) {
		t.Fatalf("a reachable hub wrote %s: %v", hubOutageFile, err)
	}
}

// TestHubOutageEmptyHomeRecordsNothing covers servers without a local AGENTS_HOME.
func TestHubOutageEmptyHomeRecordsNothing(t *testing.T) {
	if serr := recordHubReachability("", false, time.Now()); serr != nil {
		t.Fatal(serr)
	}
}

// TestHubOutageCorruptRecordIsStorageFailure verifies that an unreadable record is reported, not overwritten.
func TestHubOutageCorruptRecordIsStorageFailure(t *testing.T) {
	home := t.TempDir()
	if err := os.WriteFile(filepath.Join(home, hubOutageFile), []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	if serr := recordHubReachability(home, false, time.Now()); serr == nil {
		t.Fatal("a corrupt outage record was accepted")
	}
	data, err := os.ReadFile(filepath.Join(home, hubOutageFile)) //nolint:gosec // G304: fixed fixture filename under t.TempDir()
	if err != nil || string(data) != "{not json" {
		t.Fatalf("a corrupt record was overwritten: %q %v", data, err)
	}
}

// TestHubReachedOnlyUnavailableIsUnreachable verifies that a typed refusal from a reachable hub is not an outage.
func TestHubReachedOnlyUnavailableIsUnreachable(t *testing.T) {
	if hubReached(opErr(HubClientUnavailable, "hub down").asError()) {
		t.Fatal("an Unavailable hub call was counted as reachable")
	}
	if !hubReached(nil) {
		t.Fatal("a successful hub call was counted as an outage")
	}
	if !hubReached(opErr(CommandEnqueueInvalidParams, "bad params").asError()) {
		t.Fatal("a typed refusal from the hub was counted as an outage")
	}
}
