package main

import (
	"errors"
	"os"
	"path/filepath"
	"syscall"
	"time"
)

// metaLockTimeout matches the Bash metadata writer's default wait limit.
const metaLockTimeout = 30 * time.Second

// metaLockPoll bounds contention polling without extending the requested timeout.
const metaLockPoll = 10 * time.Millisecond

// lockTaskMeta serializes metadata writers using the Bash-compatible lock file.
// A nonpositive timeout selects the default; the caller owns the acquired file.
func lockTaskMeta(tasksDir, id string, timeout time.Duration) (*os.File, *SetTaskMetaError) {
	if timeout <= 0 {
		timeout = metaLockTimeout
	}
	path := filepath.Join(tasksDir, id, "meta.env.lock")
	f, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR|syscall.O_CLOEXEC, 0600) // #nosec G304,G703 -- tasksDir and id are trusted internal task coordinates
	if err != nil {
		return nil, fromStorage(SetTaskMetaStorage, storageErr(err, "open metadata lock %s", path))
	}
	deadline := time.Now().Add(timeout)
	for {
		err = syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB)
		if err == nil {
			return f, nil
		}
		if !errors.Is(err, syscall.EWOULDBLOCK) && !errors.Is(err, syscall.EINTR) {
			return nil, closeFailedMetaLock(f, fromStorage(SetTaskMetaStorage, storageErr(err, "lock metadata %s", path)))
		}
		remaining := time.Until(deadline)
		if remaining <= 0 {
			return nil, closeFailedMetaLock(f, opErr(SetTaskMetaLockTimeout, "timed out waiting for metadata lock %s", path))
		}
		time.Sleep(min(metaLockPoll, remaining))
	}
}

// closeFailedMetaLock preserves the operation cause and any descriptor cleanup failure.
func closeFailedMetaLock(f *os.File, failure *SetTaskMetaError) *SetTaskMetaError {
	if err := f.Close(); err != nil {
		failure.Err = errors.Join(failure.Err, storageErr(err, "close failed metadata lock %s", f.Name()).asError())
	}
	return failure
}

// unlockTaskMeta releases the lock and closes its descriptor, reporting either failure.
func unlockTaskMeta(f *os.File) *StorageError {
	unlockErr := syscall.Flock(int(f.Fd()), syscall.LOCK_UN)
	// Closing must run even when unlocking fails; both failures belong to the caller.
	closeErr := f.Close()
	if err := errors.Join(unlockErr, closeErr); err != nil {
		return storageErr(err, "release metadata lock %s", f.Name())
	}
	return nil
}
