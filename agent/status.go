package main

import (
	"context"
	"fmt"
	"io"
)

// metaLookup returns key from m or fallback when absent or empty.
func metaLookup(m MetaFile, key, fallback string) string {
	if v, _ := m.Lookup(key); v != "" {
		return v
	}
	return fallback
}

// cmdStatus displays a table of all tasks with their status, agent, round, and run number.
func cmdStatus(ctx context.Context, app *App, out io.Writer) *StatusError {
	// Print header
	if _, err := fmt.Fprintf(out, "%-30s %-12s %-13s %-6s %-5s %s\n",
		"TASK", "AGENT", "STATUS", "ROUND", "RUN", "REPO"); err != nil {
		return fromStorage(StatusStorage, storageErr(err, "write status header"))
	}

	if app.tasksDir == "" {
		return nil
	}

	tasks, terr := taskDirectories(app.tasksDir)
	if terr != nil {
		return fromStorage(StatusStorage, terr)
	}

	// Print each task
	for _, taskID := range tasks {
		meta, readErr := readTaskMeta(app.tasksDir, taskID)
		if readErr != nil {
			if readErr.Cause == ReadTaskMetaNotFound {
				continue
			}
			return fromStorage(StatusStorage, storageErr(readErr, "read task meta"))
		}

		id := metaLookup(meta, "ID", taskID)
		agent := metaLookup(meta, "AGENT", "?")
		kind, team := metaLookup(meta, "KIND", ""), metaLookup(meta, "INVESTIGATION_TEAM", "")
		if kind == kindInvestigation && team == "" {
			agent = "inv#" + metaLookup(meta, "INV_ID", "?")
		}
		status := metaLookup(meta, "STATUS", "?")

		// Check if unit is active for running/starting status
		if (status == taskRunning || status == taskStarting) && kind != kindRemoteMember && kind != kindInvestigationTeam {
			runVal := metaLookup(meta, "RUN", "0")
			result, uerr := app.execute(ctx, Command{Name: "systemctl", Args: []string{"--user", "is-active", "--quiet", "agent-" + id + "-r" + runVal + ".service"}})
			if uerr != nil || result.ExitCode != 0 {
				status += "?"
			}
		}

		if _, err := fmt.Fprintf(out, "%-30s %-12s %-13s %-6s %-5s %s\n",
			id, agent, status, metaLookup(meta, "ROUND", "-"), metaLookup(meta, "RUN", "-"), metaLookup(meta, "REPO_NAME", "")); err != nil {
			return fromStorage(StatusStorage, storageErr(err, "write status row"))
		}
	}

	return nil
}
