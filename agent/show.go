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

// showPrint formats and writes output, mapping failures to ShowStorage.
func showPrint(out io.Writer, op, format string, args ...any) *ShowError {
	if _, err := fmt.Fprintf(out, format, args...); err != nil {
		return fromStorage(ShowStorage, storageErr(err, op))
	}
	return nil
}

// cmdShow displays full details of a single task: metadata, task.md, questions, result, report.
func cmdShow(ctx context.Context, app *App, taskID string, out io.Writer) *ShowError {
	if app.tasksDir == "" {
		return opErr(ShowNotFound, "no task directory")
	}
	resolvedID, rerr := resolveTaskID(app.tasksDir, taskID)
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
	taskDir := filepath.Join(app.tasksDir, taskID)

	meta, readErr := readTaskMeta(app.tasksDir, taskID)
	if readErr != nil {
		if readErr.Cause == ReadTaskMetaNotFound {
			return opErr(ShowNotFound, "task %s not found", taskID)
		}
		return fromStorage(ShowStorage, storageErr(readErr, "read metadata for %s", taskID))
	}

	if err := showPrint(out, "write task header", "== %s\n", taskID); err != nil {
		return err
	}

	if content, err := os.ReadFile(filepath.Join(taskDir, "meta.env")); err == nil && len(content) > 0 { // #nosec G304,G703 -- local task directory inspection matches Bash path resolution
		s := "  " + strings.ReplaceAll(strings.TrimSuffix(string(content), "\n"), "\n", "\n  ") + "\n"
		if strings.HasSuffix(string(content), "\n") {
			s += "\n"
		}
		if err := showPrint(out, "write meta.env", "%s", s); err != nil {
			return err
		}
	} else if err != nil && !errors.Is(err, os.ErrNotExist) {
		return fromStorage(ShowStorage, storageErr(err, "read meta.env"))
	}

	if err := showPrint(out, "write task header", "== task\n"); err != nil {
		return err
	}
	taskContent, err := os.ReadFile(filepath.Join(taskDir, "task.md")) // #nosec G304,G703 -- local task directory inspection matches Bash path resolution
	if err != nil {
		return fromStorage(ShowStorage, storageErr(err, "read task.md"))
	}
	if err := showPrint(out, "write task content", "%s", string(taskContent)); err != nil {
		return err
	}

	if roundStr := metaLookup(meta, "ROUND", ""); roundStr != "" {
		round, err := strconv.Atoi(roundStr)
		if err != nil {
			return fromStorage(ShowStorage, storageErr(err, "invalid ROUND %q", roundStr))
		}
		if round > 0 {
			if err := showPrint(out, "write questions header", "\n== questions (round %d): %s/questions-%d.md\n", round, taskDir, round); err != nil {
				return err
			}
		}
	}

	switch metaLookup(meta, "KIND", kindWorker) {
	case kindWorker:
		var invs bytes.Buffer
		if lerr := Try(ctx, app, app.newLogger("show"), LogInvListFailed, Command{
			Name:   app.busBinary(),
			Args:   []string{"inv-list", "--dir", app.busDirectory(), "--task", taskID},
			Stdout: &invs,
		}); lerr != nil && lerr.Cause != LogWriteStorage {
			return fromStorage(ShowStorage, storageErr(lerr, "inv-list warning"))
		}
		if s := strings.TrimRight(invs.String(), "\n"); s != "" {
			if err := showPrint(out, "write investigations", "\n== investigations\n%s\n", s); err != nil {
				return err
			}
		}
	case kindInvestigation:
		header := fmt.Sprintf("\n== investigation #%s for %s (task %s)\n", metaLookup(meta, "INV_ID", ""), metaLookup(meta, "FROM_AGENT", ""), metaLookup(meta, "PARENT_TASK", ""))
		if team := metaLookup(meta, "INVESTIGATION_TEAM", ""); team != "" {
			header = fmt.Sprintf("\n== investigation team %s (phase %s)\n", team, metaLookup(meta, "INVESTIGATION_PHASE", ""))
		}
		if err := showPrint(out, "write investigation header", "%s", header); err != nil {
			return err
		}
		resultPath := filepath.Join(taskDir, "result.md")
		if fi, err := os.Stat(resultPath); err == nil && fi.Mode().IsRegular() && fi.Size() > 0 { // #nosec G703 -- local task directory inspection matches Bash path resolution
			if err := showPrint(out, "write result path", "   result: %s\n", resultPath); err != nil {
				return err
			}
		}
	}

	finalPath := filepath.Join(taskDir, "final.md")
	if fi, err := os.Stat(finalPath); err == nil && fi.Mode().IsRegular() { // #nosec G703 -- local task directory inspection matches Bash path resolution
		if content, err := os.ReadFile(finalPath); err != nil { // #nosec G304,G703 -- path constructed from validated task directory
			return fromStorage(ShowStorage, storageErr(err, "read final.md"))
		} else if last := strings.TrimRight(string(content), "\n"); last != "" {
			if err := showPrint(out, "write final message", "\n== the agent's last message\n%s\n", last); err != nil {
				return err
			}
		}
	} else if err != nil && !errors.Is(err, os.ErrNotExist) {
		return fromStorage(ShowStorage, storageErr(err, "stat final.md"))
	}

	if wt := metaLookup(meta, "WORKTREE", app.workTree); wt != "" {
		p := filepath.Join(wt, ".agents", "reports", taskID+".md")
		if info, err := os.Stat(p); err == nil && info.Mode().IsRegular() { // #nosec G703 -- report file path under task worktree
			return showPrint(out, "write report line", "\n== report: %s\n", p)
		} else if err != nil && !errors.Is(err, os.ErrNotExist) {
			return fromStorage(ShowStorage, storageErr(err, "stat %s", p))
		}
	}
	return nil
}
