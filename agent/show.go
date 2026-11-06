package main

import (
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
			return opErr(ShowNotFound, "no task matches: %s", taskID)
		case ResolveTaskAmbiguous:
			return opErr(ShowAmbiguous, "ambiguous task ID: %s", taskID)
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
	metaFile := filepath.Join(taskDir, "meta.env")
	content, err := os.ReadFile(metaFile) // #nosec G304,G703 -- local task directory inspection matches Bash path resolution
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return fromStorage(ShowStorage, storageErr(err, "read %s", metaFile))
	}
	if err == nil {
		lines := strings.Split(string(content), "\n")
		if len(lines) > 0 && lines[len(lines)-1] == "" {
			lines = lines[:len(lines)-1]
		}
		for _, line := range lines {
			if _, err := fmt.Fprintf(out, "  %s\n", line); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write meta.env line"))
			}
		}
	}

	// Print task.md if it exists
	taskmdFile := filepath.Join(taskDir, "task.md")
	content, err = os.ReadFile(taskmdFile) // #nosec G304,G703 -- local task directory inspection matches Bash path resolution
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return fromStorage(ShowStorage, storageErr(err, "read %s", taskmdFile))
	}
	if err == nil {
		if _, err := fmt.Fprintf(out, "\n== task\n"); err != nil {
			return fromStorage(ShowStorage, storageErr(err, "write task header"))
		}
		if _, err := fmt.Fprint(out, string(content)); err != nil {
			return fromStorage(ShowStorage, storageErr(err, "write task content"))
		}
	} else if !os.IsNotExist(err) {
		// Non-ENOENT errors (e.g., is a directory) are storage failures
		return fromStorage(ShowStorage, storageErr(err, "read task.md"))
	}

	// Print questions header if ROUND > 0
	roundStr, _ := meta.Lookup("ROUND")
	round, _ := strconv.Atoi(roundStr)
	if round > 0 {
		questionsFile := filepath.Join(taskDir, fmt.Sprintf("questions-%d.md", round))
		if _, err := fmt.Fprintf(out, "\n== questions (round %d): %s\n", round, questionsFile); err != nil {
			return fromStorage(ShowStorage, storageErr(err, "write questions header"))
		}
	}

	// Print investigations
	kind, _ := meta.Lookup("KIND")
	if kind == "" {
		kind = kindWorker
	}
	switch kind {
	case kindWorker:
		invs, err := busCmd(ctx, app, app.busBinary(), "inv-list", "--dir", app.busDirectory(), "--task", taskID)
		if err != nil {
			if app != nil && app.Stderr != nil {
				_, _ = fmt.Fprintf(app.Stderr, "agent: warning: inv-list failed: %s\n", err.Error())
			}
		} else {
			invs = strings.TrimRight(invs, "\r\n")
			if invs != "" {
				if _, err := fmt.Fprintf(out, "\n== investigations\n%s\n", invs); err != nil {
					return fromStorage(ShowStorage, storageErr(err, "write investigations"))
				}
			}
		}
	case kindInvestigation:
		team, _ := meta.Lookup("INVESTIGATION_TEAM")
		if team != "" {
			phase, _ := meta.Lookup("INVESTIGATION_PHASE")
			if _, err := fmt.Fprintf(out, "\n== investigation team %s (phase %s)\n", team, phase); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write investigation team header"))
			}
		} else {
			invID, _ := meta.Lookup("INV_ID")
			fromAgent, _ := meta.Lookup("FROM_AGENT")
			parentTask, _ := meta.Lookup("PARENT_TASK")
			if _, err := fmt.Fprintf(out, "\n== investigation #%s for %s (task %s)\n", invID, fromAgent, parentTask); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write investigation header"))
			}
		}

		// Show result.md if it exists
		resultFile := filepath.Join(taskDir, "result.md")
		content, err = os.ReadFile(resultFile) // #nosec G304,G703 -- local task directory inspection matches Bash path resolution
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			return fromStorage(ShowStorage, storageErr(err, "read %s", resultFile))
		}
		if err == nil && len(content) > 0 {
			if _, err := fmt.Fprintf(out, "   result: %s\n", resultFile); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write result path"))
			}
		}
	}

	// Print final message from final.md
	finalFile := filepath.Join(taskDir, "final.md")
	content, err = os.ReadFile(finalFile) // #nosec G304,G703 -- local task directory inspection matches Bash path resolution
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return fromStorage(ShowStorage, storageErr(err, "read %s", finalFile))
	}
	if err == nil {
		last := strings.TrimRight(string(content), "\r\n")
		if last != "" {
			if _, err := fmt.Fprintf(out, "\n== the agent's last message\n%s\n", last); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write final message"))
			}
		}
	} else if err != nil && !os.IsNotExist(err) {
		// Non-ENOENT errors (e.g., is a directory) are storage failures
		return fromStorage(ShowStorage, storageErr(err, "read final.md"))
	}

	// Print report if it exists
	workTree, _ := meta.Lookup("WORKTREE")
	if workTree == "" {
		workTree = app.workTree
	}
	if workTree != "" {
		reportFile := filepath.Join(workTree, ".agents", "reports", taskID+".md")
		if _, err := os.Stat(reportFile); err == nil { // #nosec G703 -- report file path under task worktree
			if _, err := fmt.Fprintf(out, "\n== report: %s\n", reportFile); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write report line"))
			}
		} else if !errors.Is(err, os.ErrNotExist) {
			return fromStorage(ShowStorage, storageErr(err, "stat %s", reportFile))
		}
	}

	return nil
}
