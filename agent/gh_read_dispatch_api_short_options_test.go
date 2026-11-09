package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestGHAPIShortOptionsExecutable keeps shorthand policy checks ahead of real processes.
func TestGHAPIShortOptionsExecutable(t *testing.T) {
	// endpoint identifies protected comments in the synthetic repository.
	const endpoint = "repos/o/r/issues/1/comments"
	// fixture honors compact queries so forwarding one really removes author metadata.
	const fixture = `#!/bin/sh
set -eu
umask 077
printf '%s\n' "$@" > "$CALLS"
for arg do
  case "$arg" in
    -q?*) query=${arg#-q}; query=${query#=}; printf '%s\n' "$RESPONSE" | jq -c "$query"; exit ;;
  esac
done
printf '%s\n' "$RESPONSE"
`
	// cases cover reads, local query semantics and writes hidden inside include clusters.
	cases := []struct {
		name  string              // name describes the spelling under test.
		args  []string            // args follow gh api.
		want  string              // want is exact stdout after native filtering and real jq.
		argv  string              // argv is the exact forwarded argument list, one per line.
		code  int                 // code is the required subprocess or policy exit status.
		cause GHReadDispatchCause // cause identifies the exact refusal when code is nonzero.
	}{
		{"preview", []string{"-p", "nebula", endpoint}, "[]\n", "api\n-p\nnebula\n" + endpoint + "\n", 0, 0},
		{"preview_equals", []string{"-p=nebula", endpoint}, "[]\n", "api\n-p\nnebula\n" + endpoint + "\n", 0, 0},
		{"preview_compact", []string{"-pnebula", endpoint}, "[]\n", "api\n-p\nnebula\n" + endpoint + "\n", 0, 0},
		{"query", []string{endpoint, "-qmap(.body)"}, "[]\n", "api\n" + endpoint + "\n", 0, 0},
		{"query_length", []string{endpoint, "-q=map(.body)|length"}, "0\n", "api\n" + endpoint + "\n", 0, 0},
		{"query_trusted", []string{endpoint, `-qmap(.body)|join(",")`}, "TRUSTED_BODY\n", "api\n" + endpoint + "\n", 0, 0},
		{"template_compact", []string{endpoint, "-t{{.}}"}, "", "", 2, GHReadDispatchTemplateRefused},
		{"include_get", []string{"repos/o/r/pulls/1", "-iXGET"}, "[]\n", "api\nrepos/o/r/pulls/1\n-i\n-X\nGET\n", 0, 0},
		{"include_patch", []string{"repos/o/r/pulls/1", "-iXPATCH"}, "", "", 1, GHReadDispatchWriteUnavailable},
		{"include_patch_equals", []string{"repos/o/r/pulls/1", "-iX=PATCH"}, "", "", 1, GHReadDispatchWriteUnavailable},
		{"include_patch_separate", []string{"repos/o/r/pulls/1", "-iX", "PATCH"}, "", "", 1, GHReadDispatchWriteUnavailable},
		{"include_raw_field", []string{"repos/o/r/pulls/1", "-ifbody=change"}, "", "", 1, GHReadDispatchWriteUnavailable},
		{"include_query_file", []string{"graphql", "-iFquery=@payload.graphql"}, "", "", 1, GHReadDispatchWriteUnavailable},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			// dir isolates every subprocess artifact and avoids the installed agent state.
			dir := t.TempDir()
			if err := os.MkdirAll(filepath.Join(dir, "agents", "logs"), 0o700); err != nil {
				t.Fatal(err)
			}
			// gh is an executable fixture, never the network-enabled GitHub CLI.
			gh := filepath.Join(dir, "gh")
			if err := os.WriteFile(gh, []byte(fixture), 0o700); err != nil { // #nosec G306 -- executable test fixture in a private temporary directory.
				t.Fatal(err)
			}
			// calls records whether gh was launched and which arguments crossed the boundary.
			calls := filepath.Join(dir, "calls")
			// response has only an untrusted author; include_get uses an ordinary unfiltered response.
			response := `[{"id":1,"user":{"login":"outsider"},"body":"UNTRUSTED_BODY"}]`
			if tc.name == "query_trusted" {
				response = `[{"user":{"login":"alice"},"body":"TRUSTED_BODY"},{"user":{"login":"outsider"},"body":"UNTRUSTED_BODY"}]`
			}
			if tc.name == "include_get" {
				response = "[]"
			}
			// env supplies only fixture state, real jq lookup and the native trust configuration.
			env := []string{"HOME=" + dir, "PATH=" + os.Getenv("PATH"), "AGENT_REAL_GH=" + gh,
				"AGENT_GITHUB_TRUSTED_RESOLVED=alice", "CALLS=" + calls, "RESPONSE=" + response}
			// out and diagnostics retain the real subprocess streams for assertions.
			var out, diagnostics bytes.Buffer
			// code and failure expose subprocess status and the exact native refusal cause.
			code, failure := ghDispatch(context.Background(), GHDispatchParams{
				Args: append([]string{"api"}, tc.args...), Env: env, Stdout: &out, Stderr: &diagnostics, Exec: OSExecutor{},
			})
			if tc.code != 0 {
				if code != tc.code || failure == nil || failure.Cause != tc.cause {
					t.Errorf("write guard: code=%d failure=%v stderr=%s", code, failure, diagnostics.String())
				}
			} else if code != 0 || failure != nil {
				t.Errorf("read: code=%d failure=%v stderr=%s", code, failure, diagnostics.String())
			}
			if out.String() != tc.want {
				t.Errorf("stdout=%q, want %q", out.String(), tc.want)
			}
			// recorded and readErr distinguish a refused launch from an unexpected filesystem failure.
			recorded, readErr := os.ReadFile(filepath.Clean(calls))
			if tc.code != 0 {
				if !os.IsNotExist(readErr) {
					t.Errorf("refused command reached gh: argv=%q error=%v", recorded, readErr)
				}
			} else if readErr != nil {
				t.Fatal(readErr)
			} else if string(recorded) != tc.argv || strings.Contains(string(recorded), "-q") {
				t.Errorf("forwarded argv=%q, want %q without a caller query", recorded, tc.argv)
			}
		})
	}
}
