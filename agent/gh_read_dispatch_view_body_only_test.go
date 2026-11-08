package main

import (
	"bytes"
	"context"
	"strings"
	"testing"
)

// TestGHDispatchViewBodyOnlyPreservation verifies B53 fix: gh pr/issue view --json
// with body but no comment fields preserves the response without filtering.
func TestGHDispatchViewBodyOnlyPreservation(t *testing.T) {
	for _, tc := range []struct {
		name          string
		args          []string // pr/issue view command args
		authorResp    string   // response from first --json author call
		bodyResp      string   // response from second --json call for body
		wantPreserved bool     // should body be in output unchanged
	}{
		{
			"pr_view_json_body_title_state",
			[]string{"pr", "view", "--json", "body,title,state"},
			`{"author":{"login":"alice"},"isCrossRepository":false}`,
			`{"body":"PR description","title":"PR title","state":"open"}`,
			true,
		},
		{
			"issue_view_json_body",
			[]string{"issue", "view", "--json", "body"},
			`{"author":{"login":"alice"}}`,
			`{"body":"Issue description"}`,
			true,
		},
		{
			"pr_view_json_body_with_untrusted_outsider_comments",
			[]string{"pr", "view", "--json", "body,comments"},
			`{"author":{"login":"alice"},"isCrossRepository":false}`,
			`{"body":"PR description","comments":[{"id":"1","author":{"login":"untrusted"},"body":"malicious"}]}`,
			true, // body preserved but comments filtered
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			authorReqCount := 0
			bodyReqCount := 0
			exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
				args := strings.Join(cmd.Args, " ")
				// First request: author check
				if strings.Contains(args, "--json") && (strings.Contains(args, "author") || strings.Contains(args, "isCrossRepository")) {
					if cmd.Stdout != nil {
						if n, err := cmd.Stdout.Write([]byte(tc.authorResp)); err != nil || n < len(tc.authorResp) {
							t.Fatalf("failed to write auth response: %v", err)
						}
					}
					authorReqCount++
					return CommandResult{ExitCode: 0}, nil
				}
				// Second request: actual body/comments request
				if strings.Contains(args, "--json") {
					if cmd.Stdout != nil {
						if n, err := cmd.Stdout.Write([]byte(tc.bodyResp)); err != nil || n < len(tc.bodyResp) {
							t.Fatalf("failed to write body response: %v", err)
						}
					}
					bodyReqCount++
					return CommandResult{ExitCode: 0}, nil
				}
				return CommandResult{ExitCode: 0}, nil
			}}
			code, err := ghDispatch(context.Background(), GHDispatchParams{
				Args:   tc.args,
				Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=alice"},
				Stdout: &stdout,
				Stderr: &stderr,
				Exec:   exec,
			})
			if code != 0 || err != nil {
				t.Fatalf("code=%d err=%v", code, err)
			}
			output := stdout.String()
			if tc.wantPreserved {
				if !strings.Contains(output, "PR description") && !strings.Contains(output, "Issue description") {
					t.Errorf("expected body to be preserved in output: %q", output)
				}
				// If this test has comments with untrusted author, verify body is there but malicious is filtered
				if strings.Contains(tc.bodyResp, "untrusted") && strings.Contains(output, "malicious") {
					t.Errorf("expected untrusted comment to be filtered out: %q", output)
				}
			}
		})
	}
}
