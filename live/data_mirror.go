package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"time"
)

// mirrorDirectories lists real child directories; files and symlinks cannot become task identities.
func mirrorDirectories(dir string) []os.DirEntry {
	entries, err := os.ReadDir(dir)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		warnOnce("mirrors:"+dir, LogTasksListFailed, storageErr(err, "cannot list %s", dir), "dir", dir)
		return nil
	}
	var dirs []os.DirEntry
	for _, entry := range entries {
		if entry.IsDir() {
			dirs = append(dirs, entry)
		}
	}
	return dirs
}

// loadMirroredTasks reads display-only mirror metadata and joins jobs without executing node data.
func loadMirroredTasks(s *Snapshot, busDir string, ttl time.Duration) {
	root := filepath.Join(busDir, "remote")
	nodes := mirrorDirectories(root)
	if len(nodes) == 0 {
		return
	}
	jobs, jerr := readWebJobs(busDir)
	if jerr != nil {
		warnOnce("mirror-jobs:"+busDir, LogBusStateUnreadable, storageErr(jerr, "cannot read mirror job registry"), "dir", busDir)
		jobs = &webJobsSnapshot{}
	}
	for _, node := range nodes {
		for _, entry := range mirrorDirectories(filepath.Join(root, node.Name())) {
			dir := filepath.Join(root, node.Name(), entry.Name())
			path := filepath.Join(dir, "metadata.json")
			data, err := os.ReadFile(path) // #nosec G304 -- path consists of configured bus root and directory names listed locally
			if errors.Is(err, os.ErrNotExist) {
				continue
			}
			if err != nil {
				warnOnce("mirror-read:"+path, LogFileReadFailed, storageErr(err, "cannot read %s", path), "file", path)
				continue
			}
			var meta map[string]string
			if err := json.Unmarshal(data, &meta); err != nil {
				warnOnce("mirror-decode:"+path, LogMetaMalformed, valueWrap(err, "cannot decode %s", path), "file", path)
				continue
			}
			if len(meta) == 0 {
				continue
			}
			task := &Task{
				ID: entry.Name() + "@" + node.Name(), Node: node.Name(), Kind: "worker", Dir: dir,
				Agent: meta["AGENT"], Type: meta["TYPE"], Model: meta["MODEL"], Note: meta["NOTE"],
				Status: meta["STATUS"], Run: parseCount(path, "RUN", meta["RUN"]), JobID: meta["JOB_ID"],
				Created: parseTime(meta["CREATED"]), Finished: parseTime(meta["FINISHED"]), RunStart: parseTime(meta["RUN_START"]),
				NodeOffline: webNodeOffline(jobs.Nodes[node.Name()], s.At, ttl),
			}
			for _, job := range jobs.Jobs {
				if job.Request.ID == task.JobID && job.Node == node.Name() {
					task.Repo, task.Branch, task.Team = job.Request.Repository, job.Branch, job.Request.Team
					break
				}
			}
			if activity := readActivity(dir, 1); len(activity) > 0 {
				task.Last = activity[0]
			}
			s.All[task.ID] = task
			s.Tasks = append(s.Tasks, task)
		}
	}
}
