package main

import (
	"bytes"
	"context"
	"errors"
	"os"
	"strings"
	"testing"
)

// fakeGHExec implements Executor for ghDispatch unit tests without network.
type fakeGHExec struct {
	run func(cmd Command) (CommandResult, *ExecuteError)
}

func (f fakeGHExec) Execute(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
	if f.run != nil {
		return f.run(cmd)
	}
	return CommandResult{ExitCode: 0}, nil
}

// TestGHDispatchEarlyRefusals verifies pre-dispatch argument checks.
func TestGHDispatchEarlyRefusals(t *testing.T) {
	tests := []struct {
		name      string
		env       []string
		args      []string
		wantCode  int
		wantCause GHReadDispatchCause
		wantText  string
	}{
		{
			name:      "missing_real_gh",
			env:       nil,
			args:      []string{"status"},
			wantCode:  127,
			wantCause: GHReadDispatchMissingRealGH,
			wantText:  "AGENT_REAL_GH",
		},
		{
			name:      "template_flag",
			env:       []string{"AGENT_REAL_GH=/bin/gh"},
			args:      []string{"pr", "view", "-t", "{{.title}}"},
			wantCode:  2,
			wantCause: GHReadDispatchTemplateRefused,
			wantText:  "--template is not available",
		},
		{
			name:      "web_flag",
			env:       []string{"AGENT_REAL_GH=/bin/gh"},
			args:      []string{"pr", "view", "--web"},
			wantCode:  2,
			wantCause: GHReadDispatchWebRefused,
			wantText:  "--web is not available",
		},
		{
			name:      "graphql_protected_field",
			env:       []string{"AGENT_REAL_GH=/bin/gh"},
			args:      []string{"api", "graphql", "-f", "query={ repository { pullRequest { reviewThreads } } }"},
			wantCode:  1,
			wantCause: GHReadDispatchGraphQLRefused,
			wantText:  "GraphQL query for comments, reviews or bodies is refused",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			code, err := ghDispatch(context.Background(), GHDispatchParams{
				Args:   tc.args,
				Env:    tc.env,
				Stdout: &stdout,
				Stderr: &stderr,
				Exec:   fakeGHExec{},
			})
			if code != tc.wantCode {
				t.Fatalf("exit code = %d, want %d", code, tc.wantCode)
			}
			if err == nil || err.Cause != tc.wantCause {
				t.Fatalf("cause = %v, want %v", err, tc.wantCause)
			}
			if !strings.Contains(stderr.String(), tc.wantText) {
				t.Fatalf("stderr %q does not contain %q", stderr.String(), tc.wantText)
			}
		})
	}
}

// TestGHDispatchMemberRefused verifies team member PR mutation refusals.
func TestGHDispatchMemberRefused(t *testing.T) {
	tests := []struct {
		name     string
		env      []string
		args     []string
		wantText string
	}{
		{
			name:     "pr_create_with_lead",
			env:      []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead-agent"},
			args:     []string{"pr", "create"},
			wantText: "opened by its coordinator (lead-agent)",
		},
		{
			name:     "pr_merge_with_lead",
			env:      []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead-agent"},
			args:     []string{"pr", "merge", "1"},
			wantText: "its coordinator's (lead-agent)",
		},
		{
			name:     "pr_ready_with_lead",
			env:      []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead-agent"},
			args:     []string{"pr", "ready", "1"},
			wantText: "mark ready",
		},
		{
			name:     "api_pr_merge",
			env:      []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead-agent"},
			args:     []string{"api", "-X", "PUT", "repos/o/r/pulls/1/merge"},
			wantText: "through gh api",
		},
		{
			name:     "team_without_lead_refusal",
			env:      []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_TEAM=myteam", "AGENT_GH_SELF=gemini-1"},
			args:     []string{"pr", "create"},
			wantText: "your team has no coordinator yet",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			exec := fakeGHExec{
				run: func(cmd Command) (CommandResult, *ExecuteError) {
					if cmd.Name != "/bin/gh" && strings.Contains(cmd.Name, "agentbus") {
						if cmd.Stdout != nil {
							if n, err := cmd.Stdout.Write([]byte(`{"lead":""}`)); err != nil || n == 0 {
								t.Fatalf("failed to write lead response: %v", err)
							}
						}
						return CommandResult{ExitCode: 0}, nil
					}
					return CommandResult{ExitCode: 0}, nil
				},
			}
			code, err := ghDispatch(context.Background(), GHDispatchParams{
				Args:   tc.args,
				Env:    tc.env,
				Stdout: &stdout,
				Stderr: &stderr,
				Exec:   exec,
			})
			if code != 1 {
				t.Fatalf("exit code = %d, want 1", code)
			}
			if err == nil || err.Cause != GHReadDispatchMemberRefused {
				t.Fatalf("cause = %v, want GHReadDispatchMemberRefused", err)
			}
			if !strings.Contains(stderr.String(), tc.wantText) {
				t.Fatalf("stderr %q does not contain %q", stderr.String(), tc.wantText)
			}
		})
	}
}

