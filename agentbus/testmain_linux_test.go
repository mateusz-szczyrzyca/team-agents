//go:build linux

package main

import (
	"os"
	"path/filepath"
)

// testOpenFiles inventories this process only; concurrent agents may write their own real logs safely.
func testOpenFiles() (map[string]string, error) {
	entries, err := os.ReadDir("/proc/self/fd")
	if err != nil {
		return nil, err
	}
	files := make(map[string]string)
	for _, entry := range entries {
		path, err := os.Readlink(filepath.Join("/proc/self/fd", entry.Name()))
		if os.IsNotExist(err) {
			continue // ReadDir's own descriptor may already have closed
		}
		if err != nil {
			return nil, err
		}
		files[entry.Name()] = path
	}
	return files, nil
}
