// Hub maintenance: periodic state backups and the reaper that reclaims the work of instances whose lease expired.
// Both run in the hub process. Each holds the bus lock for its whole operation, so it sees and writes one
// coherent state: the reaper reads the instance registry and the team state together, and a backup copies the
// files of one snapshot.
package main

import (
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

const (
	// backupKeep is the number of newest backups kept; older backup directories are deleted after each backup.
	backupKeep = 10
	// backupPrefix starts every backup directory name, so pruning never touches other files of the bus directory.
	backupPrefix = "backup-"
	// backupTimeLayout formats the UTC time of a backup in its directory name; the names sort chronologically.
	backupTimeLayout = "20060102-150405"
	// backupTempPrefix starts the temporary directory a backup is written into before it is renamed.
	backupTempPrefix = ".backup-tmp-"
	// defaultBackupInterval is the backup period when HUB_BACKUP_MINUTES is not set.
	defaultBackupInterval = 30 * time.Minute
	// reapInterval is how often the reaper checks the instance leases.
	reapInterval = 30 * time.Second
)

// backupFiles are the bus files of the snapshot, besides the replay ledger directory. commands.json and
// instances.json belong to the hub; restoring only state.json could re-enable commands that were already run.
var backupFiles = []string{"state.json", "messages.jsonl", "instances.json", "commands.json"}

// writeBackup writes one snapshot of the hub state into a new backup directory under dir, then deletes the backups
// beyond the newest backupKeep. It returns the path of the new backup. A failed pruning is logged, not returned,
// because the new backup exists already.
func writeBackup(b *Bus, dir string, at time.Time) (string, *StorageError) {
	var target string
	var failed *StorageError
	serr := b.locked(func(*State) {
		target, failed = copyBackup(b, dir, at)
	})
	if serr != nil {
		return "", serr
	}
	if failed != nil {
		return "", failed
	}
	if pruneErr := pruneBackups(dir); pruneErr != nil {
		warnSideEffect("prune hub backups", pruneErr, "dir", dir)
	}
	return target, nil
}

// copyBackup writes one backup directory; the caller holds the bus lock. The files go into a temporary directory
// that is renamed into place when complete, so a backup directory either exists whole or not at all.
func copyBackup(b *Bus, dir string, at time.Time) (string, *StorageError) {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", storageErr(err, "cannot create the backup directory %s", dir)
	}
	final := filepath.Join(dir, backupPrefix+at.UTC().Format(backupTimeLayout))
	if _, err := os.Stat(final); err == nil {
		return "", storageErr(valueErr("%s exists", final), "a backup with this time already exists; try again later")
	} else if !errors.Is(err, fs.ErrNotExist) {
		return "", storageErr(err, "cannot check the backup %s", final)
	}
	tmp, err := os.MkdirTemp(dir, backupTempPrefix)
	if err != nil {
		return "", storageErr(err, "cannot create a temporary backup directory in %s", dir)
	}
	if serr := copyHubState(b, tmp); serr != nil {
		removeBackupTemp(tmp)
		return "", serr
	}
	if err := os.Rename(tmp, final); err != nil {
		removeBackupTemp(tmp)
		return "", storageErr(err, "cannot publish the backup %s", final)
	}
	return final, nil
}

// removeBackupTemp deletes a failed temporary backup; a leftover directory is reported but harmless.
func removeBackupTemp(tmp string) {
	if err := os.RemoveAll(tmp); err != nil {
		warnSideEffect("remove temporary backup", storageErr(err, "cannot remove %s", tmp))
	}
}

// copyHubState copies the snapshot files and the replay ledger of the bus directory into dst. A file that does not
// exist yet (for example commands.json before the first command) is skipped, not an error.
func copyHubState(b *Bus, dst string) *StorageError {
	for _, name := range backupFiles {
		if err := copyFile(b.path(name), filepath.Join(dst, name)); err != nil && !errors.Is(err, fs.ErrNotExist) {
			return storageErr(err, "cannot back up %s", name)
		}
	}
	return copyTree(filepath.Join(b.dir, "replay"), filepath.Join(dst, "replay"))
}

// copyFile copies one regular file with owner-only permissions: backups hold lease credentials.
func copyFile(src, dst string) *StorageError {
	data, err := os.ReadFile(src) //nolint:gosec // G304: src is a file of the trusted bus directory or its replay ledger
	if err != nil {
		return storageErr(err, "cannot read backup source %s", src)
	}
	if err := os.WriteFile(dst, data, 0o600); err != nil { //nolint:gosec // G703: dst is inside a backup directory the hub created
		return storageErr(err, "cannot write backup destination %s", dst)
	}
	return nil
}

// copyTree copies the files of a directory tree; a missing source is an empty tree.
func copyTree(src, dst string) *StorageError {
	info, err := os.Stat(src)
	if errors.Is(err, fs.ErrNotExist) {
		return nil
	}
	if err != nil {
		return storageErr(err, "cannot read %s", src)
	}
	if !info.IsDir() {
		return storageErr(valueErr("%s is not a directory", src), "cannot back up %s", src)
	}
	// WalkDir requires error; the callback returns filesystem errors or a *StorageError.
	walkErr := filepath.WalkDir(src, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(src, p)
		if err != nil {
			return err
		}
		target := filepath.Join(dst, rel)
		if d.IsDir() {
			return os.MkdirAll(target, 0o700)
		}
		return copyFile(p, target).asError()
	})
	if walkErr != nil {
		return storageErr(walkErr, "cannot back up %s", src)
	}
	return nil
}

