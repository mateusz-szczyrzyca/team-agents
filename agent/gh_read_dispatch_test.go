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
	for _, tc := range []struct {
		name, wantText string
		env, args      []string
		wantCode       int
		wantCause      GHReadDispatchCause
	}{
		{"missing_real_gh", "AGENT_REAL_GH", nil, []string{"status"}, 127, GHReadDispatchMissingRealGH},
		{"template_flag", "--template is not available", []string{"AGENT_REAL_GH=/bin/gh"}, []string{"pr", "view", "-t", "{{.title}}"}, 2, GHReadDispatchTemplateRefused},
		{"web_flag", "--web is not available", []string{"AGENT_REAL_GH=/bin/gh"}, []string{"pr", "view", "--web"}, 2, GHReadDispatchWebRefused},
		{"graphql_protected", "GraphQL query for comments, reviews or bodies is refused", []string{"AGENT_REAL_GH=/bin/gh"}, []string{"api", "graphql", "-f", "query={ repository { pullRequest { reviewThreads } } }"}, 1, GHReadDispatchGraphQLRefused},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			code, err := ghDispatch(context.Background(), GHDispatchParams{Args: tc.args, Env: tc.env, Stdout: &stdout, Stderr: &stderr, Exec: fakeGHExec{}})
			if code != tc.wantCode || err == nil || err.Cause != tc.wantCause || !strings.Contains(stderr.String(), tc.wantText) {
				t.Fatalf("code=%d err=%v stderr=%q", code, err, stderr.String())
			}
		})
	}
}

// TestGHDispatchMemberRefused verifies team member PR mutation refusals.
func TestGHDispatchMemberRefused(t *testing.T) {
	for _, tc := range []struct {
		name, wantText string
		env, args      []string
	}{
		{"pr_create_lead", "opened by its coordinator (lead-agent)", []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead-agent"}, []string{"pr", "create"}},
		{"pr_merge_lead", "its coordinator's (lead-agent)", []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead-agent"}, []string{"pr", "merge", "1"}},
		{"pr_ready_lead", "mark ready", []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead-agent"}, []string{"pr", "ready", "1"}},
		{"api_pr_merge", "through gh api", []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead-agent"}, []string{"api", "-X", "PUT", "repos/o/r/pulls/1/merge"}},
		{"team_no_lead", "your team has no coordinator yet", []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_TEAM=myteam", "AGENT_GH_SELF=gemini-1"}, []string{"pr", "create"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
				if strings.Contains(cmd.Name, "agentbus") && cmd.Stdout != nil {
					_, _ = cmd.Stdout.Write([]byte(`{"lead":""}`))
				}
				return CommandResult{ExitCode: 0}, nil
			}}
			code, err := ghDispatch(context.Background(), GHDispatchParams{Args: tc.args, Env: tc.env, Stdout: &stdout, Stderr: &stderr, Exec: exec})
			if code != 1 || err == nil || err.Cause != GHReadDispatchMemberRefused || !strings.Contains(stderr.String(), tc.wantText) {
				t.Fatalf("code=%d err=%v stderr=%q", code, err, stderr.String())
			}
		})
	}
}

// TestGHDispatchSecondPRRefused verifies refusal when the task already has an open PR.
func TestGHDispatchSecondPRRefused(t *testing.T) {
	var stdout, stderr bytes.Buffer
	exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
		if len(cmd.Args) >= 4 && cmd.Args[0] == "pr" && cmd.Args[1] == "view" && cmd.Args[2] == "42" && cmd.Stdout != nil {
			_, _ = cmd.Stdout.Write([]byte(`{"state":"OPEN"}`))
		}
		return CommandResult{ExitCode: 0}, nil
	}}
	code, err := ghDispatch(context.Background(), GHDispatchParams{Args: []string{"pr", "create"}, Env: []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_OWN_PR=42"}, Stdout: &stdout, Stderr: &stderr, Exec: exec})
	if code != 1 || err == nil || err.Cause != GHReadDispatchSecondPRRefused || !strings.Contains(stderr.String(), "PR is #42 and still open") {
		t.Fatalf("code=%d err=%v stderr=%q", code, err, stderr.String())
	}
}

// TestGHDispatchWriteUnavailable verifies fail-closed refusal of write mutations until R12.
func TestGHDispatchWriteUnavailable(t *testing.T) {
	for _, tc := range [][]string{
		{"pr", "create", "--title", "New PR"},
		{"pr", "edit", "1", "--body", "Updated"},
		{"pr", "comment", "1", "--body", "LGTM"},
		{"issue", "create", "--title", "Bug"},
		{"api", "-X", "POST", "repos/o/r/issues", "-f", "title=Bug"},
	} {
		t.Run(strings.Join(tc, "_"), func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			calledRealGH := false
			exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
				if cmd.Name == "/bin/gh" {
					calledRealGH = true
				}
				return CommandResult{ExitCode: 0}, nil
			}}
			code, err := ghDispatch(context.Background(), GHDispatchParams{Args: tc, Env: []string{"AGENT_REAL_GH=/bin/gh"}, Stdout: &stdout, Stderr: &stderr, Exec: exec})
			if calledRealGH || code != 1 || err == nil || err.Cause != GHReadDispatchWriteUnavailable || !strings.Contains(stderr.String(), "native write formatting is not implemented yet") {
				t.Fatalf("called=%v code=%d err=%v stderr=%q", calledRealGH, code, err, stderr.String())
			}
		})
	}
}

