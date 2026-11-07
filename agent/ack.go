package main

import (
	"fmt"
	"io"
	"os"
)

// ackableStatus preserves the Bash terminal-state allowlist, including unknown-state rejection.
func ackableStatus(status string) bool {
	return status == taskDone || status == taskFailed || status == taskStopped || status == taskExcluded || status == taskHung
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
			if merr != nil && merr.Cause != ReadTaskMetaNotFound { // Missing metadata means an empty, nonterminal status, matching Bash.
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
	if merr != nil && merr.Cause != ReadTaskMetaNotFound { // Missing metadata means an empty, nonterminal status, matching Bash.
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
