package main

import (
	"context"
	"fmt"
	"io"
	"io/ioutil"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// cmdShow displays full details of a single task: metadata, task.md, questions, result, report.
func cmdShow(ctx context.Context, app *App, taskID string, out io.Writer) *ResolveTaskError {
	// Resolve the task ID (simple matching for now)
	tasksDir := app.tasksDir
	if tasksDir == "" {
		return opErr(ResolveTaskNotFound, "no task directory")
	}

	// Try exact match first
	taskDir := filepath.Join(tasksDir, taskID)
	if _, err := os.Stat(taskDir); os.IsNotExist(err) {
		// Try partial match
		entries, err := ioutil.ReadDir(tasksDir)
		if err != nil {
			return opErr(ResolveTaskNotFound, "cannot read tasks directory")
		}
		
		var matches []string
		for _, entry := range entries {
			if entry.IsDir() && strings.Contains(entry.Name(), taskID) {
				matches = append(matches, entry.Name())
			}
		}
		
		if len(matches) == 0 {
			return opErr(ResolveTaskNotFound, "no task matches: "+taskID)
		}
		if len(matches) > 1 {
			return opErr(ResolveTaskNotFound, "ambiguous task ID: "+taskID)
		}
		
		taskDir = filepath.Join(tasksDir, matches[0])
		taskID = matches[0]
	}

	// Load metadata
	meta := readMetaFile(filepath.Join(taskDir, "meta.env"))

	// Print task header
	fmt.Fprintf(out, "== %s\n", taskID)

	// Print meta.env indented by 2 spaces
	metaFile := filepath.Join(taskDir, "meta.env")
	if content, err := ioutil.ReadFile(metaFile); err == nil {
		for _, line := range strings.Split(string(content), "\n") {
			if strings.TrimSpace(line) != "" {
				fmt.Fprintf(out, "  %s\n", line)
			}
		}
	}

	// Print task.md if it exists
	taskmdFile := filepath.Join(taskDir, "task.md")
	if content, err := ioutil.ReadFile(taskmdFile); err == nil {
		fmt.Fprintf(out, "\n== task\n")
		fmt.Fprint(out, string(content))
	}

	// Print questions if ROUND > 0
	round, _ := strconv.Atoi(meta["ROUND"])
	if round > 0 {
		questionsFile := filepath.Join(taskDir, fmt.Sprintf("questions-%d.md", round))
		if content, err := ioutil.ReadFile(questionsFile); err == nil {
			fmt.Fprintf(out, "\n== questions (round %d): %s\n", round, questionsFile)
			fmt.Fprint(out, string(content))
		}
	}

	// Print investigations
	if meta["KIND"] == "worker" {
		// Worker tasks can have investigations
		// TODO: call bus inv-list
		fmt.Fprintf(out, "\n== investigations\n")
	} else if meta["KIND"] == "investigation" {
		if team := meta["INVESTIGATION_TEAM"]; team != "" {
			phase := meta["INVESTIGATION_PHASE"]
			fmt.Fprintf(out, "\n== investigation team %s (phase %s)\n", team, phase)
		} else {
			invID := meta["INV_ID"]
			fromAgent := meta["FROM_AGENT"]
			parentTask := meta["PARENT_TASK"]
			fmt.Fprintf(out, "\n== investigation #%s for %s (task %s)\n", invID, fromAgent, parentTask)
		}
		
		// Show result.md if it exists
		resultFile := filepath.Join(taskDir, "result.md")
		if content, err := ioutil.ReadFile(resultFile); err == nil && len(content) > 0 {
			fmt.Fprintf(out, "   result: %s\n", resultFile)
		}
	}

	// Print final message from final.md
	finalFile := filepath.Join(taskDir, "final.md")
	if content, err := ioutil.ReadFile(finalFile); err == nil && len(content) > 0 {
		fmt.Fprintf(out, "\n== the agent's last message\n")
		fmt.Fprint(out, string(content))
	}

	// Print report if it exists
	reportFile := filepath.Join(app.workTree, ".agents", "reports", taskID+".md")
	if _, err := os.Stat(reportFile); err == nil {
		fmt.Fprintf(out, "\n== report: %s\n", reportFile)
	}

	return nil
}
