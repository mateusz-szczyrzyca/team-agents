package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// mirrorFixture adds a remote task sharing its raw ID with an existing local task.
func mirrorFixture(t *testing.T, home string) string {
	t.Helper()
	remoteJobs(t, home)
	dir := filepath.Join(home, "bus", "remote", "node-a", "T1")
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker-1","TYPE":"codex","MODEL":"example-model","NOTE":"remote note","STATUS":"running","RUN":"3","CREATED":"2026-10-23T08:00:00Z","RUN_START":"2026-10-23T09:00:00Z","JOB_ID":"remote-1"}`)
	writeFile(t, filepath.Join(dir, "activity.jsonl"), `{"ts":"10:00:07","k":"cmd","t":"mirrored activity"}`+"\n")
	return dir
}

// TestSnapshotLoadsMirroredTask verifies identity, metadata, job enrichment, and local coexistence.
func TestSnapshotLoadsMirroredTask(t *testing.T) {
	home := fixture(t)
	dir := mirrorFixture(t, home)
	snap := loadSnapshot(home, filepath.Join(home, "bus"))
	task := snap.All["T1@node-a"]
	if task == nil {
		t.Fatal("mirrored task missing")
	}
	if task.ID != "T1@node-a" || task.Node != "node-a" || task.Dir != dir || task.Agent != "worker-1" || task.Kind != "worker" {
		t.Fatalf("identity: %+v", task)
	}
	if task.Type != "codex" || task.Model != "example-model" || task.Note != "remote note" || task.Run != 3 || task.Status != "running" || task.JobID != "remote-1" {
		t.Fatalf("metadata: %+v", task)
	}
	if task.Repo != "demo" || task.Branch != "agents/worker-1/x" || task.Created.IsZero() || task.RunStart.IsZero() || task.Last.Text != "mirrored activity" {
		t.Fatalf("enrichment: %+v", task)
	}
	if snap.All["T1"] == nil || task.NodeOffline || !task.Active() {
		t.Fatal("local task or active online state lost")
	}
	row := makeRow(snap, task)
	if row.Name != "worker-1@node-a" || row.Link != "/task/T1@node-a" || row.Running || strings.Contains(row.StatusLabel, "dead") {
		t.Fatalf("remote row: %+v", row)
	}
}

// TestMirrorDashboardDeduplicatesJob checks that the mirror replaces the job summary row.
func TestMirrorDashboardDeduplicatesJob(t *testing.T) {
	home := fixture(t)
	mirrorFixture(t, home)
	server, _ := testServer(t, home)
	rows := server.dashboard(server.snapshot()).Rows
	found := 0
	for _, row := range rows {
		if row.ID == "remote-1" {
			t.Fatal("duplicate job summary")
		}
		if row.ID == "T1@node-a" {
			found++
		}
	}
	if found != 1 {
		t.Fatalf("mirror rows: %d", found)
	}
}

// TestMirrorOfflineKeepsLastKnownStatus checks configured heartbeat expiry without changing lifecycle.
func TestMirrorOfflineKeepsLastKnownStatus(t *testing.T) {
	home := fixture(t)
	mirrorFixture(t, home)
	server, _ := testServer(t, home)
	server.cfg.nodeTTL = time.Nanosecond
	snap := server.snapshot()
	task := snap.All["T1@node-a"]
	if task == nil {
		t.Fatal("mirrored task missing")
	}
	row := makeRow(snap, task)
	if !task.NodeOffline || !task.Active() || task.Status != "running" || !strings.Contains(row.StatusLabel, "node offline") || !strings.Contains(row.StatusLabel, "running") {
		t.Fatalf("offline row: %+v task: %+v", row, task)
	}
}

// TestMirrorScanSkipsIncompleteAndCorruptDirectories keeps other tasks visible during partial updates.
func TestMirrorScanSkipsIncompleteAndCorruptDirectories(t *testing.T) {
	home := fixture(t)
	mirrorFixture(t, home)
	writeFile(t, filepath.Join(home, "bus", "remote", "node-b", "T1", "metadata.json"), `{"AGENT":"second","STATUS":"done","FINISHED":"2026-10-23T10:00:00Z"}`)
	writeFile(t, filepath.Join(home, "bus", "remote", "node-b", "bad", "metadata.json"), `{`)
	if err := os.MkdirAll(filepath.Join(home, "bus", "remote", "node-b", "pending"), 0o750); err != nil {
		t.Fatal(err)
	}
	snap := loadSnapshot(home, filepath.Join(home, "bus"))
	second := snap.All["T1@node-b"]
	if snap.All["T1@node-a"] == nil || second == nil || second.Finished.IsZero() || !second.NodeOffline {
		t.Fatalf("node identities or timestamps lost: %+v", second)
	}
	if snap.All["bad@node-b"] != nil || snap.All["pending@node-b"] != nil {
		t.Fatal("incomplete metadata became tasks")
	}
}
