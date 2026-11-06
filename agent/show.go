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
	if app.tasksDir == "" {
		return opErr(ShowNotFound, "no task directory")
	}
	resolvedID, rerr := resolveTaskID(app.tasksDir, taskID)
	if rerr != nil {
		if rerr.Cause == ResolveTaskNotFound {
			return opErr(ShowNotFound, "%s", rerr.Message)
		} else if rerr.Cause == ResolveTaskAmbiguous {
			return opErr(ShowAmbiguous, "%s", rerr.Message)
		}
		return fromStorage(ShowStorage, storageErr(rerr.Err, "%s", rerr.Message))
	}
	taskID = resolvedID
	taskDir := filepath.Join(app.tasksDir, taskID)

	meta, readErr := readTaskMeta(app.tasksDir, taskID)
	if readErr != nil {
		if readErr.Cause == ReadTaskMetaNotFound {
			return opErr(ShowNotFound, "task %s not found", taskID)
		}
		return fromStorage(ShowStorage, storageErr(readErr, "read metadata for %s", taskID))
	}

	if _, err := fmt.Fprintf(out, "== %s\n", taskID); err != nil {
		return fromStorage(ShowStorage, storageErr(err, "write task header"))
	}

	if content, err := os.ReadFile(filepath.Join(taskDir, "meta.env")); err == nil && len(content) > 0 { // #nosec G304
		hasNL := strings.HasSuffix(string(content), "\n")
		for _, line := range strings.Split(strings.TrimSuffix(string(content), "\n"), "\n") {
			if _, err := fmt.Fprintf(out, "  %s\n", line); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write meta.env line"))
			}
		}
		if hasNL {
			if _, err := fmt.Fprintln(out); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write newline"))
			}
		}
	} else if err != nil && !errors.Is(err, os.ErrNotExist) {
		return fromStorage(ShowStorage, storageErr(err, "read meta.env"))
	}

	if _, err := fmt.Fprintf(out, "== task\n"); err != nil {
		return fromStorage(ShowStorage, storageErr(err, "write task header"))
	}
	taskContent, err := os.ReadFile(filepath.Join(taskDir, "task.md")) // #nosec G304,G703
	if err != nil {
		return fromStorage(ShowStorage, storageErr(err, "read task.md"))
	}
	if _, err := fmt.Fprint(out, string(taskContent)); err != nil {
		return fromStorage(ShowStorage, storageErr(err, "write task content"))
	}

	if roundStr := metaLookup(meta, "ROUND", ""); roundStr != "" {
		round, err := strconv.Atoi(roundStr)
		if err != nil {
			return fromStorage(ShowStorage, storageErr(err, "invalid ROUND %q", roundStr))
		}
		if round > 0 {
			if _, err := fmt.Fprintf(out, "\n== questions (round %d): %s\n", round, filepath.Join(taskDir, fmt.Sprintf("questions-%d.md", round))); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write questions header"))
			}
		}
	}

	switch metaLookup(meta, "KIND", kindWorker) {
	case kindWorker:
		var invs bytes.Buffer
		logger := app.newLogger("show")
		if lerr := Try(ctx, app, logger, LogInvListFailed, Command{
			Name:   app.busBinary(),
			Args:   []string{"inv-list", "--dir", app.busDirectory(), "--task", taskID},
			Stdout: &invs,
		}); lerr != nil && lerr.Cause != LogWriteStorage {
			return fromStorage(ShowStorage, storageErr(lerr, "inv-list warning"))
		}
		if s := strings.TrimRight(invs.String(), "\n"); s != "" {
			if _, err := fmt.Fprintf(out, "\n== investigations\n%s\n", s); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write investigations"))
			}
		}
	case kindInvestigation:
		if team := metaLookup(meta, "INVESTIGATION_TEAM", ""); team != "" {
			if _, err := fmt.Fprintf(out, "\n== investigation team %s (phase %s)\n", team, metaLookup(meta, "INVESTIGATION_PHASE", "")); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write investigation team header"))
			}
		} else {
			if _, err := fmt.Fprintf(out, "\n== investigation #%s for %s (task %s)\n", metaLookup(meta, "INV_ID", ""), metaLookup(meta, "FROM_AGENT", ""), metaLookup(meta, "PARENT_TASK", "")); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write investigation header"))
			}
		}
		resultPath := filepath.Join(taskDir, "result.md")
		if fi, err := os.Stat(resultPath); err == nil && fi.Mode().IsRegular() && fi.Size() > 0 {
			if _, err := fmt.Fprintf(out, "   result: %s\n", resultPath); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write result path"))
			}
		}
	}

	finalPath := filepath.Join(taskDir, "final.md")
	if fi, err := os.Stat(finalPath); err == nil && fi.Mode().IsRegular() {
		if content, err := os.ReadFile(finalPath); err != nil { // #nosec G304
			return fromStorage(ShowStorage, storageErr(err, "read final.md"))
		} else if last := strings.TrimRight(string(content), "\n"); last != "" {
			if _, err := fmt.Fprintf(out, "\n== the agent's last message\n%s\n", last); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write final message"))
			}
		}
	}

	if workTree := metaLookup(meta, "WORKTREE", app.workTree); workTree != "" {
		reportFile := filepath.Join(workTree, ".agents", "reports", taskID+".md")
		if info, err := os.Stat(reportFile); err == nil && info.Mode().IsRegular() { // #nosec G703
			if _, err := fmt.Fprintf(out, "\n== report: %s\n", reportFile); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write report line"))
			}
		} else if err != nil && !errors.Is(err, os.ErrNotExist) {
			return fromStorage(ShowStorage, storageErr(err, "stat %s", reportFile))
		}
	}
	return nil
}
