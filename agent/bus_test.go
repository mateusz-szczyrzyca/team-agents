package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
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

// busTestExecutor records the invocation and injects outputs.
type busTestExecutor struct {
	cmd     Command       // recorded command invocation
	stdout  string        // injected standard output
	stderr  string        // injected standard error
	result  CommandResult // injected execution result
	execErr *ExecuteError // injected execution error
}

// Execute records cmd and returns the configured result and output.
func (e *busTestExecutor) Execute(ctx context.Context, cmd Command) (CommandResult, *ExecuteError) {
	_ = ctx
	e.cmd = cmd
	if e.stdout != "" && cmd.Stdout != nil {
		if _, err := cmd.Stdout.Write([]byte(e.stdout)); err != nil {
			return CommandResult{}, fromProcess(ExecuteWait, processErr("busTestExecutor", err, "stdout write failed"))
		}
	}
	if e.stderr != "" && cmd.Stderr != nil {
		if _, err := cmd.Stderr.Write([]byte(e.stderr)); err != nil {
			return CommandResult{}, fromProcess(ExecuteWait, processErr("busTestExecutor", err, "stderr write failed"))
		}
	}
	return e.result, e.execErr
}

func TestScriptNoticeJSON(t *testing.T) {
	var validNotice ScriptNotice
	if err := json.Unmarshal([]byte(`"member_finished"`), &validNotice); err != nil {
		t.Fatalf("unexpected unmarshal error for valid notice: %v", err)
	}
	if validNotice != NoticeMemberFinished {
		t.Errorf("unmarshaled notice = %q, want %q", validNotice, NoticeMemberFinished)
	}

	var unknownNotice ScriptNotice
	if err := json.Unmarshal([]byte(`"unknown_notice"`), &unknownNotice); err == nil {
		t.Fatal("expected error unmarshaling unknown notice, got nil")
	} else {
		var verr *ValueError
		if !errors.As(err, &verr) {
			t.Errorf("expected *ValueError for unknown notice, got %T: %v", err, err)
		}
	}

	var nonStringNotice ScriptNotice
	if err := json.Unmarshal([]byte(`123`), &nonStringNotice); err == nil {
		t.Fatal("expected error unmarshaling non-string notice, got nil")
	} else {
		var verr *ValueError
		if !errors.As(err, &verr) {
			t.Errorf("expected *ValueError for non-string notice, got %T: %v", err, err)
		}
	}
}

func TestBusArgsJSONZeroLimits(t *testing.T) {
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
		QueueMax:   0,
		InvSlots:   0,
		TasksDir:   "/home/dev/agents/tasks",
		Tier:       "normal",
		AgentCmd:   "/home/dev/agents/bin/agent",
		AgentsHome: "/home/dev/agents",
	}
	args := busArgsJSON(params)
	hasQueueMax := false
	hasInvSlots := false
	for i, arg := range args {
		if arg == "--queue-max" && i+1 < len(args) && args[i+1] == "0" {
			hasQueueMax = true
		}
		if arg == "--inv-slots" && i+1 < len(args) && args[i+1] == "0" {
			hasInvSlots = true
		}
	}
	if !hasQueueMax {
		t.Errorf("busArgsJSON with QueueMax=0 did not include --queue-max 0: %v", args)
	}
	if !hasInvSlots {
		t.Errorf("busArgsJSON with InvSlots=0 did not include --inv-slots 0: %v", args)
	}
}

func TestBusMessagesFromLargeRecord(t *testing.T) {
	tmpDir := t.TempDir()
	msgsFile := filepath.Join(tmpDir, "messages.jsonl")

	largeText := strings.Repeat("A", 75*1024)
	record1 := `{"id":1,"from":"agent-1","to":"all","text":"` + largeText + `"}` + "\n"
	record2 := `{"id":2,"from":"agent-1","to":"coordinator","text":"following message"}` + "\n"
	record3 := `{"id":3,"from":"agent-2","to":"all","text":"another agent"}` + "\n"

	if err := os.WriteFile(msgsFile, []byte(record1+record2+record3), 0o600); err != nil {
		t.Fatal(err)
	}

	count, err := busMessagesFrom(msgsFile, "agent-1")
	if err != nil {
		t.Fatalf("busMessagesFrom failed on large record: %v", err)
	}
	if count != 2 {
		t.Errorf("expected count 2 for agent-1, got %d", count)
	}
}

