package main

import (
	"errors"
	"fmt"
	"io"
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

// ackableStatus preserves the Bash terminal-state allowlist, including unknown-state rejection.
func ackableStatus(status string) bool {
	return status == taskDone || status == taskFailed || status == taskStopped || status == taskExcluded || status == taskHung
}

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

// acknowledgeTask truncates the marker just as Bash's redirection does.
func acknowledgeTask(root, id string) *StorageError {
	if err := os.WriteFile(root+"/"+id+"/acked", nil, 0o666); err != nil { // #nosec G306,G703 -- local task reference and marker mode preserve Bash redirection, filtered by the process umask
		return storageErr(err, "cannot acknowledge task %s", id)
	}
	return nil
}

// ackPrint writes formatted output or returns AckStorage error.
func ackPrint(out io.Writer, format string, args ...any) *AckError {
	if _, err := fmt.Fprintf(out, format, args...); err != nil {
		return fromStorage(AckStorage, storageErr(err, "write acknowledgement"))
	}
	return nil
}

// cmdAck acknowledges terminal tasks in the explicitly supplied task directory.
func (a *App) cmdAck(target string, out io.Writer, root string) *AckError {
	if target == "all" {
		ids, serr := taskDirectories(root)
		if serr != nil {
			return fromStorage(AckStorage, serr)
		}
		n := 0 // n includes existing acknowledgement markers, matching repeated Bash calls.
		for _, id := range ids {
			meta, merr := readTaskMeta(root, id)
			if merr != nil && merr.Cause != ReadTaskMetaNotFound {
				return fromStorage(AckStorage, storageErr(merr, "cannot read task %s", id))
			}
			if ackableStatus(metaLookup(meta, "STATUS", "")) {
				if serr := acknowledgeTask(root, id); serr != nil {
					return fromStorage(AckStorage, serr)
				}
				n++
			}
		}
		return ackPrint(out, "Acknowledged: %d\n", n)
	}
	id, rerr := resolveTaskID(root, target)
	if rerr != nil {
		switch rerr.Cause {
		case ResolveTaskNotFound:
			return opErr(AckNotFound, "%s", rerr.Message)
		case ResolveTaskAmbiguous:
			return opErr(AckAmbiguous, "%s", rerr.Message)
		default:
			return fromStorage(AckStorage, storageErr(rerr, "resolve task %s", target))
		}
	}
	meta, merr := readTaskMeta(root, id)
	if merr != nil && merr.Cause != ReadTaskMetaNotFound {
		return fromStorage(AckStorage, storageErr(merr, "read task %s", id))
	}
	if !ackableStatus(metaLookup(meta, "STATUS", "")) {
		return opErr(AckStillRunning, "task %s is still running", id)
	}
	if serr := acknowledgeTask(root, id); serr != nil {
		return fromStorage(AckStorage, serr)
	}
	return ackPrint(out, "Acknowledged %s.\n", id)
}