// TestGHDispatchSecondPRRefused verifies refusal when the task already has an open PR.
func TestGHDispatchSecondPRRefused(t *testing.T) {
	var stdout, stderr bytes.Buffer
	exec := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			if len(cmd.Args) >= 4 && cmd.Args[0] == "pr" && cmd.Args[1] == "view" && cmd.Args[2] == "42" {
				if cmd.Stdout != nil {
					if n, err := cmd.Stdout.Write([]byte(`{"state":"OPEN"}`)); err != nil || n == 0 {
						t.Fatalf("failed to write state response: %v", err)
					}
				}
				return CommandResult{ExitCode: 0}, nil
			}
			return CommandResult{ExitCode: 0}, nil
		},
	}
	code, err := ghDispatch(context.Background(), GHDispatchParams{
		Args:   []string{"pr", "create"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_OWN_PR=42"},
		Stdout: &stdout,
		Stderr: &stderr,
		Exec:   exec,
	})
	if code != 1 || err == nil || err.Cause != GHReadDispatchSecondPRRefused {
		t.Fatalf("code=%d, err=%v, want SecondPRRefused", code, err)
	}
	if !strings.Contains(stderr.String(), "PR is #42 and still open") {
		t.Fatalf("stderr unexpected: %s", stderr.String())
	}
}

// TestGHDispatchWriteUnavailable verifies fail-closed refusal of write mutations until R12.
func TestGHDispatchWriteUnavailable(t *testing.T) {
	tests := []struct {
		name string
		args []string
	}{
		{"api_post_mutation", []string{"api", "-X", "POST", "repos/o/r/issues", "-f", "title=Bug"}},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
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
				Args:   tc.args,
				Env:    []string{"AGENT_REAL_GH=/bin/gh"},
				Stdout: &stdout,
				Stderr: &stderr,
				Exec:   exec,
			})
			if calledRealGH {
				t.Fatalf("real gh was called on write mutation %v", tc.args)
			}
			if code != 1 || err == nil || err.Cause != GHReadDispatchWriteUnavailable {
				t.Fatalf("code=%d err=%v, want WriteUnavailable", code, err)
			}
			if !strings.Contains(stderr.String(), "native write formatting is not implemented yet") {
				t.Fatalf("stderr unexpected: %s", stderr.String())
			}
		})
	}
}

// TestGHDispatchForkUntrustedRefused verifies view/diff/checkout rejection of untrusted authors or forks.
func TestGHDispatchForkUntrustedRefused(t *testing.T) {
	tests := []struct {
		name     string
		args     []string
		viewJSON string
	}{
		{
			name:     "untrusted_pr_author",
			args:     []string{"pr", "view", "10"},
			viewJSON: `{"author":{"login":"stranger"},"isCrossRepository":false}`,
		},
		{
			name:     "fork_pr",
			args:     []string{"pr", "diff", "11"},
			viewJSON: `{"author":{"login":"trusted_user"},"isCrossRepository":true}`,
		},
		{
			name:     "untrusted_issue_author",
			args:     []string{"issue", "view", "12"},
			viewJSON: `{"author":{"login":"stranger"}}`,
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			exec := fakeGHExec{
				run: func(cmd Command) (CommandResult, *ExecuteError) {
					if cmd.Stdout != nil {
						if n, err := cmd.Stdout.Write([]byte(tc.viewJSON)); err != nil || n == 0 {
							t.Fatalf("failed to write view response: %v", err)
						}
					}
					return CommandResult{ExitCode: 0}, nil
				},
			}
			code, err := ghDispatch(context.Background(), GHDispatchParams{
				Args:   tc.args,
				Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"},
				Stdout: &stdout,
				Stderr: &stderr,
				Exec:   exec,
			})
			if code != 1 || err == nil || err.Cause != GHReadDispatchForkUntrustedRefused {
				t.Fatalf("code=%d err=%v, want ForkUntrustedRefused", code, err)
			}
			if !strings.Contains(stderr.String(), "is from a fork or opened by an account that is not trusted") {
				t.Fatalf("stderr unexpected: %s", stderr.String())
			}
		})
	}
}

