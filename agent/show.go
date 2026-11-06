package main

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// cmdShow displays full details of a single task: metadata, task.md, questions, result, report.
func cmdShow(ctx context.Context, app *App, taskID string, out io.Writer, stderr ...io.Writer) *ShowError {
	stderrWriter := io.Writer(os.Stderr)
	if len(stderr) > 0 && stderr[0] != nil {
		stderrWriter = stderr[0]
	}
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
		return opErr(ShowNotFound, "cannot read task metadata: %s", readErr.Error())
	}

	// Print task header
	if _, err := fmt.Fprintf(out, "== %s\n", taskID); err != nil {
		return fromStorage(ShowStorage, storageErr(err, "write task header"))
	}

	// Print meta.env indented by 2 spaces
	metaFile := filepath.Join(taskDir, "meta.env")
	if content, err := os.ReadFile(metaFile); err == nil { // #nosec G304,G703 -- local task metadata path under tasks directory
		lines := strings.Split(string(content), "\n")
		// Skip only the final empty element from the trailing newline
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
	if content, err := os.ReadFile(taskmdFile); err == nil { // #nosec G304,G703 -- local task description file under tasks directory
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
		if _, err := os.Stat(questionsFile); err == nil { // #nosec G703 -- constructed from validated taskDir
			if _, err := fmt.Fprintf(out, "\n== questions (round %d): %s\n", round, questionsFile); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write questions header"))
			}
		}
	}

	// Print investigations
	kind, _ := meta.Lookup("KIND")
	switch kind {
	case "worker":
		// Worker tasks can have investigations
		logDir := filepath.Join(app.agentHome, "logs")
		logger := &Logger{LogDir: logDir, Subcommand: "show", Stderr: stderrWriter}
		var invOut bytes.Buffer
		lerr := Try(ctx, app, logger, LogInvListFailed, Command{
			Name: app.busBinary(),
			Args: []string{"inv-list", "--dir", app.busDirectory(), "--task", taskID},
			Stdout: &invOut,
		})
		// Try returns nil on success, *LogWriteError on logging failure
		if lerr != nil {
			// Log write error is non-fatal, continue
		}
		if invOut.Len() > 0 && strings.TrimSpace(invOut.String()) != "" {
			if _, err := fmt.Fprintf(out, "\n== investigations\n%s\n", invOut.String()); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write investigations"))
			}
		}
	case "investigation":
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
		if content, err := os.ReadFile(resultFile); err == nil && len(content) > 0 { // #nosec G304,G703 -- local investigation result path under tasks directory
			if _, err := fmt.Fprintf(out, "   result: %s\n", resultFile); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write result path"))
			}
		}
	}

	// Print final message from final.md
	finalFile := filepath.Join(taskDir, "final.md")
	if content, err := os.ReadFile(finalFile); err == nil && len(content) > 0 { // #nosec G304,G703 -- local final message path under tasks directory
		if _, err := fmt.Fprintf(out, "\n== the agent's last message\n"); err != nil {
			return fromStorage(ShowStorage, storageErr(err, "write final message header"))
		}
		if _, err := fmt.Fprint(out, string(content)); err != nil {
			return fromStorage(ShowStorage, storageErr(err, "write final message content"))
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
		if _, err := os.Stat(reportFile); err == nil { // #nosec G703 -- local report file path under workTree
			if _, err := fmt.Fprintf(out, "\n== report: %s\n", reportFile); err != nil {
				return fromStorage(ShowStorage, storageErr(err, "write report line"))
			}
		}
	}

	return nil
}
