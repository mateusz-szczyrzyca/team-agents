package main

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"
)

// LeadAnswer carries one queued question answer destined for a node's local task.
type LeadAnswer struct {
	Job     string `json:"Job"`     // owning job identifier
	TaskID  string `json:"TaskID"`  // node-local task identifier
	File    string `json:"File"`    // questions file basename (questions-<n>.md)
	Content string `json:"Content"` // answer file content (at most 256 KiB, ends with **ANSWERED**)
	SHA256  string `json:"SHA256"`  // lowercase hex digest of Content
}

// LeadKick carries one queued kick request destined for a node's local task.
type LeadKick struct {
	Job    string `json:"Job"`    // owning job identifier
	TaskID string `json:"TaskID"` // node-local task identifier
	Run    int    `json:"Run"`    // task run number on the node
	SHA256 string `json:"SHA256"` // occurrence digest of this kick
}

// LeadAnswerAck acknowledges delivery of one answer by the node.
type LeadAnswerAck struct {
	Job    string `json:"Job"`    // owning job identifier
	TaskID string `json:"TaskID"` // node-local task identifier
	File   string `json:"File"`   // questions file basename
	SHA256 string `json:"SHA256"` // content digest of the acknowledged answer
}

// LeadKickAck acknowledges execution of one kick by the node.
type LeadKickAck struct {
	Job    string `json:"Job"`    // owning job identifier
	TaskID string `json:"TaskID"` // node-local task identifier
	Run    int    `json:"Run"`    // task run number on the node
	SHA256 string `json:"SHA256"` // occurrence digest of the acknowledged kick
}

// Max constants for heartbeat controls.
const (
	MaxHeartbeatAnswers = 2  // MaxHeartbeatAnswers is the maximum answers returned in one heartbeat reply.
	MaxHeartbeatKicks   = 64 // MaxHeartbeatKicks is the maximum kicks returned in one heartbeat reply.
)

// QuestionsFilePattern validates the basename of a questions file (questions-<n>.md).
var QuestionsFilePattern = regexp.MustCompile(`^questions-[1-9][0-9]*\.md$`)

// execQuestionsCmd executes the questions command hook or a test fake.
var execQuestionsCmd = func(cmdPath, target string) *ProcessError {
	cmd := exec.Command(cmdPath, "_remote_questions", target) //nolint:gosec // G204: operator configured command path; no shell involved.
	if err := cmd.Run(); err != nil {
		return processErr(cmdPath, err, "cannot run questions command")
	}
	return nil
}

// sha256Hex returns the lowercase hex representation of the SHA-256 digest of s.
func sha256Hex(s string) string {
	h := sha256.Sum256([]byte(s))
	return hex.EncodeToString(h[:])
}

// LeadAnswerConfig holds parsed flags for agentbus lead-answer.
type LeadAnswerConfig struct {
	Dir  string        // bus directory
	Node string        // remote node name
	Task string        // node-local task ID
	File string        // questions file name (questions-<n>.md)
	TTL  time.Duration // node liveness window
}

// parseLeadAnswerConfig parses flags for agentbus lead-answer.
func parseLeadAnswerConfig(args []string) (LeadAnswerConfig, *LeadAnswerError) {
	fs := newFlagSet("lead-answer")
	dir := fs.String("dir", defaultDir(), "the bus directory")
	node := fs.String("node", "", "remote node name")
	task := fs.String("task", "", "node-local task ID")
	file := fs.String("file", "", "questions file name (questions-<n>.md)")
	ttl := fs.Duration("ttl", 90*time.Second, "how long a silent node stays online")
	if err := fs.Parse(args); err != nil {
		return LeadAnswerConfig{}, opErr(LeadAnswerBadFlags, "%v", err)
	}
	if fs.NArg() != 0 || *node == "" || *task == "" || *file == "" {
		return LeadAnswerConfig{}, opErr(LeadAnswerBadFlags, "usage: agentbus lead-answer [--dir D] --node N --task T --file F [--ttl T]")
	}
	if *ttl <= 0 {
		return LeadAnswerConfig{}, opErr(LeadAnswerBadFlags, "--ttl %v must be a positive duration", *ttl)
	}
	if !validJobIdentifier(*node) {
		return LeadAnswerConfig{}, opErr(LeadAnswerBadFlags, "invalid node identifier %q", *node)
	}
	if !validJobIdentifier(*task) {
		return LeadAnswerConfig{}, opErr(LeadAnswerBadFlags, "invalid task identifier %q", *task)
	}
	if !QuestionsFilePattern.MatchString(*file) {
		return LeadAnswerConfig{}, opErr(LeadAnswerInvalidFile, "invalid questions file %q: must match questions-<n>.md", *file)
	}
	return LeadAnswerConfig{Dir: *dir, Node: *node, Task: *task, File: *file, TTL: *ttl}, nil
}