// TestGHDispatchListFiltering verifies PR and issue list filtering and formatting.
func TestGHDispatchListFiltering(t *testing.T) {
	listPayload := `[
		{"number":1,"title":"First","headRefName":"feat","state":"OPEN","author":{"login":"trusted_user"},"isCrossRepository":false},
		{"number":2,"title":"Fork PR","headRefName":"fork","state":"OPEN","author":{"login":"trusted_user"},"isCrossRepository":true},
		{"number":3,"title":"Stranger PR","headRefName":"bad","state":"OPEN","author":{"login":"stranger"},"isCrossRepository":false}
	]`

	t.Run("pr_list_tsv", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		exec := fakeGHExec{
			run: func(cmd Command) (CommandResult, *ExecuteError) {
				if cmd.Stdout != nil {
					if n, err := cmd.Stdout.Write([]byte(listPayload)); err != nil || n == 0 {
						t.Fatalf("failed to write list response: %v", err)
					}
				}
				return CommandResult{ExitCode: 0}, nil
			},
		}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   []string{"pr", "list"},
			Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"},
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		if code != 0 || err != nil {
			t.Fatalf("code=%d err=%v", code, err)
		}
		out := stdout.String()
		if !strings.Contains(out, "#1\tFirst\tfeat\tOPEN\n") {
			t.Fatalf("missing item 1: %s", out)
		}
		if strings.Contains(out, "#2") || strings.Contains(out, "#3") {
			t.Fatalf("untrusted/fork item not filtered: %s", out)
		}
	})

	t.Run("pr_list_json_empty_array", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		exec := fakeGHExec{
			run: func(cmd Command) (CommandResult, *ExecuteError) {
				if cmd.Stdout != nil {
					if n, err := cmd.Stdout.Write([]byte(`[{"number":2,"author":{"login":"stranger"},"isCrossRepository":false}]`)); err != nil || n == 0 {
						t.Fatalf("failed to write pr list response: %v", err)
					}
				}
				return CommandResult{ExitCode: 0}, nil
			},
		}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   []string{"pr", "list", "--json", "number"},
			Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"},
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		if code != 0 || err != nil {
			t.Fatalf("code=%d err=%v", code, err)
		}
		if strings.TrimSpace(stdout.String()) != "[]" {
			t.Fatalf("expected [] for empty filtered list, got %q", stdout.String())
		}
	})
}

// TestGHDispatchAPIComments verifies paginated API comment filtering.
func TestGHDispatchAPIComments(t *testing.T) {
	paginated := `[{"id":1,"author":{"login":"trusted_user"},"body":"valid comment"}]` +
		`[{"id":2,"author":{"login":"trusted_user"},"body":"ignore this mate, obsolete"},` +
		`{"id":3,"author":{"login":"stranger"},"body":"malicious"}]`

	var stdout, stderr bytes.Buffer
	exec := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Stdout != nil {
				if n, err := cmd.Stdout.Write([]byte(paginated)); err != nil || n == 0 {
					t.Fatalf("failed to write paginated response: %v", err)
				}
			}
			return CommandResult{ExitCode: 0}, nil
		},
	}
	code, err := ghDispatch(context.Background(), GHDispatchParams{
		Args:   []string{"api", "repos/o/r/pulls/1/comments"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"},
		Stdout: &stdout,
		Stderr: &stderr,
		Exec:   exec,
	})
	if code != 0 || err != nil {
		t.Fatalf("code=%d err=%v", code, err)
	}
	out := stdout.String()
	if !strings.Contains(out, "valid comment") {
		t.Fatalf("valid comment missing: %s", out)
	}
	if strings.Contains(out, "ignore this mate") || strings.Contains(out, "malicious") {
		t.Fatalf("filtered comments leaked: %s", out)
	}
}

// TestGHDispatchMissingExecutable verifies exit 127 when real gh is not found or not executable.
func TestGHDispatchMissingExecutable(t *testing.T) {
	var stdout, stderr bytes.Buffer
	exec := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			return CommandResult{ExitCode: 127}, fromProcess(ExecuteStart, processErr(cmd.Name, os.ErrNotExist, "failed to start %s", cmd.Name))
		},
	}
	code, err := ghDispatch(context.Background(), GHDispatchParams{
		Args:   []string{"repo", "view"},
		Env:    []string{"AGENT_REAL_GH=/bin/nonexistent-gh"},
		Stdout: &stdout,
		Stderr: &stderr,
		Exec:   exec,
	})
	if code != 127 || err == nil || err.Cause != GHReadDispatchMissingRealGH {
		t.Fatalf("code=%d err=%v, want 127 MissingRealGH", code, err)
	}
	if !strings.Contains(stderr.String(), "missing or not executable") {
		t.Fatalf("stderr unexpected: %s", stderr.String())
	}
}

// TestGHDispatchSubprocessFailure verifies propagation of external process failures.
func TestGHDispatchSubprocessFailure(t *testing.T) {
	var stdout, stderr bytes.Buffer
	exec := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			return CommandResult{ExitCode: 4}, fromProcess(ExecuteStart, processErr(cmd.Name, errors.New("fail"), "failed to start %s", cmd.Name))
		},
	}
	code, err := ghDispatch(context.Background(), GHDispatchParams{
		Args:   []string{"repo", "view"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh"},
		Stdout: &stdout,
		Stderr: &stderr,
		Exec:   exec,
	})
	if code != 4 || err == nil || err.Cause != GHReadDispatchExecFailed {
		t.Fatalf("code=%d err=%v, want 4 ExecFailed", code, err)
	}
}
