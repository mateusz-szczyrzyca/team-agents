package main

import (
	"errors"
	"os"
	"strings"
)

// Task status and kind constants classify tasks across runner listings and displays.
const (
	taskNew, taskStarting, taskRunning      string = "new", "starting", "running"          // taskNew, taskStarting, taskRunning identify active states.
	taskSleeping, taskDone, taskFailed      string = "sleeping", "done", "failed"          // taskSleeping, taskDone, taskFailed identify resting or finished states.
	taskStopped, taskExcluded, taskHung     string = "stopped", "excluded", "hung"         // taskStopped, taskExcluded, taskHung identify abnormal states.
	kindWorker, kindInvestigation           string = "worker", "investigation"             // kindWorker, kindInvestigation identify standard tasks.
	kindRemoteMember, kindInvestigationTeam string = "remote-member", "investigation_team" // kindRemoteMember, kindInvestigationTeam identify team tasks.
)

// taskDirectories follows Bash's non-hidden */ glob, including directory symlinks.
func taskDirectories(root string) ([]string, *StorageError) {
	entries, err := os.ReadDir(root)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return nil, storageErr(err, "cannot list tasks in %s", root)
	}
	var ids []string // ids retains lexical order for the first busy-task match.
	for _, entry := range entries {
		name := entry.Name()
		if strings.HasPrefix(name, ".") {
			continue
		}
		path := root + "/" + name
		if info, err := os.Stat(path); err != nil || !info.IsDir() { // #nosec G703 -- local directory entry inspection matches Bash task path resolution
			if err != nil && !errors.Is(err, os.ErrNotExist) {
				return nil, storageErr(err, "cannot inspect task %s", name)
			}
			continue
		}
		if mi, err := os.Stat(path + "/meta.env"); err != nil && !errors.Is(err, os.ErrNotExist) { // #nosec G703 -- internal task metadata file inspection matches Bash path resolution
			return nil, storageErr(err, "cannot inspect task metadata %s", name)
		} else if err == nil && mi.Mode().IsRegular() {
			ids = append(ids, name)
		}
	}
	return ids, nil
}
