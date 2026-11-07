package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestEnsureGHWrapper_UniqueStagingSentinelPreserved(t *testing.T) {
	home := t.TempDir()
	self := filepath.Join(home, "bin", "agent")
	wrap := filepath.Join(home, "run", "gh-trusted", "gh")
	if err := os.MkdirAll(filepath.Dir(wrap), 0o750); err != nil {
		t.Fatal(err)
	}
	sentinel := wrap + ".tmp"
	if err := os.Symlink("/dev/null", sentinel); err != nil {
		t.Fatal(err)
	}
	if res, err := EnsureGHWrapper(home, self); err != nil || res != wrap {
		t.Fatalf("EnsureGHWrapper = (%q, %v), want (%q, nil)", res, err, wrap)
	}
	if target, err := os.Readlink(sentinel); err != nil || target != "/dev/null" {
		t.Fatalf("sentinel target = %q, err = %v; want /dev/null", target, err)
	}
}
