package main

import (
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// checkoutFixtureGit runs Git only in temporary repositories with isolated user configuration.
func checkoutFixtureGit(t *testing.T, root string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...) // #nosec G204 -- fixed Git executable; arguments are test fixtures in temporary repositories
	cmd.Dir = root
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %q: %v: %s", args, err, out)
	}
	return strings.TrimSpace(string(out))
}

// Rule: unrelated fetches and overlapping preparations cannot change a job's selected commit.
func TestNodeCheckoutInterleavedFetch(t *testing.T) {
	t.Setenv("GIT_CONFIG_GLOBAL", "/dev/null")
	t.Setenv("GIT_CONFIG_NOSYSTEM", "1")
	t.Setenv("GIT_AUTHOR_NAME", "Example")
	t.Setenv("GIT_AUTHOR_EMAIL", "agent@example.com")
	t.Setenv("GIT_COMMITTER_NAME", "Example")
	t.Setenv("GIT_COMMITTER_EMAIL", "agent@example.com")
	for _, nested := range []bool{false, true} {
		t.Run(map[bool]string{false: "human fetch", true: "another preparation"}[nested], func(t *testing.T) {
			origin := t.TempDir() // local remote containing two different commits
			checkoutFixtureGit(t, origin, "init", "-b", "main")
			checkoutFixtureGit(t, origin, "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", "main")
			mainSHA := checkoutFixtureGit(t, origin, "rev-parse", "HEAD")
			checkoutFixtureGit(t, origin, "checkout", "-b", "other")
			checkoutFixtureGit(t, origin, "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", "other")
			otherSHA := checkoutFixtureGit(t, origin, "rev-parse", "HEAD")
			root := filepath.Join(t.TempDir(), "clone") // clone whose FETCH_HEAD is shared by all callers
			checkoutFixtureGit(t, origin, "clone", origin, root)
			repos := map[string]string{"repo": root} // local allow-list used by both preparations
			interleaved := false                     // inject exactly once after the first fetch completes
			stubNodeGit(t, func(path string, args ...string) ([]byte, *ProcessError) {
				out, err := runNodeGit(path, args...)
				if err == nil && args[0] == "fetch" && !interleaved {
					interleaved = true
					if nested {
						got, failure := nodeCheckout(repos, NodeCheckoutRequest{Repository: "repo", BaseRef: "other"})
						if failure != nil || got.Commit != otherSHA {
							t.Fatalf("nested preparation: %+v, %v", got, failure)
						}
					} else {
						checkoutFixtureGit(t, root, "fetch", "origin", "other")
					}
				}
				return out, err
			})
			got, err := nodeCheckout(repos, NodeCheckoutRequest{Repository: "repo", BaseRef: "main"})
			if err != nil {
				t.Fatal(err)
			}
			if !interleaved || got.Commit != mainSHA {
				t.Errorf("requested main %s, got %s (interleaved=%v)", mainSHA, got.Commit, interleaved)
			}
			if refs := checkoutFixtureGit(t, root, "for-each-ref", "--format=%(refname)", "refs/team-agents/node-checkout/"); refs != "" {
				t.Errorf("private fetch refs left behind: %s", refs)
			}
		})
	}
}
