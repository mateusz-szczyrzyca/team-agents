package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// seedHubFiles writes the snapshot files and one replay ledger entry into the bus directory.
func seedHubFiles(t *testing.T, b *Bus) {
	t.Helper()
	mustWith(t, b, func(*State) {}) // saves state.json
	files := map[string]string{
		"messages.jsonl":  "{\"id\":1}\n",
		"instances.json":  `{"version":1}`,
		"commands.json":   `{"version":1}`,
		"replay/key.json": `{"done":true}`,
	}
	for name, body := range files {
		if err := os.MkdirAll(filepath.Dir(b.path(name)), 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(b.path(name), []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
	}
}

// TestBackupSnapshotAndRotation verifies that a backup copies the snapshot and that only the newest backupKeep
// backups survive.
func TestBackupSnapshotAndRotation(t *testing.T) {
	b := newBus(t)
	seedHubFiles(t, b)
	dir := filepath.Join(t.TempDir(), "backups")
	start := time.Date(2026, 10, 22, 16, 0, 0, 0, time.UTC)
	var first, last string
	for i := 0; i < backupKeep+2; i++ {
		target, serr := writeBackup(b, dir, start.Add(time.Duration(i)*time.Minute))
		if serr != nil {
			t.Fatal(serr)
		}
		if i == 0 {
			first = target
		}
		last = target
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != backupKeep {
		t.Fatalf("kept %d backups, want %d", len(entries), backupKeep)
	}
	for _, e := range entries {
		if !strings.HasPrefix(e.Name(), backupPrefix) {
			t.Fatalf("unexpected entry %q in the backup directory (temporary directories must be renamed)", e.Name())
		}
	}
	if _, err := os.Stat(first); !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("the oldest backup %s was not pruned (stat: %v)", first, err)
	}
	if filepath.Base(last) != "backup-20261022-161100" {
		t.Fatalf("newest backup named %q", filepath.Base(last))
	}
	for name, want := range map[string]string{
		"instances.json":  `{"version":1}`,
		"commands.json":   `{"version":1}`,
		"messages.jsonl":  "{\"id\":1}\n",
		"replay/key.json": `{"done":true}`,
	} {
		data, err := os.ReadFile(filepath.Join(last, name)) //nolint:gosec // G304: the test reads its own backup directory
		if err != nil {
			t.Fatalf("backup lacks %s: %v", name, err)
		}
		if string(data) != want {
			t.Fatalf("backup of %s = %q, want %q", name, data, want)
		}
	}
	if _, err := os.Stat(filepath.Join(last, "state.json")); err != nil {
		t.Fatalf("backup lacks state.json: %v", err)
	}
}

// TestBackupSameTimeFails verifies that a second backup with the same time does not overwrite the first.
func TestBackupSameTimeFails(t *testing.T) {
	b := newBus(t)
	seedHubFiles(t, b)
	dir := t.TempDir()
	at := time.Date(2026, 10, 22, 16, 0, 0, 0, time.UTC)
	if _, serr := writeBackup(b, dir, at); serr != nil {
		t.Fatal(serr)
	}
	if _, serr := writeBackup(b, dir, at); serr == nil {
		t.Fatal("a second backup with the same time was accepted")
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 {
		t.Fatalf("expected the first backup only, found %d entries", len(entries))
	}
}

// TestBackupWithoutCommandJournal verifies that files which do not exist yet are skipped.
func TestBackupWithoutCommandJournal(t *testing.T) {
	b := newBus(t)
	mustWith(t, b, func(*State) {})
	dir := t.TempDir()
	target, serr := writeBackup(b, dir, time.Date(2026, 10, 22, 16, 0, 0, 0, time.UTC))
	if serr != nil {
		t.Fatal(serr)
	}
	if _, err := os.Stat(filepath.Join(target, "state.json")); err != nil {
		t.Fatalf("backup lacks state.json: %v", err)
	}
	if _, err := os.Stat(filepath.Join(target, "commands.json")); !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("commands.json should be absent from the backup (stat: %v)", err)
	}
}

// TestBackupIntervalFromEnv verifies the parsing of HUB_BACKUP_MINUTES.
func TestBackupIntervalFromEnv(t *testing.T) {
	cases := map[string]time.Duration{
		"":     0,
		"30":   30 * time.Minute,
		"1440": 24 * time.Hour,
		"0":    -1,
		"1441": -1,
		"x":    -1,
	}
	for raw, want := range cases {
		if got := backupIntervalFromEnv(raw); got != want {
			t.Errorf("backupIntervalFromEnv(%q) = %v, want %v", raw, got, want)
		}
	}
}

// TestHubServeInvalidBackupInterval verifies that an unusable HUB_BACKUP_MINUTES stops the hub before it listens.
func TestHubServeInvalidBackupInterval(t *testing.T) {
	cfg := &HubConfig{
		Port:             7777,
		Auth:             HubAuthModeTailscale,
		AllowedInstances: []string{"node1"},
		Dir:              t.TempDir(),
		BackupInterval:   -time.Minute,
	}
	if err := serveHub(cfg); err == nil || err.Cause != HubServeInvalidConfig {
		t.Fatalf("expected HubServeInvalidConfig, got %v", err)
	}
}

// reaperFixture returns a bus whose instance "mac" holds a lease registered at the returned clock's time, and the
// clock itself, which the test moves forward.
func reaperFixture(t *testing.T) (*Bus, *HubInstanceRegistry, *time.Time) {
	t.Helper()
	b := newBus(t)
	clock := time.Date(2026, 10, 22, 16, 0, 0, 0, time.UTC)
	reg := &HubInstanceRegistry{Bus: b, TTL: 180 * time.Second, Now: func() time.Time { return clock }}
	if _, err := reg.Register("mac", "node-mac", instanceRegisterFixture("mac", "session-mac")); err != nil {
		t.Fatal(err)
	}
	return b, reg, &clock
}

// reaperTeam creates team T: claude-1 (local, coordinator) and codex-1@mac and gemini-1@mac on instance mac.
// B1 is claimed by codex-1@mac, B2 by claude-1.
func reaperTeam(t *testing.T, b *Bus) {
	t.Helper()
	createTeam(t, b, "T", map[string][3]string{
		"claude-1":     {"normal", "anthropic", "active"},
		"codex-1@mac":  {"normal", "openai", "active"},
		"gemini-1@mac": {"normal", "google", "done"},
	})
	mustWith(t, b, func(st *State) {
		team := st.Teams["T"]
		team.Lead = "claude-1"
		team.Board = map[string]*BoardItem{
			"B1": {ID: "B1", Title: "build", Details: "build it", Level: LevelNormal, Status: BoardClaimed, Creator: "claude-1", Assignee: "codex-1@mac"},
			"B2": {ID: "B2", Title: "review", Details: "review it", Level: LevelNormal, Status: BoardClaimed, Creator: "codex-1@mac", Assignee: "claude-1"},
		}
	})
}

// teamState loads the state and returns team T.
func teamState(t *testing.T, b *Bus) *Team {
	t.Helper()
	st, serr := b.load()
	if serr != nil {
		t.Fatal(serr)
	}
	return st.Teams["T"]
}

// unreachableNotices returns the instance_unreachable messages on the bus.
func unreachableNotices(t *testing.T, b *Bus) []Message {
	t.Helper()
	f, err := os.Open(b.path("messages.jsonl"))
	if err != nil {
		if errors.Is(err, fs.ErrNotExist) {
			return nil
		}
		t.Fatal(err)
	}
	defer func() {
		if err := f.Close(); err != nil {
			t.Fatal(err)
		}
	}()
	var out []Message
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		var m Message
		if err := json.Unmarshal(sc.Bytes(), &m); err != nil {
			t.Fatal(err)
		}
		if m.Kind == KindInstanceUnreachable {
			out = append(out, m)
		}
	}
	if err := sc.Err(); err != nil {
		t.Fatal(err)
	}
	return out
}

// TestReapWaitsForLeaseExpiry verifies that a live lease reclaims nothing.
func TestReapWaitsForLeaseExpiry(t *testing.T) {
	b, reg, clock := reaperFixture(t)
	reaperTeam(t, b)
	*clock = clock.Add(179 * time.Second)
	if serr := reapUnreachable(b, reg); serr != nil {
		t.Fatal(serr)
	}
	team := teamState(t, b)
	if team.Members["codex-1@mac"].State != MemberActive || team.Board["B1"].Status != BoardClaimed {
		t.Fatalf("a live instance was reclaimed: %+v / %+v", team.Members["codex-1@mac"], team.Board["B1"])
	}
	if n := len(unreachableNotices(t, b)); n != 0 {
		t.Fatalf("%d notices for a live instance", n)
	}
}

// TestReapReclaimsExpiredInstance verifies that an expired instance makes its members unreachable, returns their
// board items and informs the coordinator once.
func TestReapReclaimsExpiredInstance(t *testing.T) {
	b, reg, clock := reaperFixture(t)
	reaperTeam(t, b)
	*clock = clock.Add(181 * time.Second)
	if serr := reapUnreachable(b, reg); serr != nil {
		t.Fatal(serr)
	}
	team := teamState(t, b)
	if got := team.Members["codex-1@mac"].State; got != MemberUnreachable {
		t.Fatalf("codex-1@mac is %s, want unreachable", got)
	}
	if got := team.Members["gemini-1@mac"].State; got != MemberDone {
		t.Fatalf("a finished member changed to %s", got)
	}
	if got := team.Members["claude-1"].State; got != MemberActive {
		t.Fatalf("a local member changed to %s", got)
	}
	b1 := team.Board["B1"]
	if b1.Status != BoardTodo || b1.Assignee != "" || !strings.Contains(b1.Details, "Returned to the board") {
		t.Fatalf("B1 was not returned with a note: %+v", b1)
	}
	if b2 := team.Board["B2"]; b2.Status != BoardClaimed || b2.Assignee != "claude-1" {
		t.Fatalf("B2 of a local member changed: %+v", b2)
	}
	if team.BoardRev != 1 {
		t.Fatalf("board revision %d, want 1", team.BoardRev)
	}
	notices := unreachableNotices(t, b)
	if len(notices) != 1 {
		t.Fatalf("%d notices, want 1", len(notices))
	}
	if notices[0].To != "claude-1" || !strings.Contains(notices[0].Text, "codex-1@mac") || !strings.Contains(notices[0].Text, "B1") {
		t.Fatalf("notice to %q: %s", notices[0].To, notices[0].Text)
	}
}

// TestReapIsIdempotent verifies that a second scan of the same expired instance changes nothing.
func TestReapIsIdempotent(t *testing.T) {
	b, reg, clock := reaperFixture(t)
	reaperTeam(t, b)
	*clock = clock.Add(181 * time.Second)
	for i := 0; i < 2; i++ {
		if serr := reapUnreachable(b, reg); serr != nil {
			t.Fatal(serr)
		}
	}
	if n := len(unreachableNotices(t, b)); n != 1 {
		t.Fatalf("%d notices after two scans, want 1", n)
	}
	if rev := teamState(t, b).BoardRev; rev != 1 {
		t.Fatalf("board revision %d after two scans, want 1", rev)
	}
}

// TestReapKeepsReregisteredInstance verifies that an instance which registers again after its lease expired is not
// reclaimed by the scan that follows.
func TestReapKeepsReregisteredInstance(t *testing.T) {
	b, reg, clock := reaperFixture(t)
	reaperTeam(t, b)
	*clock = clock.Add(181 * time.Second)
	if _, err := reg.Register("mac", "node-mac", instanceRegisterFixture("mac", "session-mac-2")); err != nil {
		t.Fatal(err)
	}
	if serr := reapUnreachable(b, reg); serr != nil {
		t.Fatal(serr)
	}
	team := teamState(t, b)
	if team.Members["codex-1@mac"].State != MemberActive || team.Board["B1"].Status != BoardClaimed {
		t.Fatal("an instance with a live lease was reclaimed")
	}
}
