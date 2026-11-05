package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func TestScriptNoticeValid(t *testing.T) {
	valid := []ScriptNotice{
		NoticeMemberFinished,
		NoticeMemberFailed,
		NoticeMemberStopped,
		NoticeMemberQuestions,
		NoticeMemberNoStatus,
		NoticeTeamStuck,
		NoticeWorkerHung,
		NoticeAgentHung,
		NoticeCoordinatorHung,
		NoticeHelperResult,
		NoticeHelperStartFailure,
		NoticeMemberPaused,
		NoticeMemberBack,
		NoticeMemberOut,
	}
	for _, n := range valid {
		if !n.Valid() {
			t.Errorf("expected %q to be valid", n)
		}
	}
	if ScriptNotice("unknown_notice").Valid() {
		t.Errorf("expected unknown_notice to be invalid")
	}
}

func TestScriptNoticeUnmarshalJSON(t *testing.T) {
	tests := []struct {
		name      string
		input     string
		wantValid bool
		wantErr   bool
	}{
		{"valid_member_finished", `"member_finished"`, true, false},
		{"valid_member_failed", `"member_failed"`, true, false},
		{"unknown_notice", `"unknown_notice"`, false, true},
		{"non_string", `42`, false, true},
		{"invalid_json", `invalid`, false, true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var n ScriptNotice
			err := json.Unmarshal([]byte(tt.input), &n)
			if (err != nil) != tt.wantErr {
				t.Errorf("Unmarshal error = %v, want error %v", err, tt.wantErr)
			}
			if !tt.wantErr && !n.Valid() {
				t.Errorf("expected %q to be valid", n)
			}
		})
	}
}

func TestMapBusExitCode(t *testing.T) {
	tests := []struct {
		code      int
		wantCause BusCause
		wantClass ErrorClass
	}{
		{BusExitInvalidArgument, BusInvalidArgument, ClassInvalidArgument},
		{BusExitNotFound, BusNotFound, ClassNotFound},
		{BusExitNotInTeam, BusNotInTeam, ClassNotInTeam},
		{BusExitPermission, BusPermission, ClassPermission},
		{BusExitConflict, BusConflict, ClassConflict},
		{BusExitInvalidState, BusInvalidState, ClassInvalidState},
		{BusExitStorage, BusStorage, ClassStorage},
		{BusExitUnknownTool, BusUnknownTool, ClassUnknownTool},
		{BusExitProcess, BusProcess, ClassProcess},
		{1, BusStorage, ClassStorage},
		{42, BusStorage, ClassStorage},
	}

	for _, tt := range tests {
		err := mapBusExitCode(tt.code, "some error")
		if err == nil {
			t.Fatalf("mapBusExitCode(%d) returned nil", tt.code)
		}
		if err.Cause != tt.wantCause {
			t.Errorf("mapBusExitCode(%d) cause = %v, want %v", tt.code, err.Cause, tt.wantCause)
		}
		if err.Class() != tt.wantClass {
			t.Errorf("mapBusExitCode(%d) class = %v, want %v", tt.code, err.Class(), tt.wantClass)
		}
		if !errors.Is(err, err.Class().Sentinel()) {
			t.Errorf("expected errors.Is with sentinel for %v", err.Class())
		}
	}

	if err := mapBusExitCode(BusExitSuccess, ""); err != nil {
		t.Errorf("expected nil for BusExitSuccess, got %v", err)
	}
}

func TestBusArgsJSONLocal(t *testing.T) {
	params := BusServeParams{
		BusDir:     "/home/dev/agents/bus",
		Agent:      "claude-haiku",
		TaskID:     "task-123",
		TaskDir:    "/home/dev/agents/tasks/task-123",
		RepoName:   "team-agents",
		Branch:     "agents/claude-haiku/1",
		Worktree:   "/home/dev/agents/worktrees/wt-123",
		Peers:      "claude-haiku,codex-1,gemini-1",
		Role:       "worker",
		QueueMax:   4,
		InvSlots:   2,
		TasksDir:   "/home/dev/agents/tasks",
		Team:       "team-xyz",
		Tier:       "normal",
		AgentCmd:   "/home/dev/agents/bin/agent",
		AgentsHome: "/home/dev/agents",
	}

	want := []string{
		"serve",
		"--dir", "/home/dev/agents/bus",
		"--agent", "claude-haiku",
		"--task", "task-123",
		"--task-dir", "/home/dev/agents/tasks/task-123",
		"--repo", "team-agents",
		"--branch", "agents/claude-haiku/1",
		"--worktree", "/home/dev/agents/worktrees/wt-123",
		"--peers", "claude-haiku,codex-1,gemini-1",
		"--role", "worker",
		"--queue-max", "4",
		"--inv-slots", "2",
		"--tasks-dir", "/home/dev/agents/tasks",
		"--team", "team-xyz",
		"--tier", "normal",
		"--agent-cmd", "/home/dev/agents/bin/agent",
		"--home", "/home/dev/agents",
	}

	got := busArgsJSON(params)
	if !reflect.DeepEqual(got, want) {
		t.Errorf("busArgsJSON() =\n%v\nwant:\n%v", got, want)
	}
}

