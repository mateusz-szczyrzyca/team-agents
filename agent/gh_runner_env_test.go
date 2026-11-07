package main

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestResolveCoordinator(t *testing.T) {
	ctx := context.Background()
	leadLookup := func(_ context.Context, team string) (string, *ProcessError) {
		if team == "team-1" {
			return "lead-agent", nil
		}
		return "", nil
	}

	tests := []struct {
		name string
		meta string
		want string
	}{
		{
			name: "solo task",
			meta: "AGENT=gemini-1\n",
			want: "",
		},
		{
			name: "investigation task",
			meta: "AGENT=gemini-1\nTEAM=team-1\nKIND=investigation\n",
			want: "",
		},
		{
			name: "team member",
			meta: "AGENT=gemini-1\nTEAM=team-1\nKIND=worker\n",
			want: "lead-agent",
		},
		{
			name: "team coordinator itself",
			meta: "AGENT=lead-agent\nTEAM=team-1\nKIND=worker\n",
			want: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			m := parseMeta([]byte(tt.meta))
			got := ResolveCoordinator(ctx, m, leadLookup)
			if got != tt.want {
				t.Errorf("ResolveCoordinator() = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestResolveOwnPR(t *testing.T) {
	tests := []struct {
		name string
		meta string
		want string
	}{
		{
			name: "team task has no own PR",
			meta: "TEAM=team-1\nPR_NUMBER=42\n",
			want: "",
		},
		{
			name: "solo worker with PR_NUMBER",
			meta: "KIND=worker\nPR_NUMBER=42\n",
			want: "42",
		},
		{
			name: "solo worker with PR_URL",
			meta: "KIND=worker\nPR_URL=https://github.com/org/repo/pull/123/files\n",
			want: "123",
		},
		{
			name: "investigation phase 1 has no own PR",
			meta: "KIND=investigation\nINVESTIGATION_PHASE=1\nPR_NUMBER=99\n",
			want: "",
		},
		{
			name: "investigation phase 2 has own PR",
			meta: "KIND=investigation\nINVESTIGATION_PHASE=2\nPR_URL=https://github.com/org/repo/pull/456\n",
			want: "456",
		},
		{
			name: "disallowed kind",
			meta: "KIND=other\nPR_NUMBER=10\n",
			want: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			m := parseMeta([]byte(tt.meta))
			got := ResolveOwnPR(m)
			if got != tt.want {
				t.Errorf("ResolveOwnPR() = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestReviewByline(t *testing.T) {
	tests := []struct {
		name  string
		agent string
		typ   string
		model string
		invID string
		want  string
	}{
		{
			name:  "with model",
			agent: "inv-kimi",
			typ:   "claude",
			model: "haiku",
			invID: "225",
			want:  "Review by the team-agents investigator `inv-kimi` (claude, model haiku), request #225.",
		},
		{
			name:  "without model",
			agent: "inv-opus",
			typ:   "codex",
			model: "",
			invID: "226",
			want:  "Review by the team-agents investigator `inv-opus` (codex), request #226.",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := FormatReviewByline(tt.agent, tt.typ, tt.model, tt.invID)
			if got != tt.want {
				t.Errorf("FormatReviewByline() = %q, want %q", got, tt.want)
			}
		})
	}

	// Test ResolveReviewByline gating
	invMeta := parseMeta([]byte("KIND=investigation\nAGENT=inv-1\nTYPE=claude\nINV_ID=10\n"))
	if got := ResolveReviewByline(invMeta); got != "Review by the team-agents investigator `inv-1` (claude), request #10." {
		t.Errorf("ResolveReviewByline() for investigation = %q", got)
	}

	workerMeta := parseMeta([]byte("KIND=worker\nAGENT=inv-1\nTYPE=claude\nINV_ID=10\n"))
	if got := ResolveReviewByline(workerMeta); got != "" {
		t.Errorf("ResolveReviewByline() for worker = %q, want empty", got)
	}
}

func TestResolveTeamAndSelf(t *testing.T) {
	tests := []struct {
		name     string
		runnerID string
		meta     string
		wantTeam string
		wantSelf string
	}{
		{
			name:     "empty runnerID",
			runnerID: "",
			meta:     "TEAM=t1\nAGENT=a1\n",
			wantTeam: "",
			wantSelf: "",
		},
		{
			name:     "investigation task",
			runnerID: "task-1",
			meta:     "TEAM=t1\nAGENT=a1\nKIND=investigation\n",
			wantTeam: "",
			wantSelf: "",
		},
		{
			name:     "solo task",
			runnerID: "task-1",
			meta:     "AGENT=a1\n",
			wantTeam: "",
			wantSelf: "",
		},
		{
			name:     "team task",
			runnerID: "task-1",
			meta:     "TEAM=t1\nAGENT=a1\nKIND=worker\n",
			wantTeam: "t1",
			wantSelf: "a1",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			m := parseMeta([]byte(tt.meta))
			gotTeam, gotSelf := ResolveTeamAndSelf(tt.runnerID, m)
			if gotTeam != tt.wantTeam || gotSelf != tt.wantSelf {
				t.Errorf("ResolveTeamAndSelf() = (%q, %q), want (%q, %q)", gotTeam, gotSelf, tt.wantTeam, tt.wantSelf)
			}
		})
	}
}

func TestDeriveGitHubParams(t *testing.T) {
	tmp := t.TempDir()
	tasksDir := filepath.Join(tmp, "tasks")
	taskID := "task-100"
	metaDir := filepath.Join(tasksDir, taskID)
	if err := os.MkdirAll(metaDir, 0o750); err != nil {
		t.Fatal(err)
	}
	metaContent := "AGENT=gemini-1\nTEAM=team-alpha\nKIND=worker\n"
	if err := os.WriteFile(filepath.Join(metaDir, "meta.env"), []byte(metaContent), 0o600); err != nil {
		t.Fatal(err)
	}

	leadLookup := func(_ context.Context, team string) (string, *ProcessError) {
		if team == "team-alpha" {
			return "codex-1", nil
		}
		return "", nil
	}

	cfg := newConfig()
	params := GitHubRunDerivationParams{
		AgentsHome:     tmp,
		TasksDir:       tasksDir,
		BusDir:         filepath.Join(tmp, "bus"),
		RunnerID:       taskID,
		Config:         cfg,
		TeamLeadLookup: leadLookup,
	}

	setupParams, err := DeriveGitHubParams(context.Background(), params)
	if err != nil {
		t.Fatalf("DeriveGitHubParams failed: %v", err)
	}

	if setupParams.Team != "team-alpha" {
		t.Errorf("setupParams.Team = %q, want team-alpha", setupParams.Team)
	}
	if setupParams.Agent != "gemini-1" {
		t.Errorf("setupParams.Agent = %q, want gemini-1", setupParams.Agent)
	}
	if setupParams.Coordinator != "codex-1" {
		t.Errorf("setupParams.Coordinator = %q, want codex-1", setupParams.Coordinator)
	}
}

func TestDeriveGitHubParams_ColorsFailedWarning(t *testing.T) {
	tmp := t.TempDir()
	taskDir := filepath.Join(tmp, "tasks", "t1")
	if err := os.MkdirAll(taskDir, 0o750); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte("AGENT=a1\nTEAM=t1\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	colorsPath := filepath.Join(tmp, "github-colors")
	if err := os.MkdirAll(colorsPath+".lock", 0o750); err != nil {
		t.Fatal(err)
	}
	cfg := newConfig()
	cfg.Scalars["AGENT_GITHUB_COLORS"] = "#112233"
	logger := &Logger{LogDir: filepath.Join(tmp, "logs"), Subcommand: "runner"}
	params := GitHubRunDerivationParams{
		AgentsHome: tmp, TasksDir: filepath.Join(tmp, "tasks"), BusDir: filepath.Join(tmp, "bus"),
		RunnerID: "t1", Config: cfg, Logger: logger,
	}
	setupParams, err := DeriveGitHubParams(context.Background(), params)
	if err != nil {
		t.Fatalf("DeriveGitHubParams failed: %v", err)
	}
	if setupParams.Agent != "a1" {
		t.Errorf("setupParams.Agent = %q, want a1", setupParams.Agent)
	}
	logData, readErr := os.ReadFile(filepath.Clean(filepath.Join(tmp, "logs", "agent.jsonl"))) //nolint:gosec // test log path
	if readErr != nil {
		t.Fatalf("reading log file failed: %v", readErr)
	}
	logStr := string(logData)
	if !strings.Contains(logStr, string(LogGitHubColorsFailed)) || !strings.Contains(logStr, "file="+colorsPath) {
		t.Errorf("log output missing event or file: %s", logStr)
	}
}

func TestDeriveGitHubParams_EmptyOrMissingRunnerID(t *testing.T) {
	tmp := t.TempDir()
	params := GitHubRunDerivationParams{
		AgentsHome: tmp, TasksDir: filepath.Join(tmp, "tasks"), BusDir: filepath.Join(tmp, "bus"),
		Config: newConfig(),
	}
	if _, err := DeriveGitHubParams(context.Background(), params); err != nil {
		t.Errorf("empty RunnerID failed: %v", err)
	}
	params.RunnerID = "nonexistent"
	if _, err := DeriveGitHubParams(context.Background(), params); err != nil {
		t.Errorf("missing RunnerID meta.env failed: %v", err)
	}
}

// TestDeriveGitHubParams_ColorsFailedDiagnosticContext asserts structured event, file field, and underlying lock failure cause.
func TestDeriveGitHubParams_ColorsFailedDiagnosticContext(t *testing.T) {
	tmp := t.TempDir()
	taskDir := filepath.Join(tmp, "tasks", "t1")
	if err := os.MkdirAll(taskDir, 0o750); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte("AGENT=a1\nTEAM=t1\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	colorsPath := filepath.Join(tmp, "github-colors")
	if err := os.MkdirAll(colorsPath+".lock", 0o750); err != nil {
		t.Fatal(err)
	}
	cfg := newConfig()
	cfg.Scalars["AGENT_GITHUB_COLORS"] = "#112233"
	logger := &Logger{LogDir: filepath.Join(tmp, "logs"), Subcommand: "runner"}
	params := GitHubRunDerivationParams{
		AgentsHome: tmp, TasksDir: filepath.Join(tmp, "tasks"), BusDir: filepath.Join(tmp, "bus"),
		RunnerID: "t1", Config: cfg, Logger: logger,
	}
	if _, err := DeriveGitHubParams(context.Background(), params); err != nil {
		t.Fatalf("DeriveGitHubParams failed: %v", err)
	}
	logData, err := os.ReadFile(filepath.Clean(filepath.Join(tmp, "logs", "agent.jsonl"))) //nolint:gosec // test log path
	if err != nil {
		t.Fatalf("reading log failed: %v", err)
	}
	var rec struct {
		Event, File, Cause string
	}
	if err := json.Unmarshal(bytes.TrimSpace(logData), &rec); err != nil {
		t.Fatalf("json.Unmarshal(%s) failed: %v", logData, err)
	}
	if rec.Event != string(LogGitHubColorsFailed) || rec.File != colorsPath || !strings.Contains(rec.Cause, "github-colors.lock") {
		t.Errorf("unexpected record: %+v", rec)
	}
}
