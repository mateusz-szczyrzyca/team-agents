package main

import (
	"fmt"
	"reflect"
	"strings"
	"testing"
)

func init() {
	callNodeGit = func(root string, args ...string) ([]byte, *ProcessError) {
		panic(fmt.Sprintf("callNodeGit(%q, %q) without a stub", root, args))
	}
}

// stubNodeGit restores the process boundary after each sequential test.
func stubNodeGit(t *testing.T, fn func(string, ...string) ([]byte, *ProcessError)) {
	t.Helper()
	prev := callNodeGit
	callNodeGit = fn
	t.Cleanup(func() { callNodeGit = prev })
}

// TestNodeCheckoutValidation: all untrusted fields are checked before executing Git.
func TestNodeCheckoutValidation(t *testing.T) {
	cases := []struct {
		name string              // invalid input variation
		req  NodeCheckoutRequest // request rejected without external effects
		want NodeCheckoutCause   // precise validation failure
	}{
		{"missing repository", NodeCheckoutRequest{BaseRef: "main"}, NodeCheckoutMissingArgument},
		{"missing base", NodeCheckoutRequest{Repository: "repo"}, NodeCheckoutMissingArgument},
		{"path repository", NodeCheckoutRequest{Repository: "../repo", BaseRef: "main"}, NodeCheckoutInvalidRepository},
		{"option ref", NodeCheckoutRequest{Repository: "repo", BaseRef: "--upload-pack=bad"}, NodeCheckoutInvalidBaseRef},
		{"ref traversal", NodeCheckoutRequest{Repository: "repo", BaseRef: "../HEAD"}, NodeCheckoutInvalidBaseRef},
		{"bad SHA", NodeCheckoutRequest{Repository: "repo", BaseRef: "main", BaseSHA: "--bad"}, NodeCheckoutInvalidCommit},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := nodeCheckout(map[string]string{"repo": t.TempDir()}, tc.req)
			failsWith(t, tc.want)(tc.name, err)
		})
	}
}

// TestNodeCheckoutFetchAndResolve: Git receives fixed commands and a validated ref or exact SHA.
func TestNodeCheckoutFetchAndResolve(t *testing.T) {
	for _, sha := range []string{"", strings.ToUpper(regSHA)} {
		t.Run("sha="+sha, func(t *testing.T) {
			root := t.TempDir()
			var calls [][]string // captured Git argument arrays
			stubNodeGit(t, func(gotRoot string, args ...string) ([]byte, *ProcessError) {
				if gotRoot != root {
					t.Fatalf("root=%q, want %q", gotRoot, root)
				}
				calls = append(calls, args)
				if len(calls) == 1 {
					return nil, nil
				}
				return []byte(regSHA + "\n"), nil
			})
			got, err := nodeCheckout(map[string]string{"repo": root}, NodeCheckoutRequest{Repository: "repo", BaseRef: "main", BaseSHA: sha})
			if err != nil {
				t.Fatal(err)
			}
			if got.Root != root || got.Commit != regSHA {
				t.Fatalf("result=%+v", got)
			}
			target := "main"
			if sha != "" {
				target = regSHA
			}
			want := [][]string{{"fetch", "--no-tags", "--no-recurse-submodules", "--", "origin", target}, {"rev-parse", "--verify", "FETCH_HEAD^{commit}"}}
			if !reflect.DeepEqual(calls, want) {
				t.Fatalf("calls=%q, want %q", calls, want)
			}
		})
	}
}

// TestNodeCheckoutResolveFailures: missing, malformed or mismatched commits never authorize preparation.
func TestNodeCheckoutResolveFailures(t *testing.T) {
	for _, out := range []string{"", "not-an-object", strings.Repeat("b", 40), regSHA + "\n" + regSHA} {
		t.Run(out, func(t *testing.T) {
			stubNodeGit(t, func(_ string, args ...string) ([]byte, *ProcessError) {
				if args[0] == "fetch" {
					return nil, nil
				}
				return []byte(out), nil
			})
			_, err := nodeCheckout(map[string]string{"repo": t.TempDir()}, NodeCheckoutRequest{Repository: "repo", BaseRef: "main", BaseSHA: regSHA})
			failsWith(t, NodeCheckoutCheckoutFailed)("resolve", err)
		})
	}
	stubNodeGit(t, func(_ string, args ...string) ([]byte, *ProcessError) {
		if args[0] == "fetch" {
			return nil, nil
		}
		return nil, processErr("git", nil, "cannot resolve commit")
	})
	_, err := nodeCheckout(map[string]string{"repo": t.TempDir()}, NodeCheckoutRequest{Repository: "repo", BaseRef: "main"})
	failsWith(t, NodeCheckoutCheckoutFailed)("git resolve failure", err)
}

// TestNodeCheckoutFlags: repository paths are supplied locally and malformed flags fail closed.
func TestNodeCheckoutFlags(t *testing.T) {
	repos, req, err := parseNodeCheckout([]string{"--repo", "repo", "--base", "main", "--sha", regSHA, "--repo-path", "repo=/tmp/clone with spaces=ok"})
	if err != nil {
		t.Fatal(err)
	}
	if repos["repo"] != "/tmp/clone with spaces=ok" || req.Repository != "repo" || req.BaseRef != "main" || req.BaseSHA != regSHA {
		t.Fatalf("repos=%v request=%+v", repos, req)
	}
	for _, args := range [][]string{{"--unknown"}, {"positional"}, {"--repo-path", "broken"}, {"--repo-path", "repo=relative"}, {"--repo-path", "../bad=/tmp/repo"}, {"--repo-path", "repo=/tmp/a", "--repo-path", "repo=/tmp/b"}} {
		_, _, err := parseNodeCheckout(args)
		failsWith(t, NodeCheckoutBadFlags)("bad flags", err)
	}
}
