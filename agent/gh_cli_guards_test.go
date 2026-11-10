package main

import (
	"bytes"
	"context"
	"io"
	"strings"
	"testing"
)

// TestGHDispatchMemberCloseRefused verifies that a team member cannot run pr close
// and that real gh is never called.
func TestGHDispatchMemberCloseRefused(t *testing.T) {
	var stdout, stderr bytes.Buffer
	calledRealGH := false
	exec := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
			}
			return CommandResult{ExitCode: 0}, nil
		},
	}
	code, err := ghDispatch(context.Background(), GHDispatchParams{
		Args:   []string{"pr", "close", "42"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead-agent"},
		Stdout: &stdout,
		Stderr: &stderr,
		Exec:   exec,
	})
	if calledRealGH {
		t.Fatal("real gh was called on member pr close")
	}
	if code != 1 || err == nil || err.Cause != GHReadDispatchMemberRefused {
		t.Fatalf("code=%d err=%v, want MemberRefused", code, err)
	}
	if !strings.Contains(stderr.String(), "a team member does not close a PR") {
		t.Fatalf("stderr unexpected: %s", stderr.String())
	}
}

// TestGHDispatchDynamicLeadPermitted verifies that when agentbus reports the current agent
// as the team coordinator, pr create is permitted and forwarded to real gh.
func TestGHDispatchDynamicLeadPermitted(t *testing.T) {
	var stdout, stderr bytes.Buffer
	calledRealGH := false
	exec := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			if strings.Contains(cmd.Name, "agentbus") && cmd.Stdout != nil {
				if _, err := io.WriteString(cmd.Stdout, `{"lead":"dynamic-lead"}`); err != nil {
					return CommandResult{ExitCode: 1}, opErr(ExecuteWait, err.Error())
				}
				return CommandResult{ExitCode: 0}, nil
			}
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
			}
			return CommandResult{ExitCode: 0}, nil
		},
	}
	code, err := ghDispatch(context.Background(), GHDispatchParams{
		Args:   []string{"pr", "create", "--base", "main", "--head", "agents/lead/1", "--title", "T", "--body", "B"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_TEAM=alpha", "AGENT_GH_SELF=dynamic-lead", "AGENT_GH_COORDINATOR=old"},
		Stdout: &stdout,
		Stderr: &stderr,
		Exec:   exec,
	})
	if code != 0 || err != nil || !calledRealGH {
		t.Fatalf("code=%d err=%v called=%v, want permitted (0, nil, true)", code, err, calledRealGH)
	}
}

// TestGHDispatchDynamicLeadRefusedWhenNotSelf verifies that dynamic bus coordinator lookup
// correctly refuses a member when another agent is lead.
func TestGHDispatchDynamicLeadRefusedWhenNotSelf(t *testing.T) {
	var stdout, stderr bytes.Buffer
	calledRealGH := false
	exec := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			if strings.Contains(cmd.Name, "agentbus") && cmd.Stdout != nil {
				if _, err := io.WriteString(cmd.Stdout, `{"lead":"other-lead"}`); err != nil {
					return CommandResult{ExitCode: 1}, opErr(ExecuteWait, err.Error())
				}
				return CommandResult{ExitCode: 0}, nil
			}
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
			}
			return CommandResult{ExitCode: 0}, nil
		},
	}
	code, err := ghDispatch(context.Background(), GHDispatchParams{
		Args:   []string{"pr", "create", "--base", "main", "--head", "agents/member/1", "--title", "T", "--body", "B"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_TEAM=alpha", "AGENT_GH_SELF=member-agent"},
		Stdout: &stdout,
		Stderr: &stderr,
		Exec:   exec,
	})
	if calledRealGH || code != 1 || err == nil || err.Cause != GHReadDispatchMemberRefused {
		t.Fatalf("called=%v code=%d err=%v, want MemberRefused (false, 1)", calledRealGH, code, err)
	}
	if !strings.Contains(stderr.String(), "opened by its coordinator (other-lead)") {
		t.Fatalf("stderr unexpected: %s", stderr.String())
	}
}

