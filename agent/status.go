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
	if _, err := fmt.Fprintf(out, "%-30s %-12s %-13s %-6s %-5s %s\n", "TASK", "AGENT", "STATUS", "ROUND", "RUN", "REPO"); err != nil {
		return fromStorage(StatusStorage, storageErr(err, "write status header"))
	}
	if app.tasksDir == "" {
		return nil
	}
	tasks, terr := taskDirectories(app.tasksDir)
	if terr != nil {
		return fromStorage(StatusStorage, terr)
	}
	for _, taskID := range tasks {
		meta, readErr := readTaskMeta(app.tasksDir, taskID)
		if readErr != nil {
			if readErr.Cause != ReadTaskMetaNotFound {
				return fromStorage(StatusStorage, storageErr(readErr, "read task meta"))
			}
			continue
		}
		id := metaLookup(meta, "ID", taskID)
		agent, kind := metaLookup(meta, "AGENT", "?"), metaLookup(meta, "KIND", "")
		if kind == kindInvestigation && metaLookup(meta, "INVESTIGATION_TEAM", "") == "" {
			agent = "inv#" + metaLookup(meta, "INV_ID", "?")
		}
		status := metaLookup(meta, "STATUS", "?")
		if (status == taskRunning || status == taskStarting) && kind != kindRemoteMember && kind != kindInvestigationTeam {
			res, uerr := app.execute(ctx, Command{Name: "systemctl", Args: []string{"--user", "is-active", "--quiet", "agent-" + id + "-r" + metaLookup(meta, "RUN", "0") + ".service"}})
			if uerr != nil || res.ExitCode != 0 {
				status += "?"
			}
		}
		if _, err := fmt.Fprintf(out, "%-30s %-12s %-13s %-6s %-5s %s\n", id, agent, status, metaLookup(meta, "ROUND", "-"), metaLookup(meta, "RUN", "-"), metaLookup(meta, "REPO_NAME", "")); err != nil {
			return fromStorage(StatusStorage, storageErr(err, "write status row"))
		}
	}
	return nil
}
