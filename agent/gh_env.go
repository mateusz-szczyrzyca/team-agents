package main

import (
	"context"
)

// LogAgentsGHMissing is emitted when the host environment has no gh binary available.
const LogAgentsGHMissing LogEvent = "agents_gh_missing"

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
func ExtractGitHubOwner(_ string) string {
	return ""
}

// ResolveRealGH locates the system gh executable, avoiding wrapper recursion.
func ResolveRealGH(_ func(string) (string, error), _ string, _ string) (string, bool, *GitHubSetupError) {
	return "", false, nil
}

// EnsureGHWrapper atomically creates or updates the wrapper symlink pointing to selfExecutable.
func EnsureGHWrapper(_ string, _ string) (string, *GitHubSetupError) {
	return "", nil
}

// ResolveGitHubTrusted resolves the trusted accounts (override, or repository owner + authenticated user).
func ResolveGitHubTrusted(_ context.Context, _ Executor, _ string, _ string) (string, *GitHubSetupError) {
	return "", nil
}

// ResolveGitHubTrustedRaw resolves trusted accounts returning the exact oracle format with separator.
func ResolveGitHubTrustedRaw(_ context.Context, _ Executor, _ string, _ string) string {
	return ""
}

// ResetGHLoginCacheForTest resets the internal cached GitHub login.
func ResetGHLoginCacheForTest() {
}

// SetupGitHubRun prepares the per-run gh wrapper and environment variables without mutating global state.
func SetupGitHubRun(_ context.Context, _ Executor, _ GitHubSetupParams) (*GitHubRunEnv, *GitHubSetupError) {
	return nil, nil
}
