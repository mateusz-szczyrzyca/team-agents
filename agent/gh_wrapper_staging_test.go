package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestEnsureGHWrapper_UniqueStagingSentinelPreserved(t *testing.T) {
	home := t.TempDir()
	self := filepath.Join(home, "bin", "agent")
	_ = os.MkdirAll(filepath.Dir(self), 0o750)
	_ = os.WriteFile(self, []byte("self"), 0o755)
	wrap := filepath.Join(home, "run", "gh-trusted", "gh")
	_ = os.MkdirAll(filepath.Dir(wrap), 0o750)
	sentinel := wrap + ".tmp"
	if err := os.Symlink("/dev/null", sentinel); err != nil {
		t.Fatal(err)
	}
	res, err := EnsureGHWrapper(home, self)
	if err != nil {
		t.Fatalf("EnsureGHWrapper failed with sentinel present: %v", err)
	}
	if res != wrap {
		t.Fatalf("got %q, want %q", res, wrap)
	}
	if target, rerr := os.Readlink(sentinel); rerr != nil || target != "/dev/null" {
		t.Fatalf("sentinel target = %q, err = %v; want /dev/null", target, rerr)
	}
}
