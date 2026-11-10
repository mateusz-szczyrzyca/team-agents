package main

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"testing"
	"time"
)

// colorRegex verifies the oracle hex color format ^#[0-9A-Fa-f]{6}$.
var colorRegex = regexp.MustCompile(`^#[0-9A-Fa-f]{6}$`)

// defaultRegistryPalette provides 8 distinct hex colors matching tests/unit/github_name_registry.bats.
var defaultRegistryPalette = []string{
	"#000001", "#000002", "#000003", "#000004",
	"#000005", "#000006", "#000007", "#000008",
}

// parseNameColorMap parses "name=#hex name2=#hex" into a map.
func parseNameColorMap(formatted string) map[string]string {
	res := make(map[string]string)
	for _, entry := range strings.Fields(formatted) {
		parts := strings.SplitN(entry, "=", 2)
		if len(parts) == 2 {
			res[parts[0]] = parts[1]
		}
	}
	return res
}

// TestRegisterGitHubColors_ConcurrentProcessHelper is invoked as a subprocess to test inter-process file locking.
func TestRegisterGitHubColors_ConcurrentProcessHelper(t *testing.T) {
	if os.Getenv("GO_TEST_REGISTER_COLORS_HELPER") != "1" {
		return
	}
	subDir := filepath.Base(os.Getenv("HELPER_SUBDIR"))
	if subDir == "" || subDir == "." || subDir == "/" {
		t.Fatalf("invalid HELPER_SUBDIR")
	}
	colorsPath := filepath.Join(os.TempDir(), subDir, "github-colors")
	palette := strings.Fields(os.Getenv("HELPER_PALETTE"))
	agentName := os.Getenv("HELPER_AGENT_NAME")
	if len(palette) == 0 || agentName == "" {
		t.Fatalf("missing required helper environment variables")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if err := RegisterGitHubColors(ctx, colorsPath, palette, []string{agentName}, nil); err != nil {
		t.Fatalf("helper RegisterGitHubColors failed: %v", err)
	}
}

// TestRegisterGitHubColors_RosterChangePreservesColors verifies that when NORMAL_AGENTS changes
// across runs, agents keep their previously remembered colors from github-colors.
// Oracle: tests/unit/github_name_registry.bats "a team's names keep their colours when NORMAL_AGENTS changes between rounds".
func TestRegisterGitHubColors_RosterChangePreservesColors(t *testing.T) {
	colorsPath := filepath.Join(t.TempDir(), "github-colors")
	firstNames := []string{"claude-haiku", "codex-1", "gemini-1", "inv-astra", "inv-opus"}
	if err := RegisterGitHubColors(context.Background(), colorsPath, defaultRegistryPalette, firstNames, nil); err != nil {
		t.Fatalf("first RegisterGitHubColors failed: %v", err)
	}

	cfgFirst := newConfig()
	cfgFirst.Indexed["NORMAL_AGENTS"] = map[string]string{"0": "claude-haiku", "1": "codex-1", "2": "gemini-1"}
	cfgFirst.Associative["AGENT_TYPE"] = map[string]string{"claude-haiku": "claude", "codex-1": "codex", "gemini-1": "gemini"}
	cfgFirst.Associative["INVESTIGATORS_TYPE"] = map[string]string{"inv-astra": "codex", "inv-opus": "claude"}
	firstMap := parseNameColorMap(FormatGitHubNameColors(colorsPath, defaultRegistryPalette, cfgFirst))

	// Second round: NORMAL_AGENTS roster changes (claude-haiku dropped, claude-sonnet added)
	cfgSecond := newConfig()
	cfgSecond.Indexed["NORMAL_AGENTS"] = map[string]string{"0": "codex-1", "1": "gemini-1", "2": "claude-sonnet"}
	cfgSecond.Associative["AGENT_TYPE"] = map[string]string{"codex-1": "codex", "gemini-1": "gemini", "claude-sonnet": "claude"}
	cfgSecond.Associative["INVESTIGATORS_TYPE"] = map[string]string{"inv-astra": "codex", "inv-opus": "claude"}
	secondMap := parseNameColorMap(FormatGitHubNameColors(colorsPath, defaultRegistryPalette, cfgSecond))

	for _, n := range firstNames {
		col1, col2 := firstMap[n], secondMap[n]
		if col1 == "" {
			t.Errorf("agent %s missing color in first map", n)
		}
		if col2 != col1 {
			t.Errorf("agent %s color changed across roster update: got %q, want %q", n, col2, col1)
		}
	}
}

// TestRegisterGitHubColors_AllDistinctColors verifies that coordinator, members, and every investigator
// each get a distinct color when the palette has sufficient capacity.
// Oracle: tests/unit/github_name_registry.bats "the coordinator, the members and every investigator each get a colour nobody else has".
func TestRegisterGitHubColors_AllDistinctColors(t *testing.T) {
	colorsPath := filepath.Join(t.TempDir(), "github-colors")
	allAgents := []string{"gemini-1", "claude-haiku", "codex-1", "inv-opus", "inv-astra", "inv-kimi", "inv-gemini-argon"}

	if err := RegisterGitHubColors(context.Background(), colorsPath, defaultRegistryPalette, allAgents, nil); err != nil {
		t.Fatalf("RegisterGitHubColors failed: %v", err)
	}

	cfg := newConfig()
	cfg.Indexed["NORMAL_AGENTS"] = map[string]string{"0": "codex-1", "1": "gemini-1"}
	cfg.Associative["AGENT_TYPE"] = map[string]string{"gemini-1": "gemini", "claude-haiku": "claude", "codex-1": "codex"}
	cfg.Associative["INVESTIGATORS_TYPE"] = map[string]string{"inv-opus": "claude", "inv-astra": "codex", "inv-kimi": "opencode", "inv-gemini-argon": "gemini"}

	assignedColors := parseNameColorMap(FormatGitHubNameColors(colorsPath, defaultRegistryPalette, cfg))
	seenColors := make(map[string]string)

	for _, name := range allAgents {
		color := assignedColors[name]
		if !colorRegex.MatchString(color) {
			t.Errorf("agent %s has invalid color %q", name, color)
			continue
		}
		if prev, exists := seenColors[color]; exists {
			t.Errorf("duplicate color %s assigned to %s and %s", color, prev, name)
		}
		seenColors[color] = name
	}
	if len(seenColors) != len(allAgents) {
		t.Errorf("expected %d distinct colors for 7 agents, got %d", len(allAgents), len(seenColors))
	}
}

// TestRegisterGitHubColors_IncrementalTeamAppend verifies that colors are remembered one name per line,
// and a later team registration appends only new names, preserving exact prior entries.
// Oracle: tests/unit/github_name_registry.bats "the colours are remembered in github-colors, one name a line; a later team adds its new names only".
func TestRegisterGitHubColors_IncrementalTeamAppend(t *testing.T) {
	colorsPath := filepath.Join(t.TempDir(), "github-colors")
	t1Agents := []string{"gemini-1", "claude-haiku", "codex-1", "inv-opus", "inv-astra", "inv-kimi", "inv-gemini-argon"}
	ctx := context.Background()

	if err := RegisterGitHubColors(ctx, colorsPath, defaultRegistryPalette, t1Agents, nil); err != nil {
		t.Fatalf("RegisterGitHubColors T1 failed: %v", err)
	}

	rawT1, err := os.ReadFile(filepath.Clean(colorsPath)) //nolint:gosec // G304: test-controlled path in temp directory
	if err != nil {
		t.Fatalf("ReadFile after T1 failed: %v", err)
	}
	linesT1 := strings.Split(strings.TrimRight(string(rawT1), "\n"), "\n")
	if len(linesT1) != 7 {
		t.Fatalf("expected 7 lines after T1, got %d", len(linesT1))
	}

	// Team 2 registration: claude-sonnet (new) and codex-1 (existing)
	if err := RegisterGitHubColors(ctx, colorsPath, defaultRegistryPalette, []string{"claude-sonnet", "codex-1"}, nil); err != nil {
		t.Fatalf("RegisterGitHubColors T2 failed: %v", err)
	}

	rawT2, err := os.ReadFile(filepath.Clean(colorsPath)) //nolint:gosec // G304: test-controlled path in temp directory
	if err != nil {
		t.Fatalf("ReadFile after T2 failed: %v", err)
	}
	linesT2 := strings.Split(strings.TrimRight(string(rawT2), "\n"), "\n")
	if len(linesT2) != 8 {
		t.Fatalf("expected 8 lines after T2, got %d", len(linesT2))
	}

	for i := 0; i < 7; i++ {
		if linesT2[i] != linesT1[i] {
			t.Errorf("line %d modified: got %q, want %q", i, linesT2[i], linesT1[i])
		}
	}
	if wantLast := "claude-sonnet #000008"; linesT2[7] != wantLast {
		t.Errorf("8th line = %q, want %q", linesT2[7], wantLast)
	}
}

// TestRegisterGitHubColors_PaletteExhaustionLeastUsedTie verifies that when the palette is exhausted,
// a new name gets the color fewest names have, breaking ties by first palette order.
// Oracle: tests/unit/github_name_registry.bats "a palette used up: a new name gets the colour the fewest names have".
func TestRegisterGitHubColors_PaletteExhaustionLeastUsedTie(t *testing.T) {
	colorsPath := filepath.Join(t.TempDir(), "github-colors")
	// Seed with palette where #000001 is used twice, and #000002..#000008 are used once
	initial := strings.Join([]string{
		"inv-opus #000001", "inv-astra #000002", "inv-kimi #000003", "inv-gemini-argon #000004",
		"a #000005", "b #000006", "c #000007", "d #000008", "e #000001",
	}, "\n") + "\n"

	if err := os.WriteFile(colorsPath, []byte(initial), 0o600); err != nil {
		t.Fatalf("WriteFile seed failed: %v", err)
	}
	if err := RegisterGitHubColors(context.Background(), colorsPath, defaultRegistryPalette, []string{"claude-sonnet"}, nil); err != nil {
		t.Fatalf("RegisterGitHubColors failed: %v", err)
	}

	lines, err := readColorLines(colorsPath)
	if err != nil {
		t.Fatalf("readColorLines failed: %v", err)
	}
	var gotColor string
	for _, p := range lines {
		if p[0] == "claude-sonnet" {
			gotColor = p[1]
			break
		}
	}
	if wantColor := "#000002"; gotColor != wantColor {
		t.Errorf("claude-sonnet assigned color %q, want %q", gotColor, wantColor)
	}
}

// TestRegisterGitHubColors_ConcurrentProcessRace tests concurrent inter-process execution of
// RegisterGitHubColors, ensuring that flock on .lock guarantees distinct colors without collisions.
// Oracle: tests/unit/github_name_registry.bats "two runs starting at once never give two names one colour".
func TestRegisterGitHubColors_ConcurrentProcessRace(t *testing.T) {
	tmp, err := os.MkdirTemp(os.TempDir(), "gh-reg-race-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if remErr := os.RemoveAll(tmp); remErr != nil {
			t.Errorf("cleanup RemoveAll(%s) failed: %v", tmp, remErr)
		}
	})
	subDir := filepath.Base(tmp)
	colorsPath := filepath.Join(tmp, "github-colors")
	agents := []string{"gemini-1", "claude-haiku", "codex-1", "claude-sonnet"}
	var cmds []*exec.Cmd

	for _, name := range agents {
		cmd := exec.Command(os.Args[0], "-test.run=^TestRegisterGitHubColors_ConcurrentProcessHelper$") //nolint:gosec // G204: test-only subprocess re-invoking test binary
		cmd.Env = append(os.Environ(),
			"GO_TEST_REGISTER_COLORS_HELPER=1",
			"HELPER_SUBDIR="+subDir,
			"HELPER_PALETTE="+strings.Join(defaultRegistryPalette, " "),
			"HELPER_AGENT_NAME="+name,
		)
		cmds = append(cmds, cmd)
	}

	var wg sync.WaitGroup
	errs := make([]error, len(cmds))
	for i, cmd := range cmds {
		wg.Add(1)
		go func(idx int, c *exec.Cmd) {
			defer wg.Done()
			out, err := c.CombinedOutput()
			if err != nil {
				errs[idx] = fmt.Errorf("process %d failed (%w): %s", idx, err, string(out))
			}
		}(i, cmd)
	}
	wg.Wait()

	for _, err := range errs {
		if err != nil {
			t.Fatalf("concurrent process error: %v", err)
		}
	}

	lines, rerr := readColorLines(colorsPath)
	if rerr != nil {
		t.Fatalf("readColorLines failed: %v", rerr)
	}
	if len(lines) != len(agents) {
		t.Fatalf("expected %d lines, got %d", len(agents), len(lines))
	}

	seen := make(map[string]string)
	for _, p := range lines {
		if prev, exists := seen[p[1]]; exists {
			t.Errorf("duplicate color %s assigned to %s and %s during concurrent race", p[1], prev, p[0])
		}
		seen[p[1]] = p[0]
	}
}

