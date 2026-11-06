package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

// execNotifyCmd executes the notification command or a test fake.
var execNotifyCmd = func(cmdPath, filePath string) error {
	cmd := exec.Command(cmdPath, filePath) //nolint:gosec // G204: the operator configures the local notification command; no shell is involved.
	return cmd.Run()
}

// notifyLead delivers a terminal job notification to the human requester.
// jobStateIcon is the icon, with its space, that a lead's notification puts before its title, so that a device shows
// the job's state at once; none for a state without one
func jobStateIcon(s JobState) string {
	switch s {
	case JobDone:
		return "✅ "
	case JobFailed:
		return "❌ "
	case JobStopped:
		return "⏹ "
	case JobWaiting:
		return "❓ "
	case JobRunning:
		return "🚀 "
	case JobQueued, JobAssigned:
		return "⏳ "
	}
	return ""
}

func (a *leadAPI) notifyLead(job Job) {
	if job.Request.Requester != "human" {
		return
	}
	if a.notifyDir == "" {
		return
	}
	if err := os.MkdirAll(a.notifyDir, 0o750); err != nil {
		warnSideEffect("create lead notify directory", storageErr(err, "cannot create %s", a.notifyDir))
		return
	}
	fileName := fmt.Sprintf("%s-summary.md", job.Request.ID)
	filePath := filepath.Join(a.notifyDir, fileName)

	statusStr := string(job.State)
	var sb strings.Builder
	repoName := job.Request.Repository
	if repoName == "" {
		repoName = "team-agents"
	}
	sb.WriteString(jobStateIcon(job.State) + repoName + "\n") // the state's icon first: a device shows the title first
	sb.WriteString("Repo: " + repoName + "\n")
	sb.WriteString("Job: " + job.Request.ID + "\n")
	sb.WriteString("Status: " + statusStr + "\n")
	if job.Node != "" {
		sb.WriteString("Node: " + job.Node + "\n")
	}
	if job.Branch != "" {
		sb.WriteString("Branch: " + job.Branch + "\n")
	}
	if job.PRURL != "" {
		sb.WriteString("PR: " + job.PRURL + "\n")
	}
	sb.WriteString("Time: " + time.Now().UTC().Format(time.RFC3339) + "\n\n")
	if job.Result != "" {
		res := job.Result
		if len(res) > 1500 {
			res = res[:1500]
		}
		sb.WriteString(res + "\n")
	}

	if err := os.WriteFile(filePath, []byte(sb.String()), 0o600); err != nil {
		warnSideEffect("write lead notification file", storageErr(err, "cannot write %s", filePath))
		return
	}

	if a.notifyCmd != "" {
		if err := execNotifyCmd(a.notifyCmd, filePath); err != nil {
			warnSideEffect("run lead notify command", processErr(a.notifyCmd, err, "cannot run notify command"))
		}
	}
}
