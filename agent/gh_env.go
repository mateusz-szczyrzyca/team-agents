package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	execpkg "os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"
)

// LogAgentsGHMissing is emitted when the host environment has no gh binary available.
const LogAgentsGHMissing LogEvent = "agents_gh_missing"

// githubOwnerPattern extracts the owner from common GitHub remote URL formats.
var githubOwnerPattern = regexp.MustCompile(`^(?:https://|git@|ssh://git@)github\.com[:/]([^/]+)/.*`)

// ghDefaultLoginStore holds the default process-wide GitHub login cache.
var ghDefaultLoginStore ghLoginCacheStore

// ghLoginCacheStore holds an isolated, thread-safe GitHub login cache.
type ghLoginCacheStore struct {
	mu    sync.Mutex // protects concurrent access to login
	login string     // cached authenticated GitHub user login
}

func (s *ghLoginCacheStore) fetch(load func() string) string {
	s.mu.Lock()
	defer s.mu.Unlock()
	if load == nil {
		s.login = ""
	} else if s.login == "" {
		s.login = load()
	}
	return s.login
}

// GitHubSetupParams carries the per-run inputs needed to configure the trusted gh wrapper environment.
type GitHubSetupParams struct {
	AgentsHome, SelfExecutable, RepoRoot, RunnerID string                       // runner directories and task identifier
	Team, Agent, Coordinator, OwnPR                string                       // team membership and task PR number
	BusDir, AgentbusBin, ReviewByline              string                       // coordination paths and review byline
	GitHubTrusted, GitHubColors, GitHubNameColors  string                       // trusted accounts and color preferences
	CurrentPath                                    string                       // existing PATH to prepend wrapper directory to
	LookPath                                       func(string) (string, error) // lookup for gh binary
	Logger                                         *Logger                      // logger for warnings
}

// GitHubRunEnv carries the environment variables and path settings for a run.
type GitHubRunEnv struct {
	RealGH, WrapperPath         string            // real executable and wrapper symlink paths
	TrustedAccounts, RawTrusted string            // normalized and raw trusted account logins
	PrependPath, EffectivePath  string            // wrapper directory to prepend and resulting PATH
	Env                         map[string]string // exported environment variables
}

// runCmd executes an external helper command for github setup. Command failures or non-zero exits
// return an empty string per the Bash oracle (bin/agent github_owner and github_trusted), where
// missing origin URL yields no owner and unauthenticated gh API yields no trusted user login.
func runCmd(ctx context.Context, exec Executor, dir, name string, args ...string) string {
	if exec == nil {
		exec = OSExecutor{}
	}
	var out strings.Builder
	if r, err := exec.Execute(ctx, Command{Name: name, Args: args, Dir: dir, Stdout: &out}); err == nil && r.ExitCode == 0 {
		return strings.TrimSpace(out.String())
	}
	return ""
}

// ExtractGitHubOwner parses a git remote origin URL and extracts the repository owner.
func ExtractGitHubOwner(remoteURL string) string {
	if m := githubOwnerPattern.FindStringSubmatch(strings.TrimSpace(remoteURL)); len(m) > 1 {
		return strings.TrimSuffix(m[1], ".git")
	}
	return ""
}

// ResolveRealGH locates the system gh executable, avoiding wrapper recursion.
func ResolveRealGH(lookPath func(string) (string, error), home, cur string) (string, bool, *GitHubSetupError) {
	if lookPath == nil {
		lookPath = execpkg.LookPath
	}
	wrap := filepath.Join(home, "run", "gh-trusted", "gh")
	p, err := lookPath("gh")
	if p == wrap {
		p = cur
	}
	if err == nil && p == "" {
		return "", false, opErr(GitHubSetupMissingRealGH, "gh points to wrapper %s but AGENT_REAL_GH is not set", wrap)
	}
	return p, err == nil, nil
}

// EnsureGHWrapper atomically creates or updates the wrapper symlink pointing to selfExecutable.
func EnsureGHWrapper(home, self string) (string, *GitHubSetupError) {
	wrap := filepath.Join(home, "run", "gh-trusted", "gh")
	if err := os.MkdirAll(filepath.Dir(wrap), 0o750); err != nil {
		return "", fromStorage(GitHubSetupStorage, storageErr(err, "create gh wrapper directory %s", filepath.Dir(wrap)))
	}
	if t, err := os.Readlink(wrap); err == nil && t == self {
		return wrap, nil
	}
	tmp := fmt.Sprintf("%s.%d.%d", wrap, os.Getpid(), time.Now().UnixNano())
	if err := os.Symlink(self, tmp); err != nil {
		return "", fromStorage(GitHubSetupStorage, storageErr(err, "create temporary gh wrapper symlink %s", tmp))
	}
	if err := os.Rename(tmp, wrap); err != nil {
		return "", fromStorage(GitHubSetupStorage, storageErr(errors.Join(err, os.Remove(tmp)), "rename gh wrapper symlink %s", wrap))
	}
	return wrap, nil
}