func TestBusArgsJSONZeroLimits(t *testing.T) {
	params := BusServeParams{
		BusDir:     "/home/dev/agents/bus",
		Agent:      "claude-haiku",
		QueueMax:   0,
		InvSlots:   0,
		AgentsHome: "/home/dev/agents",
	}

	got := busArgsJSON(params)
	wantFlags := map[string]bool{
		"--queue-max": false,
		"--inv-slots": false,
	}

	for i := 0; i < len(got); i++ {
		switch got[i] {
		case "--queue-max":
			wantFlags["--queue-max"] = true
			if i+1 < len(got) && got[i+1] != "0" {
				t.Errorf("expected --queue-max 0, got --queue-max %s", got[i+1])
			}
		case "--inv-slots":
			wantFlags["--inv-slots"] = true
			if i+1 < len(got) && got[i+1] != "0" {
				t.Errorf("expected --inv-slots 0, got --inv-slots %s", got[i+1])
			}
		}
	}

	for flag, found := range wantFlags {
		if !found {
			t.Errorf("expected %s flag to be present even with zero value", flag)
		}
	}
}

func TestBusArgsJSONRemote(t *testing.T) {
	params := BusServeParams{
		RemoteTeam: true,
		JobLead:    "https://lead.example.com",
		Member:     "remote-member-1",
		AgentsHome: "/home/dev/agents",
	}

	want := []string{
		"serve",
		"--relay", "https://lead.example.com",
		"--member", "remote-member-1",
		"--home", "/home/dev/agents",
	}

	got := busArgsJSON(params)
	if !reflect.DeepEqual(got, want) {
		t.Errorf("busArgsJSON(remote) =\n%v\nwant:\n%v", got, want)
	}
}

func TestBusMessagesFrom(t *testing.T) {
	tmpDir := t.TempDir()
	msgsFile := filepath.Join(tmpDir, "messages.jsonl")

	count, err := busMessagesFrom(msgsFile, "agent-1")
	if err != nil {
		t.Fatalf("unexpected error for missing file: %v", err)
	}
	if count != 0 {
		t.Errorf("expected count 0 for missing file, got %d", count)
	}

	content := `{"id":1,"from":"agent-1","to":"all","text":"hello"}
{"id":2,"from":"agent-2","to":"all","text":"hi"}
{"id":3,"from":"agent-1","to":"agent-2","text":"question"}
{"id":4,"from":"coordinator","to":"agent-1","text":"reply"}
`
	if err := os.WriteFile(msgsFile, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}

	count, err = busMessagesFrom(msgsFile, "agent-1")
	if err != nil {
		t.Fatalf("busMessagesFrom failed: %v", err)
	}
	if count != 2 {
		t.Errorf("expected count 2 for agent-1, got %d", count)
	}

	count, err = busMessagesFrom(msgsFile, "agent-3")
	if err != nil {
		t.Fatalf("busMessagesFrom failed: %v", err)
	}
	if count != 0 {
		t.Errorf("expected count 0 for agent-3, got %d", count)
	}
}

func TestBusMessagesFromLargeMessage(t *testing.T) {
	tmpDir := t.TempDir()
	msgsFile := filepath.Join(tmpDir, "messages.jsonl")

	largeBody := string(make([]byte, 70*1024)) + "x"
	for i := range largeBody {
		largeBody = string([]byte{byte(i % 256)}) + largeBody
	}

	content := `{"id":1,"from":"agent-1","to":"all","text":"hello"}
{"id":2,"from":"agent-1","to":"all","body":"` + largeBody + `"}
{"id":3,"from":"agent-1","to":"all","text":"world"}
`
	if err := os.WriteFile(msgsFile, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}

	count, err := busMessagesFrom(msgsFile, "agent-1")
	if err != nil {
		t.Fatalf("busMessagesFrom failed with large message: %v", err)
	}
	if count != 3 {
		t.Errorf("expected count 3 with large message, got %d", count)
	}
}

func TestBusMessagesFromReturnsTypedError(t *testing.T) {
	tmpDir := t.TempDir()
	msgsFile := filepath.Join(tmpDir, "missing_dir", "messages.jsonl")

	count, err := busMessagesFrom(msgsFile, "agent-1")
	if err == nil {
		t.Errorf("expected error for non-existent directory, got nil")
	}
	if count != 0 {
		t.Errorf("expected count 0 on error, got %d", count)
	}
	if !errors.Is(err, ErrStorage) {
		t.Errorf("expected error to be a storage error, got %T: %v", err, err)
	}
}
