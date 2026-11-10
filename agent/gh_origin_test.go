package main

import (
	"context"
	"strings"
	"testing"
)

func TestCanonicalGitHubRepoURL_HTTPSFormat(t *testing.T) {
	tests := []struct {
		name  string
		input string
		want  string
	}{
		{"https github.com", "https://github.com/owner/repo", "https://github.com/owner/repo"},
		{"https with git suffix", "https://github.com/owner/repo.git", "https://github.com/owner/repo"},
		{"git@ format", "git@github.com:owner/repo", "https://github.com/owner/repo"},
		{"git@ with git suffix", "git@github.com:owner/repo.git", "https://github.com/owner/repo"},
		{"ssh format", "ssh://git@github.com/owner/repo", "https://github.com/owner/repo"},
		{"ssh with git suffix", "ssh://git@github.com/owner/repo.git", "https://github.com/owner/repo"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := CanonicalGitHubRepoURL(tt.input)
			if got != tt.want {
				t.Errorf("CanonicalGitHubRepoURL(%q) = %q, want %q", tt.input, got, tt.want)
			}
		})
	}
}

func TestCanonicalGitHubRepoURL_OwnerRepoValidation(t *testing.T) {
	tests := []struct {
		name  string
		input string
		want  string
		desc  string
	}{
		{"valid owner", "https://github.com/user-name/repo_name", "https://github.com/user-name/repo_name", "owner with dash and digits"},
		{"owner single char", "https://github.com/a/repo", "https://github.com/a/repo", "single char owner"},
		{"repo with dot", "https://github.com/owner/repo.name", "https://github.com/owner/repo.name", "repo with dot"},
		{"repo with underscore", "https://github.com/owner/repo_name", "https://github.com/owner/repo_name", "repo with underscore"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := CanonicalGitHubRepoURL(tt.input)
			if got != tt.want {
				t.Errorf("CanonicalGitHubRepoURL(%q) = %q, want %q (%s)", tt.input, got, tt.want, tt.desc)
			}
		})
	}
}

func TestCanonicalGitHubRepoURL_InvalidInputs(t *testing.T) {
	tests := []struct {
		name  string
		input string
		desc  string
	}{
		{"non-github https", "https://gitlab.com/owner/repo", "non-GitHub HTTPS origin"},
		{"non-github git@", "git@bitbucket.org:owner/repo", "non-GitHub git@ origin"},
		{"malformed no slashes", "github.com/owner/repo", "missing https:// or git@"},
		{"trailing slash", "https://github.com/owner/repo/", "trailing slash"},
		{"single path component", "https://github.com/owner", "missing repo name"},
		{"empty repo", "https://github.com/owner/", "empty repo name"},
		{"repo is dot", "https://github.com/owner/.", "repo is dot"},
		{"repo is double dot", "https://github.com/owner/..", "repo is double dot"},
		{"invalid owner char", "https://github.com/-owner/repo", "owner starts with dash"},
		{"owner too long", "https://github.com/" + strings.Repeat("a", 40) + "/repo", "owner exceeds 39 chars"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := CanonicalGitHubRepoURL(tt.input)
			if got != "" {
				t.Errorf("CanonicalGitHubRepoURL(%q) = %q, want empty string (%s)", tt.input, got, tt.desc)
			}
		})
	}
}

func TestResolveGitHubRepoURL_ValidRepo(t *testing.T) {
	tmpDir := t.TempDir()
	gitDir := tmpDir

	tests := []struct {
		name   string
		origin string
		want   string
	}{
		{"https origin", "https://github.com/user/repo", "https://github.com/user/repo"},
		{"git@ origin", "git@github.com:user/repo", "https://github.com/user/repo"},
		{"ssh origin", "ssh://git@github.com/user/repo", "https://github.com/user/repo"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			fakeExec := &fakeExecutor{
				results: map[string]fakeExecResult{
					"git": {
						exitCode: 0,
						stdout:   []byte(tt.origin),
					},
				},
			}
			ctx := context.Background()
			logger := &Logger{LogDir: tmpDir}
			got := ResolveGitHubRepoURL(ctx, fakeExec, gitDir, logger)
			if got != tt.want {
				t.Errorf("ResolveGitHubRepoURL(...) = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestResolveGitHubRepoURL_MissingOrigin(t *testing.T) {
	tmpDir := t.TempDir()
	gitDir := tmpDir

	fakeExec := &fakeExecutor{
		results: map[string]fakeExecResult{
			"git": {
				exitCode: 2,
				stderr:   []byte("fatal: not a git repository"),
			},
		},
	}

	ctx := context.Background()
	logger := &Logger{LogDir: tmpDir}
	got := ResolveGitHubRepoURL(ctx, fakeExec, gitDir, logger)
	if got != "" {
		t.Errorf("ResolveGitHubRepoURL(...) with exit 2 = %q, want empty string (silent fail)", got)
	}
}

func TestResolveGitHubRepoURL_OtherGitFailure(t *testing.T) {
	tmpDir := t.TempDir()
	gitDir := tmpDir

	fakeExec := &fakeExecutor{
		results: map[string]fakeExecResult{
			"git": {
				exitCode: 128,
				stderr:   []byte("fatal: repo error"),
			},
		},
	}

	ctx := context.Background()
	logger := &Logger{LogDir: tmpDir}
	got := ResolveGitHubRepoURL(ctx, fakeExec, gitDir, logger)
	if got != "" {
		t.Errorf("ResolveGitHubRepoURL(...) with git failure = %q, want empty string", got)
	}
}

func TestResolveGitHubRepoURL_NonGitHubOrigin(t *testing.T) {
	tmpDir := t.TempDir()
	gitDir := tmpDir

	fakeExec := &fakeExecutor{
		results: map[string]fakeExecResult{
			"git": {
				exitCode: 0,
				stdout:   []byte("https://gitlab.com/user/repo"),
			},
		},
	}

	ctx := context.Background()
	logger := &Logger{LogDir: tmpDir}
	got := ResolveGitHubRepoURL(ctx, fakeExec, gitDir, logger)
	if got != "" {
		t.Errorf("ResolveGitHubRepoURL(...) with non-GitHub origin = %q, want empty string", got)
	}
}

// fakeExecutor implements Executor for testing.
type fakeExecutor struct {
	results map[string]fakeExecResult
}

type fakeExecResult struct {
	exitCode int
	stdout   []byte
	stderr   []byte
}

func (f *fakeExecutor) Execute(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
	result, ok := f.results[cmd.Name]
	if !ok {
		return CommandResult{ExitCode: 1}, nil
	}
	if cmd.Stdout != nil {
		_, _ = cmd.Stdout.Write(result.stdout)
	}
	if cmd.Stderr != nil {
		_, _ = cmd.Stderr.Write(result.stderr)
	}
	return CommandResult{ExitCode: result.exitCode}, nil
}
