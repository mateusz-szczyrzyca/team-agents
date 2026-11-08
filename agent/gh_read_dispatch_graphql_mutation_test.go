package main

import (
	"bytes"
	"context"
	"testing"
)

// TestGHDispatchGraphQLMutationsAndInput verifies GraphQL mutations and --input are refused without invoking realGH.
func TestGHDispatchGraphQLMutationsAndInput(t *testing.T) {
	cases := []struct {
		name         string
		args         []string
		env          []string
		wantCode     int
		wantErrCause GHReadDispatchCause
		wantCalled   bool
	}{
		{
			name:         "member_graphql_pr_mutation_refused",
			args:         []string{"api", "graphql", "-f", "query=mutation { updatePullRequest(input:{id:\"1\"}) { clientMutationId } }"},
			env:          []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead"},
			wantCode:     1,
			wantErrCause: GHReadDispatchMemberRefused,
			wantCalled:   false,
		},
		{
			name:         "general_graphql_mutation_refused",
			args:         []string{"api", "graphql", "-f", "query=mutation { addStar(input:{starrableId:\"1\"}) { clientMutationId } }"},
			env:          []string{"AGENT_REAL_GH=/bin/gh"},
			wantCode:     1,
			wantErrCause: GHReadDispatchWriteUnavailable,
			wantCalled:   false,
		},
		{
			name:         "graphql_input_flag_refused",
			args:         []string{"api", "graphql", "--input", "query.json"},
			env:          []string{"AGENT_REAL_GH=/bin/gh"},
			wantCode:     1,
			wantErrCause: GHReadDispatchWriteUnavailable,
			wantCalled:   false,
		},
		{
			name:       "harmless_graphql_query_allowed",
			args:       []string{"api", "graphql", "-f", "query={ viewer { login } }"},
			env:        []string{"AGENT_REAL_GH=/bin/gh"},
			wantCode:   0,
			wantCalled: true,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			calledRealGH := false
			exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
				if cmd.Name == "/bin/gh" {
					calledRealGH = true
				}
				return CommandResult{ExitCode: 0}, nil
			}}
			var stdout, stderr bytes.Buffer
			code, err := ghDispatch(context.Background(), GHDispatchParams{
				Args:   tc.args,
				Env:    tc.env,
				Stdout: &stdout,
				Stderr: &stderr,
				Exec:   exec,
			})
			if calledRealGH != tc.wantCalled {
				t.Errorf("calledRealGH: got %v, want %v", calledRealGH, tc.wantCalled)
			}
			if code != tc.wantCode {
				t.Errorf("code: got %d, want %d", code, tc.wantCode)
			}
			if tc.wantErrCause != 0 && (err == nil || err.Cause != tc.wantErrCause) {
				t.Errorf("err cause: got %v, want %v", err, tc.wantErrCause)
			}
		})
	}
}