// pruneBackups deletes all but the newest backupKeep backup directories in dir.
func pruneBackups(dir string) *StorageError {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return storageErr(err, "cannot list the backup directory %s", dir)
	}
	var names []string
	for _, e := range entries {
		if e.IsDir() && strings.HasPrefix(e.Name(), backupPrefix) {
			names = append(names, e.Name())
		}
	}
	sort.Strings(names)
	for i := 0; i < len(names)-backupKeep; i++ {
		if err := os.RemoveAll(filepath.Join(dir, names[i])); err != nil {
			return storageErr(err, "cannot delete the old backup %s", names[i])
		}
	}
	return nil
}

// runBackups writes a backup every interval until stop is closed. A failed backup is logged; the next one is tried
// at the next tick.
func runBackups(b *Bus, dir string, interval time.Duration, stop <-chan struct{}) {
	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	for {
		select {
		case <-stop:
			return
		case at := <-ticker.C:
			target, serr := writeBackup(b, dir, at)
			if serr != nil {
				warnSideEffect("write hub backup", serr, "dir", dir)
				continue
			}
			logInfo(LogHubBackup, "path", target)
		}
	}
}

// runReaper checks the instance leases every reapInterval until stop is closed.
func runReaper(b *Bus, reg *HubInstanceRegistry, stop <-chan struct{}) {
	ticker := time.NewTicker(reapInterval)
	defer ticker.Stop()
	for {
		select {
		case <-stop:
			return
		case <-ticker.C:
			if serr := reapUnreachable(b, reg); serr != nil {
				warnSideEffect("reclaim unreachable instances", serr)
			}
		}
	}
}

// reapUnreachable reclaims the work of every instance whose lease has expired: its members become unreachable,
// the board items they claimed return to the board, and each affected team is told in one message. Because the
// registry and the teams are read under one lock, an instance that registers meanwhile is not reclaimed.
func reapUnreachable(b *Bus, reg *HubInstanceRegistry) *StorageError {
	var failed *StorageError
	serr := b.locked(func(st *State) {
		store, err := reg.load()
		if err != nil {
			failed = err
			return
		}
		expired := expiredInstances(store, reg.now())
		if len(expired) == 0 {
			return
		}
		for _, id := range sortedNames(st.Teams) {
			reclaimTeam(b, st, st.Teams[id], expired)
		}
	})
	if serr != nil {
		return serr
	}
	return failed
}

// expiredInstances returns the registered instances whose lease deadline has passed. The deadline already holds
// the TTL: every heartbeat moves it one TTL ahead, so an instance is reclaimed one TTL after its last heartbeat.
func expiredInstances(store *hubInstanceState, now time.Time) map[string]bool {
	expired := map[string]bool{}
	for name, rec := range store.Instances {
		if !now.Before(rec.Lease.ExpiresAt.Time) {
			expired[name] = true
		}
	}
	return expired
}

// instanceOfName returns the instance of a qualified name such as "codex-1@mac". Unqualified names belong to a
// single-instance installation and yield "", which is never an expired instance.
func instanceOfName(name string) string {
	if i := strings.LastIndex(name, "@"); i > 0 {
		return name[i+1:]
	}
	return ""
}

// reclaimTeam applies the reclamation to one team (the caller holds the lock) and posts one notice to its
// coordinator, or to the whole team when the coordinator is on an expired instance too. Nothing is posted when
// nothing changed, so repeated scans are quiet.
func reclaimTeam(b *Bus, st *State, t *Team, expired map[string]bool) {
	instances := map[string]bool{}
	var members, returned []string
	for _, name := range sortedNames(t.Members) {
		m := t.Members[name]
		if !expired[instanceOfName(name)] || m.State == MemberDone || m.State == MemberExcluded || m.State == MemberUnreachable {
			continue
		}
		m.State = MemberUnreachable
		members = append(members, name)
		instances[instanceOfName(name)] = true
	}
	now := Now()
	for _, id := range sortedNames(t.Board) {
		it := t.Board[id]
		if it.Status != BoardClaimed || !expired[instanceOfName(it.Assignee)] {
			continue
		}
		instance := instanceOfName(it.Assignee)
		instances[instance] = true
		it.Details += "\n\n[Returned to the board: " + it.Assignee + " was unreachable (instance " + instance + ") at " + now.String() + ".]"
		it.Status, it.Assignee, it.Updated = BoardTodo, "", now
		returned = append(returned, id)
	}
	if len(members) == 0 && len(returned) == 0 {
		return
	}
	if len(returned) > 0 {
		t.bumpBoard()
	}
	logWarn(LogInstanceReclaimed, "team", t.ID, "members", len(members), "board_items", len(returned))
	ev := EventInstanceUnreachable{Instances: sortedNames(instances), Members: members, Returned: returned}
	to := teamRecipient(t.ID)
	if t.Lead != "" && !expired[instanceOfName(t.Lead)] {
		to = t.Lead
	}
	if _, serr := b.post(st, "agentbus", to, ev); serr != nil {
		warnSideEffect("announce unreachable instance", serr, "team", t.ID)
	}
}

// sortedNames returns the keys of a map in ascending order, so that reclamation and notices are deterministic.
func sortedNames[V any](m map[string]V) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}