// answer queues a question answer for a waiting remote job.
func (a *leadAPI) answer(node, taskID, file, content string) (LeadAnswer, *LeadAnswerError) {
	if !validJobIdentifier(node) {
		return LeadAnswer{}, opErr(LeadAnswerBadFlags, "invalid node identifier %q", node)
	}
	if !validJobIdentifier(taskID) {
		return LeadAnswer{}, opErr(LeadAnswerBadFlags, "invalid task identifier %q", taskID)
	}
	if !QuestionsFilePattern.MatchString(file) {
		return LeadAnswer{}, opErr(LeadAnswerInvalidFile, "invalid questions file %q", file)
	}
	if len(content) > syncFileBytes {
		return LeadAnswer{}, opErr(LeadAnswerContentTooLarge, "answer content exceeds %d bytes", syncFileBytes)
	}
	trimmed := strings.TrimRight(content, "\r\n \t")
	lastNewline := strings.LastIndex(trimmed, "\n")
	lastLine := trimmed
	if lastNewline >= 0 {
		lastLine = strings.TrimSpace(trimmed[lastNewline+1:])
	}
	if lastLine != "**ANSWERED**" {
		return LeadAnswer{}, opErr(LeadAnswerNotAnswered, "content must end with **ANSWERED** on its own final line")
	}
	if !utf8.ValidString(content) {
		return LeadAnswer{}, opErr(LeadAnswerInvalidContent, "content is not valid UTF-8")
	}
	for _, r := range content {
		if r < 0x20 && r != '\n' && r != '\r' && r != '\t' {
			return LeadAnswer{}, opErr(LeadAnswerInvalidContent, "content contains non-printable control characters")
		}
	}

	var result LeadAnswer
	err := transactJobs(a.bus, LeadAnswerStorage, Timestamp{time.Now().UTC()}, func(r *leadRegistry) *LeadAnswerError {
		var matched *Job
		for _, j := range r.jobs {
			if j.Node == node && j.TaskID == taskID {
				matched = j
				break
			}
		}
		if matched == nil {
			return opErr(LeadAnswerNoSuchJob, "no job found for node %q and task %q", node, taskID)
		}
		if matched.State != JobWaiting {
			return opErr(LeadAnswerInvalidState, "job %s is not waiting (state %s)", matched.Request.ID, matched.State)
		}

		sha := sha256Hex(content)
		for _, existing := range matched.PendingAnswers {
			if existing.File == file && existing.SHA256 == sha {
				result = existing
				return nil
			}
		}

		result = LeadAnswer{
			Job:     matched.Request.ID,
			TaskID:  taskID,
			File:    file,
			Content: content,
			SHA256:  sha,
		}
		matched.PendingAnswers = []LeadAnswer{result}
		r.jobs[matched.Request.ID] = matched
		return nil
	})
	if err != nil {
		return LeadAnswer{}, err
	}
	return result, nil
}

// LeadKickConfig holds parsed flags for agentbus lead-kick.
type LeadKickConfig struct {
	Dir  string        // bus directory
	Node string        // remote node name
	Task string        // node-local task ID
	TTL  time.Duration // node liveness window
}

// parseLeadKickConfig parses flags for agentbus lead-kick.
func parseLeadKickConfig(args []string) (LeadKickConfig, *LeadKickError) {
	fs := newFlagSet("lead-kick")
	dir := fs.String("dir", defaultDir(), "the bus directory")
	node := fs.String("node", "", "remote node name")
	task := fs.String("task", "", "node-local task ID")
	ttl := fs.Duration("ttl", 90*time.Second, "how long a silent node stays online")
	if err := fs.Parse(args); err != nil {
		return LeadKickConfig{}, opErr(LeadKickBadFlags, "%v", err)
	}
	if fs.NArg() != 0 || *node == "" || *task == "" {
		return LeadKickConfig{}, opErr(LeadKickBadFlags, "usage: agentbus lead-kick [--dir D] --node N --task T [--ttl T]")
	}
	if *ttl <= 0 {
		return LeadKickConfig{}, opErr(LeadKickBadFlags, "--ttl %v must be a positive duration", *ttl)
	}
	if !validJobIdentifier(*node) {
		return LeadKickConfig{}, opErr(LeadKickBadFlags, "invalid node identifier %q", *node)
	}
	if !validJobIdentifier(*task) {
		return LeadKickConfig{}, opErr(LeadKickBadFlags, "invalid task identifier %q", *task)
	}
	return LeadKickConfig{Dir: *dir, Node: *node, Task: *task, TTL: *ttl}, nil
}

