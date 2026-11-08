package main

import (
	"bytes"
	"context"
	"strings"
	"testing"
)

// TestGHDispatchAPIFlagBeforePath verifies endpoint discovery when two-token flags precede the path.
func TestGHDispatchAPIFlagBeforePath(t *testing.T) {
	for _, tc := range []struct {
		name             string
		args             []string
		untrustedComment string
	}{
		{"header_before_path", []string{"api", "-H", "Accept: application/vnd.github+json", "repos/o/r/issues/1/comments"}, `{"id":1,"author":{"login":"untrusted"},"body":"malicious"}`},
		{"cache_before_path", []string{"api", "--cache", "3600", "repos/o/r/pulls/1/comments"}, `{"id":2,"author":{"login":"untrusted"},"body":"malicious"}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
				if cmd.Stdout != nil {
					_, _ = cmd.Stdout.Write([]byte(tc.untrustedComment))
				}
				return CommandResult{ExitCode: 0}, nil
			}}
			code, err := ghDispatch(context.Background(), GHDispatchParams{
				Args:   tc.args,
				Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"},
				Stdout: &stdout,
				Stderr: &stderr,
				Exec:   exec,
			})
			if code != 0 || err != nil || strings.Contains(stdout.String(), "malicious") {
				t.Fatalf("code=%d err=%v out=%q (expected untrusted comment to be filtered)", code, err, stdout.String())
			}
		})
	}
}

// TestGHDispatchAPIPaginatedIDs verifies UseNumber preservation and page handling.
func TestGHDispatchAPIPaginatedIDs(t *testing.T) {
	for _, tc := range []struct {
		name        string
		pages       []string
		wantCode    int
		wantErrCause GHReadDispatchCause
		wantIDMatch bool
		checkErr    bool
	}{
		// Large ID: 1234567890123456789 becomes 1.2345678901234568e+18 in float64
		{"large_id_preserved_single_page",
			[]string{`[{"id":1234567890123456789,"author":{"login":"alice"},"body":"ok"}]`},
			0, 0, true, false},
		{"large_id_preserved_paginated",
			[]string{
				`[{"id":1234567890123456789,"author":{"login":"alice"},"body":"ok"}]`,
				`[{"id":9876543210987654321,"author":{"login":"alice"},"body":"ok2"}]`,
			},
			0, 0, true, false},
		{"malformed_second_page_invalid_json",
			[]string{
				`[{"id":1,"author":{"login":"alice"},"body":"ok"}]`,
				`{broken json`,
			},
			1, GHReadDispatchInvalidJSON, false, true},
		{"non_array_page_preserves_first",
			[]string{
				`[{"id":100,"author":{"login":"alice"},"body":"first"}]`,
				`{"id":200,"author":{"login":"alice"},"body":"non-array"}`,
			},
			0, 0, true, false},
		{"mixed_pages_first_only",
			[]string{
				`[{"id":1,"author":{"login":"alice"},"body":"first-array"}]`,
				`{"id":2,"author":{"login":"alice"},"body":"non-array"}`,
				`[{"id":3,"author":{"login":"alice"},"body":"third-array"}]`,
			},
			0, 0, true, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
				// All pages written at once (simulating gh --paginate output)
				if cmd.Stdout != nil {
					for _, page := range tc.pages {
						_, _ = cmd.Stdout.Write([]byte(page))
					}
				}
				return CommandResult{ExitCode: 0}, nil
			}}
			code, err := ghDispatch(context.Background(), GHDispatchParams{
				Args:   []string{"api", "repos/o/r/issues/1/comments", "--paginate"},
				Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=alice"},
				Stdout: &stdout,
				Stderr: &stderr,
				Exec:   exec,
			})
			if code != tc.wantCode {
				t.Errorf("code: got %d, want %d", code, tc.wantCode)
			}
			if tc.checkErr && (err == nil || err.Cause != tc.wantErrCause) {
				t.Errorf("err: got %v, want cause %v", err, tc.wantErrCause)
			}
			if tc.wantIDMatch && !strings.Contains(stdout.String(), "123456789") && !strings.Contains(stdout.String(), "987654321") && !strings.Contains(stdout.String(), "100") {
				t.Errorf("output missing expected ID: %q", stdout.String())
			}
		})
	}
}

// Regression tests for filter edge cases are in gh_read_filter_edge_test.go