func TestBusMessagesFromStorageError(t *testing.T) {
	tmpDir := t.TempDir()
	_, err := busMessagesFrom(tmpDir, "agent-1")
	if err == nil {
		t.Fatal("expected error reading directory as messages file, got nil")
	}
	if !errors.Is(err, ErrStorage) {
		t.Errorf("expected ErrStorage, got %v", err)
	}
	var storage *StorageError
	if !errors.As(err, &storage) {
		t.Errorf("expected *StorageError, got %T: %v", err, err)
	}
}

func TestBusCmd(t *testing.T) {
	ctx := context.Background()

	execSuccess := &busTestExecutor{
		result: CommandResult{ExitCode: 0},
		stdout: "team-member-1\n",
	}
	appSuccess := newApp(execSuccess, nil, nil)
	out, berr := busCmd(ctx, appSuccess, "/bin/agentbus", "inv-next")
	if berr != nil {
		t.Fatalf("unexpected busCmd error: %v", berr)
	}
	if out != "team-member-1\n" {
		t.Errorf("busCmd stdout = %q, want %q", out, "team-member-1\n")
	}
	if execSuccess.cmd.Name != "/bin/agentbus" || !reflect.DeepEqual(execSuccess.cmd.Args, []string{"inv-next"}) {
		t.Errorf("unexpected command: %+v", execSuccess.cmd)
	}

	execNotFound := &busTestExecutor{
		result: CommandResult{ExitCode: BusExitNotFound},
		stderr: "investigation not found\n",
	}
	appNotFound := newApp(execNotFound, nil, nil)
	_, berr = busCmd(ctx, appNotFound, "/bin/agentbus", "inv-status", "inv-99")
	if berr == nil {
		t.Fatal("expected BusNotFound error, got nil")
	}
	if berr.Cause != BusNotFound {
		t.Errorf("expected cause BusNotFound, got %v", berr.Cause)
	}

	execFail := &busTestExecutor{
		execErr: opErr(ExecuteDenied, "command denied"),
	}
	appFail := newApp(execFail, nil, nil)
	_, berr = busCmd(ctx, appFail, "/bin/agentbus", "status")
	if berr == nil {
		t.Fatal("expected BusProcess error, got nil")
	}
	if berr.Cause != BusProcess {
		t.Errorf("expected cause BusProcess, got %v", berr.Cause)
	}
}

func TestSendNotice(t *testing.T) {
	ctx := context.Background()
	logDir := t.TempDir()
	logger := &Logger{LogDir: logDir, Subcommand: "runner"}

	exec := &busTestExecutor{
		result: CommandResult{ExitCode: 0},
	}
	app := newApp(exec, nil, nil)

	err := sendNotice(ctx, app, logger, "/bin/agentbus", "/home/dev/agents/bus", NoticeMemberFinished, "claude-haiku", "coordinator", "done", "task=task-1", "status=DONE")
	if err != nil {
		t.Fatalf("unexpected sendNotice error: %v", err)
	}

	wantArgs := []string{
		"send",
		"--dir", "/home/dev/agents/bus",
		"--from", "claude-haiku",
		"--to", "coordinator",
		"--notice", "member_finished",
		"--field", "task=task-1",
		"--field", "status=DONE",
		"done",
	}
	if exec.cmd.Name != "/bin/agentbus" || !reflect.DeepEqual(exec.cmd.Args, wantArgs) {
		t.Errorf("sendNotice command = %v %v, want /bin/agentbus %v", exec.cmd.Name, exec.cmd.Args, wantArgs)
	}

	// Verify failure handling is warning-only: non-zero exit from bus send does not return error.
	execFail := &busTestExecutor{
		result: CommandResult{ExitCode: 1},
		stderr: "bus communication failed",
	}
	appFail := newApp(execFail, nil, nil)
	err = sendNotice(ctx, appFail, logger, "/bin/agentbus", "/home/dev/agents/bus", NoticeMemberFailed, "agent-1", "coordinator", "failed")
	if err != nil {
		t.Fatalf("sendNotice should not return error on bus send failure (warning only): %v", err)
	}
}

