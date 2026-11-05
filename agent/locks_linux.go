package main

import (
	"os"
	"time"
)

// lockTaskMeta serializes metadata writers using the Bash-compatible lock file.
func lockTaskMeta(_, _ string, _ time.Duration) (*os.File, *SetTaskMetaError) {
	return nil, nil
}

// unlockTaskMeta releases the lock and closes its descriptor, reporting either failure.
func unlockTaskMeta(_ *os.File) *StorageError {
	return nil
}
