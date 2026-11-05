package main

import (
	"context"
	"fmt"
	"io"
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

	tasks, terr := taskDirectories(tasksDir)
	if terr != nil {
		return fromStorage(StatusStorage, terr)
	}

	// Print each task
	for _, taskID := range tasks {
		meta, readErr := readTaskMeta(tasksDir, taskID)
		if readErr != nil {
			if readErr.Cause == ReadTaskMetaNotFound {
				continue
			}
			return fromStorage(StatusStorage, storageErr(readErr, "read task meta"))
		}

		id, ok := meta.Lookup("ID")
		if !ok || id == "" {
			id = taskID
		}

		agent, ok := meta.Lookup("AGENT")
		if !ok || agent == "" {
			agent = "?"
		}

		// Special handling for investigation teams
		kind, _ := meta.Lookup("KIND")
		team, _ := meta.Lookup("INVESTIGATION_TEAM")
		if kind == "investigation" && team == "" {
			invID, _ := meta.Lookup("INV_ID")
			if invID == "" {
				invID = "?"
			}
			agent = "inv#" + invID
		}

		status, ok := meta.Lookup("STATUS")
		if !ok || status == "" {
			status = "?"
		}

		// Check if unit is active for running/starting status
		if (status == "running" || status == "starting") && kind != "remote-member" && kind != "investigation_team" {
			runVal, _ := meta.Lookup("RUN")
			if runVal == "" {
				runVal = "0"
			}
			unitName := "agent-" + id + "-r" + runVal + ".service"
			cmd := Command{
				Name: "systemctl",
				Args: []string{"--user", "is-active", "--quiet", unitName},
			}
			result, uerr := app.Executor.Execute(context.Background(), cmd)
			if uerr != nil {
				return opErr(StatusProcess, "check unit active %s: %s", id, uerr.Error())
			}
			// If unit is inactive, append "?" suffix
			if result.ExitCode != 0 {
				status = status + "?"
			}
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
			id, agent, status, round, run, repo); err != nil {
			return fromStorage(StatusStorage, storageErr(err, "write status row"))
		}
	}

	return nil
}