// busFailWriter is an io.Writer that always fails write operations.
type busFailWriter struct{}

// Write implements io.Writer by always returning io.ErrClosedPipe.
func (busFailWriter) Write([]byte) (int, error) {
	return 0, io.ErrClosedPipe
}

// TestBusTestExecutorStreamErrors verifies busTestExecutor returns ExecuteWait when stream writes fail.
func TestBusTestExecutorStreamErrors(t *testing.T) {
	t.Parallel()

	t.Run("stdout write error", func(t *testing.T) {
		exec := &busTestExecutor{stdout: "hello"}
		cmd := Command{Stdout: busFailWriter{}}
		_, err := exec.Execute(context.Background(), cmd)
		if err == nil {
			t.Fatal("expected ExecuteError on stdout write failure, got nil")
		}
		if err.Cause != ExecuteWait {
			t.Errorf("got cause %v, want ExecuteWait", err.Cause)
		}
		if !errors.Is(err, io.ErrClosedPipe) {
			t.Errorf("expected wrapped io.ErrClosedPipe, got: %v", err)
		}
	})

	t.Run("stderr write error", func(t *testing.T) {
		exec := &busTestExecutor{stderr: "err message"}
		cmd := Command{Stderr: busFailWriter{}}
		_, err := exec.Execute(context.Background(), cmd)
		if err == nil {
			t.Fatal("expected ExecuteError on stderr write failure, got nil")
		}
		if err.Cause != ExecuteWait {
			t.Errorf("got cause %v, want ExecuteWait", err.Cause)
		}
		if !errors.Is(err, io.ErrClosedPipe) {
			t.Errorf("expected wrapped io.ErrClosedPipe, got: %v", err)
		}
	})
}

// TestBusCmdStderrReplayFailure verifies that busCmd returns BusStorage when writing stderr to app.Stderr fails.
func TestBusCmdStderrReplayFailure(t *testing.T) {
	ctx := context.Background()
	exec := &busTestExecutor{
		result: CommandResult{ExitCode: 0},
		stderr: "warning: low disk space\n",
	}
	app := newApp(exec, nil, nil)
	app.Stderr = busFailWriter{}
	_, err := busCmd(ctx, app, "/bin/agentbus", "status")
	if err == nil {
		t.Fatal("expected BusStorage error on stderr write failure, got nil")
	}
	if err.Cause != BusStorage || !errors.Is(err, ErrStorage) {
		t.Fatalf("expected cause BusStorage/ErrStorage, got %v", err)
	}
}

// TestDelegatedBusLaunchFailureExitCode verifies launch failures exit 127 for bus commands.
func TestDelegatedBusLaunchFailureExitCode(t *testing.T) {
	for _, cmd := range [][]string{{"bus", "5"}, {"helpers"}, {"inv", "99"}, {"notes", "testrepo"}} {
		var stdout, stderr bytes.Buffer
		app := newApp(OSExecutor{}, nil, nil)
		app.busBin = "/nonexistent/path/to/agentbus"
		if code := app.run(cmd, &stdout, &stderr); code != 127 {
			t.Fatalf("run(%v) code = %d, want 127", cmd, code)
		}
	}
}

// TestDelegatedBusPermissionFailureExitCode verifies launch permission failures exit 126 for bus commands.
func TestDelegatedBusPermissionFailureExitCode(t *testing.T) {
	tmpDir := t.TempDir()
	nonExecBus := filepath.Join(tmpDir, "agentbus")
	if err := os.WriteFile(nonExecBus, []byte("#!/bin/sh\nexit 0\n"), 0600); err != nil {
		t.Fatal(err)
	}
	for _, cmd := range [][]string{{"bus", "5"}, {"helpers"}, {"inv", "99"}, {"notes", "testrepo"}} {
		var stdout, stderr bytes.Buffer
		app := newApp(OSExecutor{}, nil, nil)
		app.busBin = nonExecBus
		if code := app.run(cmd, &stdout, &stderr); code != 126 {
			t.Fatalf("run(%v) code = %d, want 126", cmd, code)
		}
	}
}
