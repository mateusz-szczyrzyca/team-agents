package main

import (
	"errors"
	"os"
	"path/filepath"
	"syscall"
	"testing"
	"time"
)

// metaLockTaskDir creates an isolated task directory for lock tests.
func metaLockTaskDir(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	if err := os.Mkdir(filepath.Join(root, "task"), 0700); err != nil {
		t.Fatal(err)
	}
	return root
}

// TestMetaLockOwnsCompatibleFile checks exclusion, permissions and exec isolation.
func TestMetaLockOwnsCompatibleFile(t *testing.T) {
	root := metaLockTaskDir(t)
	f, err := lockTaskMeta(root, "task", 0)
	if err != nil || f == nil {
		t.Fatalf("lockTaskMeta: file=%v error=%v", f, err)
	}
	defer func() {
		if closeErr := unlockTaskMeta(f); closeErr != nil {
			t.Error(closeErr)
		}
	}()
	info, statErr := os.Stat(filepath.Join(root, "task", "meta.env.lock"))
	if statErr != nil {
		t.Fatal(statErr)
	}
	if info.Mode().Perm() != 0600 {
		t.Errorf("permissions = %o, want 600", info.Mode().Perm())
	}
	flags, _, errno := syscall.Syscall(syscall.SYS_FCNTL, f.Fd(), syscall.F_GETFD, 0)
	if errno != 0 || flags&syscall.FD_CLOEXEC == 0 {
		t.Fatalf("descriptor flags=%d errno=%v, want close-on-exec", flags, errno)
	}
	other, openErr := os.OpenFile(f.Name(), os.O_RDWR, 0600)
	if openErr != nil {
		t.Fatal(openErr)
	}
	defer func() {
		if closeErr := other.Close(); closeErr != nil {
			t.Error(closeErr)
		}
	}()
	if flockErr := syscall.Flock(int(other.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); !errors.Is(flockErr, syscall.EWOULDBLOCK) {
		t.Fatalf("competing flock = %v, want EWOULDBLOCK", flockErr)
	}
}

// TestMetaLockTimeout checks the exact timeout cause against an independently held lock.
func TestMetaLockTimeout(t *testing.T) {
	root := metaLockTaskDir(t)
	path := filepath.Join(root, "task", "meta.env.lock")
	holder, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0600) // #nosec G304 -- path belongs to this test’s temporary task directory.
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if closeErr := holder.Close(); closeErr != nil {
			t.Error(closeErr)
		}
	}()
	if err := syscall.Flock(int(holder.Fd()), syscall.LOCK_EX); err != nil {
		t.Fatal(err)
	}
	f, lockErr := lockTaskMeta(root, "task", time.Millisecond)
	if lockErr == nil || lockErr.Cause != SetTaskMetaLockTimeout || f != nil {
		t.Fatalf("lockTaskMeta: file=%v error=%v, want LockTimeout", f, lockErr)
	}
}

// TestMetaLockMissingTask checks that failure to open the lock is a storage cause.
func TestMetaLockMissingTask(t *testing.T) {
	f, err := lockTaskMeta(t.TempDir(), "missing", time.Millisecond)
	if err == nil || err.Cause != SetTaskMetaStorage || !errors.Is(err.asError(), os.ErrNotExist) || f != nil {
		t.Fatalf("lockTaskMeta: file=%v error=%v, want Storage wrapping ENOENT", f, err)
	}
}

// TestMetaUnlockClosesAndReleases checks descriptor ownership and preserves lock-file data.
func TestMetaUnlockClosesAndReleases(t *testing.T) {
	root := metaLockTaskDir(t)
	path := filepath.Join(root, "task", "meta.env.lock")
	if err := os.WriteFile(path, []byte("keep"), 0600); err != nil {
		t.Fatal(err)
	}
	f, err := lockTaskMeta(root, "task", time.Millisecond)
	if err != nil || f == nil {
		t.Fatalf("lockTaskMeta: file=%v error=%v", f, err)
	}
	fd := int(f.Fd())
	if err := unlockTaskMeta(f); err != nil {
		t.Fatal(err)
	}
	var stat syscall.Stat_t
	if err := syscall.Fstat(fd, &stat); !errors.Is(err, syscall.EBADF) {
		t.Fatalf("released descriptor: %v, want EBADF", err)
	}
	again, err := lockTaskMeta(root, "task", time.Millisecond)
	if err != nil || again == nil {
		t.Fatalf("reacquire: file=%v error=%v", again, err)
	}
	if err := unlockTaskMeta(again); err != nil {
		t.Fatal(err)
	}
	data, readErr := os.ReadFile(path) // #nosec G304 -- path belongs to this test’s temporary task directory.
	if readErr != nil || string(data) != "keep" {
		t.Fatalf("lock file: %q error=%v, want unchanged data", data, readErr)
	}
}

// TestMetaUnlockReportsClosedDescriptor checks that unlock and close errors are observable.
func TestMetaUnlockReportsClosedDescriptor(t *testing.T) {
	f, err := os.CreateTemp(t.TempDir(), "lock")
	if err != nil {
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	unlockErr := unlockTaskMeta(f)
	if unlockErr == nil || !errors.Is(unlockErr.asError(), syscall.EBADF) || !errors.Is(unlockErr.asError(), os.ErrClosed) {
		t.Fatalf("unlockTaskMeta = %v, want both EBADF and ErrClosed", unlockErr)
	}
}
