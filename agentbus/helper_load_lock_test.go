package main

import (
	"errors"
	"os"
	"syscall"
	"testing"
)

// TestOwnedReceiptLoadSerializesWithTransactions pauses a real file read so the
// competing transaction cannot be accidentally ordered by goroutine scheduling.
func TestOwnedReceiptLoadSerializesWithTransactions(t *testing.T) {
	b, cmd := ownedFixture(t)
	if _, err := b.helperOwned(cmd); err != nil {
		t.Fatal(err)
	}
	st, serr := b.load()
	if serr != nil {
		t.Fatal(serr)
	}
	data, err := os.ReadFile(b.path("state.json"))
	if err != nil {
		t.Fatal(err)
	}
	if _, serr := b.post(st, "human", "all", EventChat{Body: "pending receipt cursor"}); serr != nil {
		t.Fatal(serr)
	}
	if err := os.Remove(b.path("state.json")); err != nil {
		t.Fatal(err)
	}
	if err := syscall.Mkfifo(b.path("state.json"), 0o600); err != nil {
		t.Fatal(err)
	}
	loaded := make(chan *StorageError, 1)
	go func() { _, err := (&Bus{dir: b.dir}).load(); loaded <- err }()
	// Opening the writer rendezvous with load's reader; its state read is now paused.
	writer, err := os.OpenFile(b.path("state.json"), os.O_WRONLY, 0)
	if err != nil {
		t.Fatal(err)
	}
	if serr := b.save(st); serr != nil {
		t.Fatal(serr)
	}
	probe, err := os.OpenFile(b.path("bus.lock"), os.O_RDWR, 0)
	if err != nil {
		t.Fatal(err)
	}
	lockErr := syscall.Flock(int(probe.Fd()), syscall.LOCK_EX|syscall.LOCK_NB)
	if lockErr == nil {
		if err := syscall.Flock(int(probe.Fd()), syscall.LOCK_UN); err != nil {
			t.Fatal(err)
		}
	} else if !errors.Is(lockErr, syscall.EWOULDBLOCK) {
		t.Fatal(lockErr)
	}
	if err := probe.Close(); err != nil {
		t.Fatal(err)
	}
	changed := make(chan *StorageError, 1)
	go func() { changed <- (&Bus{dir: b.dir}).locked(func(current *State) { current.NextHelperID = 77 }) }()
	// An unprotected load permits the transaction to finish before its stale repair.
	// With locking, release the read first so that the transaction can acquire the lock.
	if lockErr == nil {
		if serr := <-changed; serr != nil {
			t.Fatal(serr)
		}
	}
	if _, err := writer.Write(data); err != nil {
		t.Fatal(err)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	if serr := <-loaded; serr != nil {
		t.Fatal(serr)
	}
	if lockErr != nil {
		if serr := <-changed; serr != nil {
			t.Fatal(serr)
		}
	}
	final, serr := b.load()
	if serr != nil {
		t.Fatal(serr)
	}
	if final.NextHelperID != 77 {
		t.Errorf("transaction overwritten: next helper = %d, want 77", final.NextHelperID)
	}
	if lockErr == nil {
		t.Error("receipt recovery read state without holding the bus lock")
	}
}
