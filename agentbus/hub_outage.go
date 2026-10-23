package main

import (
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"syscall"
	"time"
)

// hubOutageFile is the instance-wide hub outage record under AGENTS_HOME. bin/agent reads it (hub_outage_secs) to
// leave outage time out of the hang watchdog.
const hubOutageFile = "hub-outage.json"

// hubOutageLock serializes updates of hubOutageFile between the node daemon and the serve processes of one instance.
const hubOutageLock = "hub-outage.lock"

// hubOutage is the content of hubOutageFile. Times are unix seconds. The file is written only when reachability
// changes, so repeated failures never move the start of an outage.
type hubOutage struct {
	TotalSeconds int64 `json:"total_seconds"` // length of all closed outages, in seconds
	OpenSince    int64 `json:"open_since"`    // start of the current outage; 0 while the hub is reachable
}

// hubReached reports whether a hub call proves that the hub answered. Only the Unavailable class means it did not
// (no connection, no result within the retry budget, or a hub that could not serve the request); any other error
// is a typed answer from a reachable hub.
func hubReached(err error) bool {
	return !errors.Is(err, ErrUnavailable)
}

// recordHubReachability opens an outage when the hub stops answering and closes it, adding its length to
// TotalSeconds, at the first answer after it. An empty home records nothing: a server without a local AGENTS_HOME
// (for instance one that runs inside the hub) has no instance to report for. Failures are returned, and the callers
// log them as side effects: the hub operation itself has already completed or failed on its own terms.
func recordHubReachability(home string, reachable bool, at time.Time) *StorageError {
	if home == "" {
		return nil
	}
	lock, err := os.OpenFile(filepath.Join(home, hubOutageLock), os.O_CREATE|os.O_RDWR, 0o600) //nolint:gosec // G304: fixed lock filename under operator-selected AGENTS_HOME
	if err != nil {
		return storageErr(err, "cannot open the hub outage lock")
	}
	defer func() {
		if cerr := lock.Close(); cerr != nil {
			warnSideEffect("close hub outage lock", storageErr(cerr, "cannot close the hub outage lock"))
		}
	}()
	if err := syscall.Flock(int(lock.Fd()), syscall.LOCK_EX); err != nil {
		return storageErr(err, "cannot lock the hub outage record")
	}
	state, serr := readHubOutage(home)
	if serr != nil {
		return serr
	}
	now := at.Unix()
	switch {
	case reachable && state.OpenSince != 0:
		if now > state.OpenSince {
			state.TotalSeconds += now - state.OpenSince
		}
		state.OpenSince = 0
	case !reachable && state.OpenSince == 0:
		state.OpenSince = now
	default:
		return nil
	}
	return writeHubOutage(home, state)
}

// reachabilityObserver returns the hub client's per-attempt observer for one instance. It records every attempt as it
// happens, so an outage counts from its first failed attempt, not from the end of the retry budget. Failures are
// side effects: the call itself is not affected.
func reachabilityObserver(home, instance string) func(bool) {
	return func(reached bool) {
		if serr := recordHubReachability(home, reached, time.Now()); serr != nil {
			warnSideEffect("record hub reachability", serr, "instance", instance)
		}
	}
}

// readHubOutage loads the record; a missing file means no outage has been recorded yet.
func readHubOutage(home string) (hubOutage, *StorageError) {
	var state hubOutage
	data, err := os.ReadFile(filepath.Join(home, hubOutageFile)) //nolint:gosec // G304: fixed filename under operator-selected AGENTS_HOME
	if errors.Is(err, fs.ErrNotExist) {
		return state, nil
	}
	if err != nil {
		return state, storageErr(err, "cannot read %s", hubOutageFile)
	}
	if err := json.Unmarshal(data, &state); err != nil {
		return state, storageErr(err, "cannot decode %s", hubOutageFile)
	}
	return state, nil
}

// writeHubOutage replaces the record atomically, so that a reader (the watchdog) never sees a partial file.
func writeHubOutage(home string, state hubOutage) *StorageError {
	data, err := json.Marshal(state)
	if err != nil {
		return storageErr(err, "cannot encode %s", hubOutageFile)
	}
	path := filepath.Join(home, hubOutageFile)
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, append(data, '\n'), 0o600); err != nil {
		return storageErr(err, "cannot write %s", hubOutageFile)
	}
	if err := os.Rename(tmp, path); err != nil {
		return storageErr(err, "cannot replace %s", hubOutageFile)
	}
	return nil
}
