package main

import (
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// cmdShow displays full details of a single task: metadata, task.md, questions, result, report.
func cmdShow(_ context.Context, app *App, taskID string, out io.Writer) *ShowError {
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
		for _, line := range strings.Split(string(content), "\n") {
			if strings.TrimSpace(line) != "" {
				if _, err := fmt.Fprintf(out, "  %s\n", line); err != nil {
					return fromStorage(ShowStorage, storageErr(err, "write meta.env line"))
				}
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
		// Worker tasks can have investigations (query via bus inv-list in future rounds)
		// For now, only print header if there are investigations
		// TODO: call bus inv-list --dir busDir --task taskID and only print header if non-empty
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