// TestGHDispatchForkUntrustedRefused verifies view/diff/checkout rejection of untrusted authors or forks.
func TestGHDispatchForkUntrustedRefused(t *testing.T) {
	for _, tc := range []struct {
		name, viewJSON string
		args           []string
	}{
		{"untrusted_pr", `{"author":{"login":"stranger"},"isCrossRepository":false}`, []string{"pr", "view", "10"}},
		{"fork_pr", `{"author":{"login":"trusted_user"},"isCrossRepository":true}`, []string{"pr", "diff", "11"}},
		{"untrusted_issue", `{"author":{"login":"stranger"}}`, []string{"issue", "view", "12"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
				if cmd.Stdout != nil {
					_, _ = cmd.Stdout.Write([]byte(tc.viewJSON))
				}
				return CommandResult{ExitCode: 0}, nil
			}}
			code, err := ghDispatch(context.Background(), GHDispatchParams{Args: tc.args, Env: []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"}, Stdout: &stdout, Stderr: &stderr, Exec: exec})
			if code != 1 || err == nil || err.Cause != GHReadDispatchForkUntrustedRefused || !strings.Contains(stderr.String(), "is from a fork or opened by an account that is not trusted") {
				t.Fatalf("code=%d err=%v stderr=%q", code, err, stderr.String())
			}
		})
	}
}

// TestGHDispatchListFiltering verifies PR and issue list filtering and formatting.
func TestGHDispatchListFiltering(t *testing.T) {
	listPayload := `[{"number":1,"title":"First","headRefName":"feat","state":"OPEN","author":{"login":"trusted_user"},"isCrossRepository":false},{"number":2,"title":"Fork PR","headRefName":"fork","state":"OPEN","author":{"login":"trusted_user"},"isCrossRepository":true},{"number":3,"title":"Stranger PR","headRefName":"bad","state":"OPEN","author":{"login":"stranger"},"isCrossRepository":false}]`
	t.Run("pr_list_tsv", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Stdout != nil {
				_, _ = cmd.Stdout.Write([]byte(listPayload))
			}
			return CommandResult{ExitCode: 0}, nil
		}}
		code, err := ghDispatch(context.Background(), GHDispatchParams{Args: []string{"pr", "list"}, Env: []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"}, Stdout: &stdout, Stderr: &stderr, Exec: exec})
		if code != 0 || err != nil || !strings.Contains(stdout.String(), "#1\tFirst\tfeat\tOPEN\n") || strings.Contains(stdout.String(), "#2") || strings.Contains(stdout.String(), "#3") {
			t.Fatalf("code=%d err=%v out=%q", code, err, stdout.String())
		}
	})
	t.Run("pr_list_json_empty_array", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Stdout != nil {
				_, _ = cmd.Stdout.Write([]byte(`[{"number":2,"author":{"login":"stranger"},"isCrossRepository":false}]`))
			}
			return CommandResult{ExitCode: 0}, nil
		}}
		code, err := ghDispatch(context.Background(), GHDispatchParams{Args: []string{"pr", "list", "--json", "number"}, Env: []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"}, Stdout: &stdout, Stderr: &stderr, Exec: exec})
		if code != 0 || err != nil || strings.TrimSpace(stdout.String()) != "[]" {
			t.Fatalf("code=%d err=%v out=%q", code, err, stdout.String())
		}
	})
}

// TestGHDispatchAPIComments verifies paginated API comment filtering.
func TestGHDispatchAPIComments(t *testing.T) {
	paginated := `[{"id":1,"author":{"login":"trusted_user"},"body":"valid comment"}][{"id":2,"author":{"login":"trusted_user"},"body":"ignore this mate, obsolete"},{"id":3,"author":{"login":"stranger"},"body":"malicious"}]`
	var stdout, stderr bytes.Buffer
	exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
		if cmd.Stdout != nil {
			_, _ = cmd.Stdout.Write([]byte(paginated))
		}
		return CommandResult{ExitCode: 0}, nil
	}}
	code, err := ghDispatch(context.Background(), GHDispatchParams{Args: []string{"api", "repos/o/r/pulls/1/comments"}, Env: []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"}, Stdout: &stdout, Stderr: &stderr, Exec: exec})
	if code != 0 || err != nil || !strings.Contains(stdout.String(), "valid comment") || strings.Contains(stdout.String(), "ignore this mate") || strings.Contains(stdout.String(), "malicious") {
		t.Fatalf("code=%d err=%v out=%q", code, err, stdout.String())
	}
}

// TestGHDispatchSubprocessFailure verifies propagation of external process failures and exit 127.
func TestGHDispatchSubprocessFailure(t *testing.T) {
	t.Run("missing_executable_127", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			return CommandResult{ExitCode: 127}, fromProcess(ExecuteStart, processErr(cmd.Name, os.ErrNotExist, "failed to start %s", cmd.Name))
		}}
		code, err := ghDispatch(context.Background(), GHDispatchParams{Args: []string{"repo", "view"}, Env: []string{"AGENT_REAL_GH=/bin/nonexistent-gh"}, Stdout: &stdout, Stderr: &stderr, Exec: exec})
		if code != 127 || err == nil || err.Cause != GHReadDispatchMissingRealGH || !strings.Contains(stderr.String(), "missing or not executable") {
			t.Fatalf("code=%d err=%v stderr=%q", code, err, stderr.String())
		}
	})
	t.Run("process_fail_code", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			return CommandResult{ExitCode: 4}, fromProcess(ExecuteStart, processErr(cmd.Name, errors.New("fail"), "failed to start %s", cmd.Name))
		}}
		code, err := ghDispatch(context.Background(), GHDispatchParams{Args: []string{"repo", "view"}, Env: []string{"AGENT_REAL_GH=/bin/gh"}, Stdout: &stdout, Stderr: &stderr, Exec: exec})
		if code != 4 || err == nil || err.Cause != GHReadDispatchExecFailed {
			t.Fatalf("code=%d err=%v", code, err)
		}
	})
}
