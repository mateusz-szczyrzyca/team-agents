package main

import (
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
)

// cmdStatus displays a table of all tasks with their status, agent, round, and run number.
func cmdStatus(_ context.Context, app *App, out io.Writer) *StatusError {
	// Print header
	if _, err := fmt.Fprintf(out, "%-30s %-12s %-13s %-6s %-5s %s\n",
		"TASK", "AGENT", "STATUS", "ROUND", "RUN", "REPO"); err != nil {
		return fromStorage(StatusStorage, storageErr(err, "write status header"))
	}

	// Read all tasks from tasksDir
	tasksDir := app.tasksDir
	if tasksDir == "" {
		return nil
	}

	entries, err := os.ReadDir(tasksDir)
	if err != nil && !os.IsNotExist(err) {
		return fromStorage(StatusStorage, storageErr(err, "read tasks dir"))
	}

	var tasks []string
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		metaFile := filepath.Join(tasksDir, entry.Name(), "meta.env")
		if _, err := os.Stat(metaFile); err == nil {
			tasks = append(tasks, entry.Name())
		}
	}
	sort.Strings(tasks)

	// Print each task
	for _, taskID := range tasks {
		meta, readErr := readTaskMeta(tasksDir, taskID)
		if readErr != nil {
			continue
		}

		agent, ok := meta.Lookup("AGENT")
		if !ok || agent == "" {
			agent = "?"
		}

		// Special handling for investigation teams
		kind, _ := meta.Lookup("KIND")
		team, _ := meta.Lookup("INVESTIGATION_TEAM")
		if kind == "investigation" && team == "" {
			if invID, ok := meta.Lookup("INV_ID"); ok && invID != "" {
				agent = "inv#" + invID
			}
		}

		status, ok := meta.Lookup("STATUS")
		if !ok || status == "" {
			status = "?"
		}

		round, ok := meta.Lookup("ROUND")
		if !ok || round == "" {
			round = "-"
		}

		run, ok := meta.Lookup("RUN")
		if !ok || run == "" {
			run = "-"
		}

		repo, _ := meta.Lookup("REPO_NAME")

		if _, err := fmt.Fprintf(out, "%-30s %-12s %-13s %-6s %-5s %s\n",
			taskID, agent, status, round, run, repo); err != nil {
			return fromStorage(StatusStorage, storageErr(err, "write status row"))
		}
	}

	return nil
}
