package main

import (
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
)

// Metadata status values remain strings so unknown legacy states stay readable.
const (
	taskNew      string = "new"      // taskNew has not started a run.
	taskStarting string = "starting" // taskStarting is preparing a run.
	taskRunning  string = "running"  // taskRunning has an executing run.
	taskSleeping string = "sleeping" // taskSleeping reserves its agent between turns.
	taskDone     string = "done"     // taskDone completed successfully.
	taskFailed   string = "failed"   // taskFailed completed unsuccessfully.
	taskStopped  string = "stopped"  // taskStopped was stopped explicitly.
	taskExcluded string = "excluded" // taskExcluded was removed from a team.
	taskHung     string = "hung"     // taskHung lost its executing unit.
)

// Task kind values classify tasks across runner listings and detail displays.
const (
	kindWorker            string = "worker"             // kindWorker is an autonomous worker task.
	kindInvestigation     string = "investigation"      // kindInvestigation is an investigation task.
	kindRemoteMember      string = "remote-member"      // kindRemoteMember is a remote team member task.
	kindInvestigationTeam string = "investigation_team" // kindInvestigationTeam is an investigation team task.
)

// ackableStatus preserves the Bash terminal-state allowlist, including unknown-state rejection.
func ackableStatus(status string) bool {
	switch status {
	case taskDone, taskFailed, taskStopped, taskExcluded, taskHung:
		return true
	default:
		return false
	}
}

// taskDirectories follows Bash's non-hidden */ glob, including directory symlinks.
func taskDirectories(root string) ([]string, *StorageError) {
	entries, err := os.ReadDir(root)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return nil, storageErr(err, "cannot list tasks in %s", root)
	}
	var ids []string // ids retains lexical order for the first busy-task match.
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), ".") {
			continue
		}
		info, statErr := os.Stat(root + "/" + entry.Name()) // #nosec G703 -- local directory entry inspection matches Bash task path resolution
		if errors.Is(statErr, os.ErrNotExist) {
			continue
		}
		if statErr != nil {
			return nil, storageErr(statErr, "cannot inspect task %s", entry.Name())
		}
		if !info.IsDir() {
			continue
		}
		metaInfo, metaErr := os.Stat(root + "/" + entry.Name() + "/meta.env") // #nosec G703 -- internal task metadata file inspection matches Bash path resolution
		if errors.Is(metaErr, os.ErrNotExist) {
			continue
		}
		if metaErr != nil {
			return nil, storageErr(metaErr, "cannot inspect task metadata %s", entry.Name())
		}
		if metaInfo.Mode().IsRegular() {
			ids = append(ids, entry.Name())
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
			if merr != nil {
				if merr.Cause == ReadTaskMetaNotFound {
					continue
				}
				return fromStorage(AckStorage, storageErr(merr, "cannot read task %s", id))
			}
			status, _ := meta.Lookup("STATUS")
			if !ackableStatus(status) {
				continue
			}
			if serr := acknowledgeTask(root, id); serr != nil {
				return fromStorage(AckStorage, serr)
			}
			n++
		}
		if _, err := fmt.Fprintf(out, "Acknowledged: %d\n", n); err != nil {
			return fromStorage(AckStorage, storageErr(err, "write acknowledgement count"))
		}
		return nil
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
	status, _ := meta.Lookup("STATUS")
	if !ackableStatus(status) {
		return opErr(AckStillRunning, "task %s is still running", id)
	}
	if serr := acknowledgeTask(root, id); serr != nil {
		return fromStorage(AckStorage, serr)
	}
	if _, err := fmt.Fprintf(out, "Acknowledged %s.\n", id); err != nil {
		return fromStorage(AckStorage, storageErr(err, "write acknowledgement"))
	}
	return nil
}
