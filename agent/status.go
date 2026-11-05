package main

import (
	"context"
	"fmt"
	"io"
	"io/ioutil"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// cmdStatus displays a table of all tasks with their status, agent, round, and run number.
func cmdStatus(ctx context.Context, app *App, out io.Writer) error {
	// Print header
	fmt.Fprintf(out, "%-30s %-12s %-13s %-6s %-5s %s\n",
		"TASK", "AGENT", "STATUS", "ROUND", "RUN", "REPO")

	// Read all tasks from tasksDir
	tasksDir := app.tasksDir
	if tasksDir == "" {
		return nil
	}

	entries, err := ioutil.ReadDir(tasksDir)
	if err != nil && !os.IsNotExist(err) {
		return err
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
		meta := readMetaFile(filepath.Join(tasksDir, taskID, "meta.env"))
		
		agent := meta["AGENT"]
		if agent == "" {
			agent = "?"
		}

		// Special handling for investigation teams
		if meta["KIND"] == "investigation" && meta["INVESTIGATION_TEAM"] == "" {
			if invID := meta["INV_ID"]; invID != "" {
				agent = "inv#" + invID
			}
		}

		status := meta["STATUS"]
		if status == "" {
			status = "?"
		}

		round := meta["ROUND"]
		if round == "" {
			round = "-"
		}

		run := meta["RUN"]
		if run == "" {
			run = "-"
		}

		repo := meta["REPO_NAME"]
		if repo == "" {
			repo = ""
		}

		fmt.Fprintf(out, "%-30s %-12s %-13s %-6s %-5s %s\n",
			taskID, agent, status, round, run, repo)
	}

	return nil
}

// readMetaFile reads a Bash-quoted metadata file and returns the key-value pairs.
func readMetaFile(path string) map[string]string {
	meta := make(map[string]string)
	content, err := ioutil.ReadFile(path)
	if err != nil {
		return meta
	}

	for _, line := range strings.Split(string(content), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		if len(parts) == 2 {
			meta[parts[0]] = unquoteValue(parts[1])
		}
	}
	return meta
}

// unquoteValue unquotes a Bash-quoted metadata value.
func unquoteValue(s string) string {
	s = strings.TrimSpace(s)
	if len(s) >= 2 && s[0] == '\'' && s[len(s)-1] == '\'' {
		return s[1 : len(s)-1]
	}
	if len(s) >= 2 && s[0] == '"' && s[len(s)-1] == '"' {
		return strings.NewReplacer("\\\"", "\"", "\\\\", "\\").Replace(s[1 : len(s)-1])
	}
	return s
}