// TestEnsureGHWrapper_ConcurrentRace verifies that multiple concurrent callers of EnsureGHWrapper
// all succeed without errors, leave a valid executable symlink pointing to self, and leave no
// temporary files in the wrapper staging directory.
// Oracle: tests/unit/agents_gh_race.bats "runs setting up the agents' gh at once all succeed, and the link is the script".
func TestEnsureGHWrapper_ConcurrentRace(t *testing.T) {
	home := t.TempDir()
	self := filepath.Join(home, "bin", "agent")
	if err := os.MkdirAll(filepath.Dir(self), 0o750); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(self, []byte("#!/bin/sh\necho ok\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(self, 0o700); err != nil { //nolint:gosec // G302: test-only wrapper executable requires execute permissions
		t.Fatal(err)
	}

	wrapDir := filepath.Join(home, "run", "gh-trusted")
	expectedLink := filepath.Join(wrapDir, "gh")

	const concurrency = 10
	var wg sync.WaitGroup
	errs := make([]*GitHubSetupError, concurrency)
	results := make([]string, concurrency)

	for i := 0; i < concurrency; i++ {
		wg.Add(1)
		go func(idx int) {
			defer wg.Done()
			res, err := EnsureGHWrapper(home, self)
			errs[idx] = err
			results[idx] = res
		}(i)
	}
	wg.Wait()

	for i, err := range errs {
		if err != nil {
			t.Errorf("caller %d failed: %v", i, err)
		}
		if results[i] != expectedLink {
			t.Errorf("caller %d returned %q, want %q", i, results[i], expectedLink)
		}
	}

	target, rerr := os.Readlink(expectedLink)
	if rerr != nil {
		t.Fatalf("Readlink failed: %v", rerr)
	}
	if target != self {
		t.Errorf("wrapper target = %q, want %q", target, self)
	}

	info, serr := os.Stat(expectedLink)
	if serr != nil {
		t.Fatalf("Stat wrapper failed: %v", serr)
	}
	if info.Mode()&0o111 == 0 {
		t.Errorf("wrapper is not executable: mode %v", info.Mode())
	}

	entries, derr := os.ReadDir(wrapDir)
	if derr != nil {
		t.Fatalf("ReadDir wrapDir failed: %v", derr)
	}
	for _, e := range entries {
		if e.Name() != "gh" {
			t.Errorf("unexpected leftover file in wrapper dir: %s", e.Name())
		}
	}
}

// TestRegisterGitHubColors_DisabledPaletteNoFileOrOverwrite verifies that when AGENT_GITHUB_COLORS=none
// or empty, no file or lock is created, existing files are not overwritten, and formatting returns empty.
// Oracle: tests/unit/github_name_registry.bats "AGENT_GITHUB_COLORS=none: no colours, nothing remembered".
func TestRegisterGitHubColors_DisabledPaletteNoFileOrOverwrite(t *testing.T) {
	colorsPath := filepath.Join(t.TempDir(), "github-colors")
	ctx := context.Background()

	// None palette on empty directory: nothing created
	if err := RegisterGitHubColors(ctx, colorsPath, []string{"none"}, []string{"gemini-1"}, nil); err != nil {
		t.Fatalf("RegisterGitHubColors none failed: %v", err)
	}
	if _, err := os.Stat(colorsPath); !os.IsNotExist(err) {
		t.Errorf("expected %s to not exist, err=%v", colorsPath, err)
	}
	if _, err := os.Stat(colorsPath + ".lock"); !os.IsNotExist(err) {
		t.Errorf("expected %s.lock to not exist, err=%v", colorsPath, err)
	}

	cfg := newConfig()
	cfg.Indexed["NORMAL_AGENTS"] = map[string]string{"0": "gemini-1"}
	if got := FormatGitHubNameColors(colorsPath, []string{"none"}, cfg); got != "" {
		t.Errorf("FormatGitHubNameColors with 'none' = %q, want empty", got)
	}
	if got := FormatGitHubNameColors(colorsPath, nil, cfg); got != "" {
		t.Errorf("FormatGitHubNameColors with nil = %q, want empty", got)
	}

	// Existing file is not modified or overwritten when palette is disabled
	existingContent := "gemini-1 #0969DA\n"
	if err := os.WriteFile(colorsPath, []byte(existingContent), 0o600); err != nil {
		t.Fatalf("WriteFile failed: %v", err)
	}
	if err := RegisterGitHubColors(ctx, colorsPath, []string{"none"}, []string{"claude-haiku"}, nil); err != nil {
		t.Fatalf("RegisterGitHubColors with none on existing file failed: %v", err)
	}
	raw, err := os.ReadFile(filepath.Clean(colorsPath)) //nolint:gosec // G304: test-controlled path in temp directory
	if err != nil {
		t.Fatalf("ReadFile failed: %v", err)
	}
	if string(raw) != existingContent {
		t.Errorf("existing content modified: got %q, want %q", string(raw), existingContent)
	}
}
