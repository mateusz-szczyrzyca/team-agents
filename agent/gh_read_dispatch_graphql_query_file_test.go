package main

import (
	"bytes"
	"context"
	"testing"
)

// TestGHDispatchGraphQLQueryFile asserts typed GraphQL query-file and stdin payloads are refused without invoking realGH.
func TestGHDispatchGraphQLQueryFile(t *testing.T) {
	cases := []struct {
		name         string
		args         []string
		env          []string
		wantCode     int
		wantErrCause GHReadDispatchCause
		wantCalled   bool
	}{
		{
			name:         "typed_field_query_file_refused",
			args:         []string{"api", "graphql", "-F", "query=@query.graphql"},
			env:          []string{"AGENT_REAL_GH=/bin/gh"},
			wantCode:     1,
			wantErrCause: GHReadDispatchWriteUnavailable,
			wantCalled:   false,
		},
		{
			name:         "typed_field_query_stdin_refused",
			args:         []string{"api", "graphql", "--field", "query=@-"},
			env:          []string{"AGENT_REAL_GH=/bin/gh"},
			wantCode:     1,
			wantErrCause: GHReadDispatchWriteUnavailable,
			wantCalled:   false,
		},
		{
			name:         "typed_field_long_query_file_refused",
			args:         []string{"api", "graphql", "--field=query=@payload.graphql"},
			env:          []string{"AGENT_REAL_GH=/bin/gh"},
			wantCode:     1,
			wantErrCause: GHReadDispatchWriteUnavailable,
			wantCalled:   false,
		},
		{
			name:         "typed_field_compact_query_file_refused",
			args:         []string{"api", "graphql", "-Fquery=@req.graphql"},
			env:          []string{"AGENT_REAL_GH=/bin/gh"},
			wantCode:     1,
			wantErrCause: GHReadDispatchWriteUnavailable,
			wantCalled:   false,
		},
		{
			name:         "typed_field_compact_equal_query_file_refused",
			args:         []string{"api", "graphql", "-F=query=@data.graphql"},
			env:          []string{"AGENT_REAL_GH=/bin/gh"},
			wantCode:     1,
			wantErrCause: GHReadDispatchWriteUnavailable,
			wantCalled:   false,
		},
		{
			name:       "raw_field_literal_at_allowed",
			args:       []string{"api", "graphql", "-f", "query={ viewer { login } }", "-f", "note=@literal"},
			env:        []string{"AGENT_REAL_GH=/bin/gh"},
			wantCode:   0,
			wantCalled: true,
		},
		{
			name:       "typed_field_inline_query_allowed",
			args:       []string{"api", "graphql", "-F", "query={ viewer { login } }"},
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
