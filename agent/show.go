package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// cmdShow displays full details of a single task: metadata, task.md, questions, result, report.
func cmdShow(ctx context.Context, app *App, taskID string, out io.Writer) *ShowError {
	tasksDir := app.tasksDir
	if tasksDir == "" {
		return opErr(ShowNotFound, "no task directory")
	}

	resolvedID, rerr := resolveTaskID(tasksDir, taskID)
	if rerr != nil {
		switch rerr.Cause {
		case ResolveTaskNotFound:
			return opErr(ShowNotFound, "%s", rerr.Message)
		case ResolveTaskAmbiguous:
			return opErr(ShowAmbiguous, "%s", rerr.Message)
		default:
			return fromStorage(ShowStorage, storageErr(rerr.Err, "%s", rerr.Message))
		}
	}
	taskID = resolvedID
	taskDir := filepath.Join(tasksDir, taskID)

	meta, readErr := readTaskMeta(tasksDir, taskID)
	if readErr != nil {
		if readErr.Cause == ReadTaskMetaNotFound {
			return opErr(ShowNotFound, "task %s not found", taskID)
		}
		return fromStorage(ShowStorage, storageErr(readErr, "read metadata for %s", taskID))
	}

	// Print task header
	if _, err := fmt.Fprintf(out, "== %s\n", taskID); err != nil {
		return fromStorage(ShowStorage, storageErr(err, "write task header"))
	}

	// Print meta.env indented by 2 spaces
	if content, err := readOptionalTaskFile(taskDir, "meta.env"); err != nil {
		return err
	} else if len(content) > 0 {
		for _, line := range strings.Split(strings.TrimSuffix(string(content), "\n"), "\n") {
			if _, err := fmt.Fprintf(out, "  %s\n", line); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write meta.env line"))
			}
		}
	}

	// Print task.md
	if _, err := fmt.Fprintf(out, "\n== task\n"); err != nil {
		return fromStorage(ShowStorage, storageErr(err, "write task header"))
	}
	taskmdFile := filepath.Join(taskDir, "task.md")
	taskContent, err := os.ReadFile(taskmdFile) // #nosec G304,G703 -- local task directory inspection matches Bash path resolution
	if err != nil {
		return fromStorage(ShowStorage, storageErr(err, "read %s", taskmdFile))
	}
	if _, err := fmt.Fprint(out, string(taskContent)); err != nil {
		return fromStorage(ShowStorage, storageErr(err, "write task content"))
	}

	// Print questions header if ROUND > 0
	if roundStr, ok := meta.Lookup("ROUND"); ok && roundStr != "" {
		round, err := strconv.Atoi(roundStr)
		if err != nil {
			return fromStorage(ShowStorage, storageErr(err, "invalid ROUND %q", roundStr))
		}
		if round > 0 {
			questionsFile := filepath.Join(taskDir, fmt.Sprintf("questions-%d.md", round))
			if _, err := fmt.Fprintf(out, "\n== questions (round %d): %s\n", round, questionsFile); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write questions header"))
			}
		}
	}

	// Print investigations
	switch metaLookup(meta, "KIND", kindWorker) {
	case kindWorker:
		var invs bytes.Buffer
		logger := app.newLogger("show")
		if lerr := Try(ctx, app, logger, LogInvListFailed, Command{
			Name:   app.busBinary(),
			Args:   []string{"inv-list", "--dir", app.busDirectory(), "--task", taskID},
			Stdout: &invs,
		}); lerr != nil {
			if lerr.Cause == LogWriteStorage {
				// Best-effort warning log failure is nonfatal per bin/agent:460; diagnostic was written to stderr by logger.
				lerr = nil
			} else {
				return fromStorage(ShowStorage, storageErr(lerr, "inv-list warning"))
			}
		}
		if s := strings.TrimRight(invs.String(), "\r\n"); s != "" {
			if _, err := fmt.Fprintf(out, "\n== investigations\n%s\n", s); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write investigations"))
			}
		}
	case kindInvestigation:
		if team := metaLookup(meta, "INVESTIGATION_TEAM", ""); team != "" {
			phase := metaLookup(meta, "INVESTIGATION_PHASE", "")
			if _, err := fmt.Fprintf(out, "\n== investigation team %s (phase %s)\n", team, phase); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write investigation team header"))
			}
		} else {
			invID, fromAgent, parentTask := metaLookup(meta, "INV_ID", ""), metaLookup(meta, "FROM_AGENT", ""), metaLookup(meta, "PARENT_TASK", "")
			if _, err := fmt.Fprintf(out, "\n== investigation #%s for %s (task %s)\n", invID, fromAgent, parentTask); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write investigation header"))
			}
		}

		// Show result.md if it exists
		if content, err := readOptionalTaskFile(taskDir, "result.md"); err != nil {
			return err
		} else if len(content) > 0 {
			if _, err := fmt.Fprintf(out, "   result: %s\n", filepath.Join(taskDir, "result.md")); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write result path"))
			}
		}
	}

	// Print final message from final.md
	if content, err := readOptionalTaskFile(taskDir, "final.md"); err != nil {
		return err
	} else if last := strings.TrimRight(string(content), "\r\n"); last != "" {
		if _, err := fmt.Fprintf(out, "\n== the agent's last message\n%s\n", last); err != nil {
			return fromStorage(ShowStorage, storageErr(err, "write final message"))
		}
	}

	// Print report if it exists
	workTree := metaLookup(meta, "WORKTREE", app.workTree)
	if workTree != "" {
		reportFile := filepath.Join(workTree, ".agents", "reports", taskID+".md")
		if info, err := os.Stat(reportFile); err == nil && info.Mode().IsRegular() { // #nosec G703 -- report file path under task worktree
			if _, err := fmt.Fprintf(out, "\n== report: %s\n", reportFile); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write report line"))
			}
		} else if err != nil && !errors.Is(err, os.ErrNotExist) {
			return fromStorage(ShowStorage, storageErr(err, "stat %s", reportFile))
		}
	}

	return nil
}

// readOptionalTaskFile reads a named file in taskDir, returning nil if it does not exist.
func readOptionalTaskFile(taskDir, name string) ([]byte, *ShowError) {
	data, err := os.ReadFile(filepath.Join(taskDir, name)) // #nosec G304,G703 -- local task directory inspection matches Bash path resolution
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return nil, fromStorage(ShowStorage, storageErr(err, "read %s", filepath.Join(taskDir, name)))
	}
	return data, nil
}