// ResolveGitHubTrustedRaw resolves trusted accounts returning the exact oracle format with separator.
func ResolveGitHubTrustedRaw(ctx context.Context, exec Executor, repoRoot, trustedOverride string) string {
	if trustedOverride != "" {
		return trustedOverride
	}
	owner := ExtractGitHubOwner(runCmd(ctx, exec, "", "git", "-C", repoRoot, "remote", "get-url", "origin"))
	s, _ := ctx.Value((*ghLoginCacheStore)(nil)).(*ghLoginCacheStore)
	if s == nil {
		s = &ghDefaultLoginStore
	}
	login := s.fetch(func() string { return runCmd(ctx, exec, repoRoot, "gh", "api", "user", "--jq", ".login") })
	return owner + " " + login
}

// ResetGHLoginCacheForTest resets the internal cached GitHub login.
func ResetGHLoginCacheForTest() { ghDefaultLoginStore.fetch(nil) }

// WithGHLoginCache returns a context that carries an isolated GitHub login cache.
func WithGHLoginCache(ctx context.Context) context.Context {
	if ctx.Value((*ghLoginCacheStore)(nil)) == nil {
		return context.WithValue(ctx, (*ghLoginCacheStore)(nil), &ghLoginCacheStore{})
	}
	return ctx
}

// ResolveGitHubTrusted resolves the trusted accounts (override, or repository owner + authenticated user).
func ResolveGitHubTrusted(ctx context.Context, exec Executor, repoRoot, trustedOverride string) (string, *GitHubSetupError) {
	if err := ctx.Err(); err != nil {
		return "", fromStorage(GitHubSetupStorage, storageErr(err, "resolve trusted github accounts in %s", repoRoot))
	}
	return strings.TrimSpace(ResolveGitHubTrustedRaw(WithGHLoginCache(ctx), exec, repoRoot, trustedOverride)), nil
}

// SetupGitHubRun prepares the per-run gh wrapper and environment variables without mutating global state.
func SetupGitHubRun(ctx context.Context, exec Executor, params GitHubSetupParams) (*GitHubRunEnv, *GitHubSetupError) {
	realPath, found, err := ResolveRealGH(params.LookPath, params.AgentsHome, os.Getenv("AGENT_REAL_GH"))
	if err != nil {
		return nil, err
	}
	if !found {
		if params.Logger != nil {
			// Side effect: log write failure is nonfatal per Bash oracle log_event (which warns to stderr); do not abort run.
			_ = params.Logger.Log(ctx, LevelWarn, LogAgentsGHMissing, "task="+params.RunnerID)
		}
		return &GitHubRunEnv{Env: map[string]string{}, EffectivePath: params.CurrentPath}, nil
	}
	wrap, err := EnsureGHWrapper(params.AgentsHome, params.SelfExecutable)
	if err != nil {
		return nil, err
	}
	raw := ResolveGitHubTrustedRaw(WithGHLoginCache(ctx), exec, params.RepoRoot, params.GitHubTrusted)
	dir := filepath.Dir(wrap)
	prep, eff := dir, dir+":"+params.CurrentPath
	if strings.Contains(":"+params.CurrentPath+":", ":"+dir+":") {
		prep, eff = "", params.CurrentPath
	} else if params.CurrentPath == "" {
		eff = dir
	}
	return &GitHubRunEnv{
		RealGH: realPath, WrapperPath: wrap, TrustedAccounts: strings.TrimSpace(raw), RawTrusted: raw, PrependPath: prep, EffectivePath: eff,
		Env: map[string]string{"AGENT_REAL_GH": realPath, "AGENT_GITHUB_TRUSTED_RESOLVED": raw, "AGENT_REVIEW_BYLINE": params.ReviewByline, "AGENT_GITHUB_COLORS": params.GitHubColors, "AGENT_GH_COORDINATOR": params.Coordinator, "AGENT_GH_TEAM": params.Team, "AGENT_GH_SELF": params.Agent, "AGENT_GH_BUS_DIR": params.BusDir, "AGENTBUS_BIN": params.AgentbusBin, "AGENT_GH_OWN_PR": params.OwnPR, "AGENT_GITHUB_NAME_COLORS": params.GitHubNameColors},
	}, nil
}
