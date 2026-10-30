package main

import (
	"crypto/sha256"
	"encoding/hex"
	"io"
	"os"
	"os/exec"
	"regexp"
	"time"
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
var execQuestionsCmd = func(cmdPath, target string) error {
	cmd := exec.Command(cmdPath, "_remote_questions", target) //nolint:gosec // G204: operator configured command path; no shell involved.
	return cmd.Run()
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
	_ = args
	return LeadAnswerConfig{}, nil
}

// answer queues a question answer for a waiting remote job.
func (a *leadAPI) answer(node, taskID, file, content string) (LeadAnswer, *LeadAnswerError) {
	_ = node
	_ = taskID
	_ = file
	_ = content
	return LeadAnswer{}, nil
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
	_ = args
	return LeadKickConfig{}, nil
}

// kick queues a kick request for a running or active remote job.
func (a *leadAPI) kick(node, taskID string) (LeadKick, *LeadKickError) {
	_ = node
	_ = taskID
	return LeadKick{}, nil
}

// cmdLeadAnswer reads content from stdin and queues an answer in the lead registry.
func cmdLeadAnswer(args []string) {
	_ = args
	_, _ = io.ReadAll(os.Stdin)
}

// cmdLeadKick queues a kick request in the lead registry.
func cmdLeadKick(args []string) {
	_ = args
}