// kick queues a kick request for a running or active remote job.
func (a *leadAPI) kick(node, taskID string) (LeadKick, *LeadKickError) {
	if !validJobIdentifier(node) {
		return LeadKick{}, opErr(LeadKickBadFlags, "invalid node identifier %q", node)
	}
	if !validJobIdentifier(taskID) {
		return LeadKick{}, opErr(LeadKickBadFlags, "invalid task identifier %q", taskID)
	}

	var result LeadKick
	err := transactJobs(a.bus, LeadKickStorage, Timestamp{time.Now().UTC()}, func(r *leadRegistry) *LeadKickError {
		var matched *Job
		for _, j := range r.jobs {
			if j.Node == node && j.TaskID == taskID {
				matched = j
				break
			}
		}
		if matched == nil {
			return opErr(LeadKickNoSuchJob, "no job found for node %q and task %q", node, taskID)
		}
		if matched.State != JobRunning {
			return opErr(LeadKickInvalidState, "job %s is not running (state %s)", matched.Request.ID, matched.State)
		}

		metaPath := filepath.Join(a.bus.dir, "remote", node, taskID, "metadata.json")
		meta := make(map[SyncMetadataKey]string)
		if serr := readSyncJSON(metaPath, &meta); serr != nil {
			return fromStorage(LeadKickStorage, serr)
		}
		if len(meta) == 0 {
			return opErr(LeadKickInvalidState, "job %s task %s has no mirror metadata", matched.Request.ID, taskID)
		}
		if st, ok := meta[SyncStatus]; ok && st != "" && st != string(JobRunning) && st != "starting" {
			return opErr(LeadKickInvalidState, "job %s task %s is not running (metadata status %s)", matched.Request.ID, taskID, st)
		}
		rStr, ok := meta[SyncRun]
		if !ok || rStr == "" {
			return opErr(LeadKickInvalidState, "job %s task %s mirror metadata missing RUN", matched.Request.ID, taskID)
		}
		run, rerr := strconv.Atoi(rStr)
		if rerr != nil || run <= 0 {
			return opErr(LeadKickInvalidState, "job %s task %s mirror metadata has invalid RUN %q", matched.Request.ID, taskID, rStr)
		}

		sha := sha256Hex(fmt.Sprintf("%s:%s:%d", matched.Request.ID, taskID, run))
		for _, existing := range matched.PendingKicks {
			if existing.Run == run && existing.SHA256 == sha {
				result = existing
				return nil
			}
		}

		result = LeadKick{
			Job:    matched.Request.ID,
			TaskID: taskID,
			Run:    run,
			SHA256: sha,
		}
		matched.PendingKicks = []LeadKick{result}
		r.jobs[matched.Request.ID] = matched
		return nil
	})
	if err != nil {
		return LeadKick{}, err
	}
	return result, nil
}

// cmdLeadAnswer reads content from stdin and queues an answer in the lead registry.
func cmdLeadAnswer(args []string) {
	cfg, err := parseLeadAnswerConfig(args)
	if err != nil {
		exitOn(err.asError())
		return
	}
	data, rerr := io.ReadAll(io.LimitReader(os.Stdin, syncFileBytes+1))
	if rerr != nil {
		exitOn(opErr(LeadAnswerStorage, "cannot read answer from stdin: %v", rerr).asError())
		return
	}
	a := &leadAPI{bus: &Bus{dir: cfg.Dir}, ttl: cfg.TTL}
	ans, aerr := a.answer(cfg.Node, cfg.Task, cfg.File, string(data))
	if aerr != nil {
		exitOn(aerr.asError())
		return
	}
	if serr := printJSON(ans); serr != nil {
		exitOn(fromStorage(LeadAnswerStorage, serr).asError())
	}
}

// cmdLeadKick queues a kick request in the lead registry.
func cmdLeadKick(args []string) {
	cfg, err := parseLeadKickConfig(args)
	if err != nil {
		exitOn(err.asError())
		return
	}
	a := &leadAPI{bus: &Bus{dir: cfg.Dir}, ttl: cfg.TTL}
	kick, kerr := a.kick(cfg.Node, cfg.Task)
	if kerr != nil {
		exitOn(kerr.asError())
		return
	}
	if serr := printJSON(kick); serr != nil {
		exitOn(fromStorage(LeadKickStorage, serr).asError())
	}
}