// TestGHDispatchDynamicLeadFallbackOnBusFailure verifies fallback to AGENT_GH_COORDINATOR
// when agentbus lookup fails.
func TestGHDispatchDynamicLeadFallbackOnBusFailure(t *testing.T) {
	var stdout, stderr bytes.Buffer
	calledRealGH := false
	exec := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			if strings.Contains(cmd.Name, "agentbus") {
				return CommandResult{ExitCode: 1}, opErr(ExecuteStart, "bus failure")
			}
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
			}
			return CommandResult{ExitCode: 0}, nil
		},
	}
	code, err := ghDispatch(context.Background(), GHDispatchParams{
		Args:   []string{"pr", "close", "99"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_TEAM=alpha", "AGENT_GH_SELF=member", "AGENT_GH_COORDINATOR=fallback-lead"},
		Stdout: &stdout,
		Stderr: &stderr,
		Exec:   exec,
	})
	if calledRealGH || code != 1 || err == nil || err.Cause != GHReadDispatchMemberRefused {
		t.Fatalf("called=%v code=%d err=%v, want fallback MemberRefused", calledRealGH, code, err)
	}
	if !strings.Contains(stderr.String(), "fallback-lead") {
		t.Fatalf("stderr unexpected: %s", stderr.String())
	}
}

// TestGHDispatchNonMemberSoloPermitted verifies that non-members (solo runs or coordinators)
// can execute pr create to real gh.
func TestGHDispatchNonMemberSoloPermitted(t *testing.T) {
	var stdout, stderr bytes.Buffer
	calledRealGH := false
	exec := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
			}
			return CommandResult{ExitCode: 0}, nil
		},
	}
	code, err := ghDispatch(context.Background(), GHDispatchParams{
		Args:   []string{"pr", "create", "--base", "main", "--head", "agents/solo/1", "--title", "Solo PR", "--body", "Body"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh"},
		Stdout: &stdout,
		Stderr: &stderr,
		Exec:   exec,
	})
	if code != 0 || err != nil || !calledRealGH {
		t.Fatalf("code=%d err=%v called=%v, want permitted (0, nil, true)", code, err, calledRealGH)
	}
}

// TestGHDispatchOwnPROpenVsMerged verifies that an existing OPEN PR blocks a second pr create,
// while an existing MERGED PR allows a new pr create to proceed to real gh.
func TestGHDispatchOwnPROpenVsMerged(t *testing.T) {
	t.Run("open_pr_refused", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		calledRealGH := false
		exec := fakeGHExec{
			run: func(cmd Command) (CommandResult, *ExecuteError) {
				if len(cmd.Args) >= 4 && cmd.Args[0] == "pr" && cmd.Args[1] == "view" && cmd.Args[2] == "42" && cmd.Stdout != nil {
					if _, err := io.WriteString(cmd.Stdout, `{"state":"OPEN"}`); err != nil {
						return CommandResult{ExitCode: 1}, opErr(ExecuteWait, err.Error())
					}
					return CommandResult{ExitCode: 0}, nil
				}
				if cmd.Name == "/bin/gh" {
					calledRealGH = true
				}
				return CommandResult{ExitCode: 0}, nil
			},
		}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   []string{"pr", "create", "--title", "Second", "--body", "B"},
			Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_OWN_PR=42"},
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		if calledRealGH || code != 1 || err == nil || err.Cause != GHReadDispatchSecondPRRefused {
			t.Fatalf("called=%v code=%d err=%v, want SecondPRRefused", calledRealGH, code, err)
		}
		if !strings.Contains(stderr.String(), "this task's PR is #42 and still open") {
			t.Fatalf("stderr unexpected: %s", stderr.String())
		}
	})

	t.Run("merged_pr_permitted", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		calledRealGH := false
		exec := fakeGHExec{
			run: func(cmd Command) (CommandResult, *ExecuteError) {
				if len(cmd.Args) >= 4 && cmd.Args[0] == "pr" && cmd.Args[1] == "view" && cmd.Args[2] == "42" && cmd.Stdout != nil {
					if _, err := io.WriteString(cmd.Stdout, `{"state":"MERGED"}`); err != nil {
						return CommandResult{ExitCode: 1}, opErr(ExecuteWait, err.Error())
					}
					return CommandResult{ExitCode: 0}, nil
				}
				if cmd.Name == "/bin/gh" {
					calledRealGH = true
				}
				return CommandResult{ExitCode: 0}, nil
			},
		}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   []string{"pr", "create", "--base", "main", "--head", "agents/solo/2", "--title", "Next", "--body", "B"},
			Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_OWN_PR=42"},
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		if code != 0 || err != nil || !calledRealGH {
			t.Fatalf("code=%d err=%v called=%v, want permitted (0, nil, true)", code, err, calledRealGH)
		}
	})
}
