// Recoverable storage for membership replacements. All helpers require the bus lock.
package main

import (
	"errors"
	"io"
	"os"
	"path/filepath"
)

// replacementRollback opts one transaction into recovery of membership and messages.
type replacementRollback struct{}

// replacementFiles are restored together so receipt offsets always describe the message log.
func replacementFiles() [2]string { return [2]string{"state.json", "messages.jsonl"} }

// copyReplacementFile makes an independent, flushed backup; append-only files cannot use hard links.
func copyReplacementFile(from, to string) *StorageError {
	in, err := os.Open(from) //nolint:gosec // G304: both paths are fixed bus-owned transaction files.
	if err != nil {
		return storageErr(err, "cannot open replacement source %s", from)
	}
	defer func() { warnReplacementCleanup("close replacement source", in.Close()) }()
	out, err := os.OpenFile(to, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0600) //nolint:gosec // G304: destination is a fixed bus-owned transaction file.
	if err != nil {
		return storageErr(err, "cannot open replacement backup %s", to)
	}
	if _, err := io.Copy(out, in); err != nil {
		warnReplacementCleanup("close failed replacement backup", out.Close())
		return storageErr(err, "cannot copy replacement file %s", from)
	}
	if err := out.Sync(); err != nil {
		warnReplacementCleanup("close failed replacement backup", out.Close())
		return storageErr(err, "cannot flush replacement file %s", to)
	}
	if err := out.Close(); err != nil {
		return storageErr(err, "cannot close replacement file %s", to)
	}
	return nil
}

// beginReplacement preserves both originals before any visible replacement writes.
func (b *Bus) beginReplacement() *StorageError {
	dir := b.path("replacement-undo")
	if err := os.RemoveAll(b.path("replacement-discard")); err != nil {
		return storageErr(err, "cannot remove retired replacement backups")
	}
	if err := os.Mkdir(dir, 0700); err != nil {
		return storageErr(err, "cannot create replacement rollback directory")
	}
	for _, name := range replacementFiles() {
		if _, err := os.Stat(b.path(name)); errors.Is(err, os.ErrNotExist) {
			continue
		} else if err != nil {
			return storageErr(err, "cannot inspect replacement file %s", name)
		}
		if serr := copyReplacementFile(b.path(name), filepath.Join(dir, name)); serr != nil {
			return serr
		}
	}
	if err := os.WriteFile(filepath.Join(dir, "ready"), nil, 0600); err != nil {
		return storageErr(err, "cannot mark replacement rollback ready")
	}
	return nil
}

// finishReplacement atomically retires the undo directory: subsequent loads keep the committed state.
func (b *Bus) finishReplacement() *StorageError {
	discard := b.path("replacement-discard")
	if err := os.Rename(b.path("replacement-undo"), discard); err != nil {
		return storageErr(err, "cannot finish replacement transaction")
	}
	// The commit is already visible; leftover backup cleanup cannot invalidate it.
	warnReplacementCleanup("remove retired replacement backups", os.RemoveAll(discard))
	return nil
}

// recoverReplacement rolls back incomplete writes, including after an interrupted process.
// Backups remain intact until both originals have been restored, so recovery itself is repeatable.
func (b *Bus) recoverReplacement() *StorageError {
	dir := b.path("replacement-undo")
	if _, err := os.Stat(filepath.Join(dir, "ready")); errors.Is(err, os.ErrNotExist) {
		if err := os.RemoveAll(dir); err != nil {
			return storageErr(err, "cannot remove incomplete replacement backups")
		}
		return nil
	} else if err != nil {
		return storageErr(err, "cannot inspect replacement rollback marker")
	}
	for _, name := range replacementFiles() {
		source := filepath.Join(dir, name)
		if _, err := os.Stat(source); errors.Is(err, os.ErrNotExist) {
			if err := os.Remove(b.path(name)); err != nil && !errors.Is(err, os.ErrNotExist) {
				return storageErr(err, "cannot restore absent replacement file %s", name)
			}
			continue
		} else if err != nil {
			return storageErr(err, "cannot inspect replacement backup %s", name)
		}
		// A separate recovery suffix must work even when state.json.tmp caused the failure.
		target := b.path(name + ".replacement-recover")
		if serr := copyReplacementFile(source, target); serr != nil {
			return serr
		}
		if err := os.Rename(target, b.path(name)); err != nil {
			return storageErr(err, "cannot restore replacement file %s", name)
		}
	}
	return b.finishReplacement()
}

// replacementTransact opts into rollback of state and messages without changing other operations' semantics.
func replacementTransact[C opCause](b *Bus, storage C, fn func(*State) *OpError[C]) *OpError[C] {
	var opErr *OpError[C]
	serr := b.withLock(func() *StorageError {
		st, serr := b.loadLocked()
		if serr != nil {
			return serr
		}
		if serr = b.beginReplacement(); serr != nil {
			return serr
		}
		b.gc(st)
		opErr = fn(st)
		if opErr == nil {
			serr = b.save(st)
		}
		if opErr == nil && serr == nil {
			if serr = b.finishReplacement(); serr == nil {
				return nil
			}
		}
		if rollback := b.recoverReplacement(); rollback != nil {
			return storageErr(errors.Join(serr, opErr.asError(), rollback), "cannot roll back replacement; recovery will retry on next load")
		}
		return serr
	})
	if serr != nil {
		return fromStorage(storage, serr)
	}
	return opErr
}

// warnReplacementCleanup reports cleanup failures after a commit or alongside an operation failure.
func warnReplacementCleanup(effect string, err error) {
	if err != nil {
		warnSideEffect(effect, storageErr(err, "replacement cleanup failed"))
	}
}
