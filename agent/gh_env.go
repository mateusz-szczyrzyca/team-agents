package main

import (
	"bytes"
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

var (
	// githubOwnerPattern extracts the owner from common GitHub remote URL formats.
	githubOwnerPattern = regexp.MustCompile(`^(?:https://|git@|ssh://git@)github\.com[:/]([^/]+)/.*`)
	// ghLoginMu synchronizes access to the default in-process GitHub login cache.
	ghLoginMu sync.Mutex
	// ghLoginCache holds the default cached authenticated GitHub user login.
	ghLoginCache string
)

// ghLoginCacheKey is the context key for an isolated GitHub login cache.
type ghLoginCacheKey struct{}

// ghLoginCacheStore holds an isolated, thread-safe GitHub login cache.
type ghLoginCacheStore struct {
	mu    sync.Mutex // protects concurrent access to login
	login string     // cached authenticated GitHub user login
}

// GitHubSetupParams carries the per-run inputs needed to configure the trusted gh wrapper environment.
type GitHubSetupParams struct {
	AgentsHome       string                       // base agents home directory ($AGENTS_HOME)
	SelfExecutable   string                       // absolute path to self executable ($SELF)
	RepoRoot         string                       // working repository root directory ($REPO_ROOT)
	RunnerID         string                       // task identifier of the current runner ($RUNNER_ID)
	Team             string                       // team name ($AGENT_GH_TEAM)
	Agent            string                       // agent name ($AGENT_GH_SELF)
	Coordinator      string                       // coordinator name ($AGENT_GH_COORDINATOR)
	OwnPR            string                       // task's own PR number ($AGENT_GH_OWN_PR)
	BusDir           string                       // shared bus directory ($AGENT_GH_BUS_DIR)
	AgentbusBin      string                       // agentbus binary path ($AGENTBUS_BIN)
	ReviewByline     string                       // review byline for investigator runs ($AGENT_REVIEW_BYLINE)
	GitHubTrusted    string                       // explicit override for trusted GitHub accounts ($AGENT_GITHUB_TRUSTED)
	GitHubColors     string                       // GitHub colors setting ($AGENT_GITHUB_COLORS)
	GitHubNameColors string                       // registered name colors ($AGENT_GITHUB_NAME_COLORS)
	CurrentPath      string                       // existing PATH to prepend wrapper directory to
	LookPath         func(string) (string, error) // lookup function for gh binary, exec.LookPath if nil
	Logger           *Logger                      // logger for warnings like agents_gh_missing
}

// GitHubRunEnv carries the environment variables and path settings for a run.
type GitHubRunEnv struct {
	RealGH          string            // resolved path to the underlying real gh binary
	WrapperPath     string            // path to the created symlink ($AGENTS_HOME/run/gh-trusted/gh)
	TrustedAccounts string            // space-separated resolved trusted GitHub logins
	RawTrusted      string            // raw unnormalized trusted accounts string as exported to environment
	Env             map[string]string // map of exported environment variables
	PrependPath     string            // wrapper directory to prepend to PATH
	EffectivePath   string            // new PATH with wrapper directory prepended
}

// ExtractGitHubOwner parses a git remote origin URL and extracts the repository owner.
func ExtractGitHubOwner(remoteURL string) string {
	matches := githubOwnerPattern.FindStringSubmatch(strings.TrimSpace(remoteURL))
	if len(matches) < 2 {
		return ""
	}
	return strings.TrimSuffix(matches[1], ".git")
}

// ResolveRealGH locates the system gh executable, avoiding wrapper recursion.
func ResolveRealGH(lookPath func(string) (string, error), agentsHome string, currentRealGH string) (string, bool, *GitHubSetupError) {
	if lookPath == nil {
		lookPath = execpkg.LookPath
	}
	wrapperPath := filepath.Join(agentsHome, "run", "gh-trusted", "gh")
	realPath, err := lookPath("gh")
	if err != nil {
		return "", false, nil
	}
	if realPath == wrapperPath {
		realPath = currentRealGH
	}
	if realPath == "" {
		return "", false, opErr(GitHubSetupMissingRealGH, "gh points to wrapper %s but AGENT_REAL_GH is not set", wrapperPath)
	}
	return realPath, true, nil
}

// EnsureGHWrapper atomically creates or updates the wrapper symlink pointing to selfExecutable.
func EnsureGHWrapper(agentsHome string, selfExecutable string) (string, *GitHubSetupError) {
	dir := filepath.Join(agentsHome, "run", "gh-trusted")
	if err := os.MkdirAll(dir, 0o750); err != nil {
		return "", fromStorage(GitHubSetupStorage, storageErr(err, "create gh wrapper directory %s", dir))
	}
	wrapperPath := filepath.Join(dir, "gh")
	if target, err := os.Readlink(wrapperPath); err == nil && target == selfExecutable {
		return wrapperPath, nil
	}
	tempPath := fmt.Sprintf("%s.%d.%d", wrapperPath, os.Getpid(), time.Now().UnixNano())
	if err := os.Symlink(selfExecutable, tempPath); err != nil {
		return "", fromStorage(GitHubSetupStorage, storageErr(err, "create temporary gh wrapper symlink %s", tempPath))
	}
	if err := os.Rename(tempPath, wrapperPath); err != nil {
		if rerr := os.Remove(tempPath); rerr != nil { // #nosec G703 -- cleanup temp symlink
			err = errors.Join(err, rerr)
		}
		return "", fromStorage(GitHubSetupStorage, storageErr(err, "rename gh wrapper symlink %s", wrapperPath))
	}
	return wrapperPath, nil
}

// ResolveGitHubTrustedRaw resolves trusted accounts returning the exact oracle format with separator.
func ResolveGitHubTrustedRaw(ctx context.Context, exec Executor, repoRoot string, trustedOverride string) string {
	if trustedOverride != "" {
		return trustedOverride
	}
	if exec == nil {
		exec = OSExecutor{}
	}
	var owner string
	var gitOut bytes.Buffer
	if res, err := exec.Execute(ctx, Command{Name: "git", Args: []string{"-C", repoRoot, "remote", "get-url", "origin"}, Stdout: &gitOut}); err == nil && res.ExitCode == 0 {
		owner = ExtractGitHubOwner(gitOut.String())
	}
	var login string
	var store *ghLoginCacheStore
	if s, ok := ctx.Value(ghLoginCacheKey{}).(*ghLoginCacheStore); ok && s != nil {
		store = s
		store.mu.Lock()
		login = store.login
		store.mu.Unlock()
	} else {
		ghLoginMu.Lock()
		login = ghLoginCache
		ghLoginMu.Unlock()
	}
	if login == "" {
		var ghOut bytes.Buffer
		if res, err := exec.Execute(ctx, Command{Name: "gh", Args: []string{"api", "user", "--jq", ".login"}, Dir: repoRoot, Stdout: &ghOut}); err == nil && res.ExitCode == 0 {
			if login = strings.TrimSpace(ghOut.String()); login != "" {
				if store != nil {
					store.mu.Lock()
					store.login = login
					store.mu.Unlock()
				} else {
					ghLoginMu.Lock()
					ghLoginCache = login
					ghLoginMu.Unlock()
				}
			}
		}
	}
	return fmt.Sprintf("%s %s", owner, login)
}

// ResetGHLoginCacheForTest resets the internal cached GitHub login.
func ResetGHLoginCacheForTest() {
	ghLoginMu.Lock()
	defer ghLoginMu.Unlock()
	ghLoginCache = ""
}

// WithGHLoginCache returns a context that carries an isolated GitHub login cache.
// If ctx already carries a cache store, the existing store is preserved.
func WithGHLoginCache(ctx context.Context) context.Context {
	if ctx.Value(ghLoginCacheKey{}) != nil {
		return ctx
	}
	return context.WithValue(ctx, ghLoginCacheKey{}, &ghLoginCacheStore{})
}

// ResolveGitHubTrusted resolves the trusted accounts (override, or repository owner + authenticated user).
func ResolveGitHubTrusted(ctx context.Context, exec Executor, repoRoot string, trustedOverride string) (string, *GitHubSetupError) {
	if err := ctx.Err(); err != nil {
		return "", fromStorage(GitHubSetupStorage, storageErr(err, "resolve trusted github accounts in %s", repoRoot))
	}
	if ctx.Value(ghLoginCacheKey{}) == nil {
		ctx = WithGHLoginCache(ctx)
	}
	raw := ResolveGitHubTrustedRaw(ctx, exec, repoRoot, trustedOverride)
	return strings.TrimSpace(raw), nil
}

// SetupGitHubRun prepares the per-run gh wrapper and environment variables without mutating global state.
func SetupGitHubRun(ctx context.Context, exec Executor, params GitHubSetupParams) (*GitHubRunEnv, *GitHubSetupError) {
	realPath, found, err := ResolveRealGH(params.LookPath, params.AgentsHome, os.Getenv("AGENT_REAL_GH"))
	if err != nil {
		return nil, err
	}
	if !found {
		if params.Logger != nil {
			if lerr := params.Logger.Log(ctx, LevelWarn, LogAgentsGHMissing, "task="+params.RunnerID, "detail=task="+params.RunnerID); lerr != nil {
				return nil, fromStorage(GitHubSetupStorage, storageErr(lerr, "log agents_gh_missing warning"))
			}
		}
		return &GitHubRunEnv{Env: make(map[string]string), EffectivePath: params.CurrentPath}, nil
	}
	wrapperPath, wrapErr := EnsureGHWrapper(params.AgentsHome, params.SelfExecutable)
	if wrapErr != nil {
		return nil, wrapErr
	}
	if ctx.Value(ghLoginCacheKey{}) == nil {
		ctx = WithGHLoginCache(ctx)
	}
	rawTrusted := ResolveGitHubTrustedRaw(ctx, exec, params.RepoRoot, params.GitHubTrusted)
	wrapperDir := filepath.Dir(wrapperPath)
	effectivePath, prependPath := params.CurrentPath, wrapperDir
	if strings.Contains(":"+params.CurrentPath+":", ":"+wrapperDir+":") {
		prependPath = ""
	} else if effectivePath == "" {
		effectivePath = wrapperDir
	} else {
		effectivePath = wrapperDir + ":" + effectivePath
	}
	return &GitHubRunEnv{
		RealGH: realPath, WrapperPath: wrapperPath, TrustedAccounts: strings.TrimSpace(rawTrusted), RawTrusted: rawTrusted,
		Env: map[string]string{
			"AGENT_REAL_GH": realPath, "AGENT_GITHUB_TRUSTED_RESOLVED": rawTrusted, "AGENT_REVIEW_BYLINE": params.ReviewByline,
			"AGENT_GITHUB_COLORS": params.GitHubColors, "AGENT_GH_COORDINATOR": params.Coordinator, "AGENT_GH_TEAM": params.Team,
			"AGENT_GH_SELF": params.Agent, "AGENT_GH_BUS_DIR": params.BusDir, "AGENTBUS_BIN": params.AgentbusBin,
			"AGENT_GH_OWN_PR": params.OwnPR, "AGENT_GITHUB_NAME_COLORS": params.GitHubNameColors,
		},
		PrependPath: prependPath, EffectivePath: effectivePath,
	}, nil
}
