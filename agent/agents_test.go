package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// agentsExecutor records process boundaries and supplies deterministic bus/unit replies.
type agentsExecutor struct {
	t      *testing.T     // t reports unexpected execution or output errors.
	calls  []Command      // calls preserves complete argv for contract assertions.
	active map[string]int // active maps unit names to systemctl query exit codes.
}

// Execute never starts real processes or reads the live installation.
func (e *agentsExecutor) Execute(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
	e.calls = append(e.calls, cmd)
	switch cmd.Name {
	case "systemctl":
		if len(cmd.Args) != 4 || !reflect.DeepEqual(cmd.Args[:3], []string{"--user", "is-active", "--quiet"}) {
			e.t.Fatalf("unexpected unit query: %+v", cmd)
		}
		if code, ok := e.active[cmd.Args[3]]; ok {
			return CommandResult{ExitCode: code}, nil
		}
		return CommandResult{ExitCode: 3}, nil
	case "fixture-agentbus":
		if _, err := io.WriteString(cmd.Stdout, "Queue summary\nhidden investigation detail\n"); err != nil {
			e.t.Fatal(err)
		}
		return CommandResult{}, nil
	default:
		e.t.Fatalf("unexpected command %q", cmd.Name)
		return CommandResult{}, nil
	}
}

// agentsFixtureConfig keeps config paths, models and queue options explicit.
func agentsFixtureConfig() *Config {
	return &Config{
		Scalars: map[string]string{"AGENTBUS_BIN": "fixture-agentbus", "INVESTIGATION_QUEUE_MAX": "7", "INV_SLOTS": "3"},
		Indexed: map[string]map[string]string{"WORKER_AGENTS": {"2": "cheap"}},
		Associative: map[string]map[string]string{
			"AGENT_TYPE":          {"main": "codex", "cheap": "opencode", "anth": "claude", "google": "gemini", "unknown": "custom"},
			"AGENT_MODEL":         {"main": "", "cheap": "example/model", "anth": "quoted\"model"},
			"INVESTIGATORS_TYPE":  {"inv-z": "codex", "inv-a": "claude"},
			"INVESTIGATORS_MODEL": {"inv-a": "reviewer"},
		},
	}
}

// Rule: text formatting uses default models, sorted investigators and one bus summary line.
func TestAgentsTextFormatting(t *testing.T) {
	cfg := agentsFixtureConfig() // cfg supplies an empty model to exercise the text-only default.
	cfg.Associative["AGENT_TYPE"] = map[string]string{"main": "codex"}
	ex := &agentsExecutor{t: t} // ex guards against real agentbus/systemctl invocation.
	var out bytes.Buffer        // out captures fixed-width rows exactly.
	want := fmt.Sprintf("%-12s %-7s model=%-24s free\n", "main", "codex", "default") +
		fmt.Sprintf("%-12s %-7s model=%-24s investigator\n", "inv-a", "claude", "reviewer") +
		fmt.Sprintf("%-12s %-7s model=%-24s investigator\n", "inv-z", "codex", "default") + "Queue summary\n"
	if err := (&App{Executor: ex}).cmdAgents(&out, cfg, t.TempDir(), "fixture-bus"); err != nil || out.String() != want {
		t.Fatalf("output=%q want=%q error=%v", out.String(), want, err)
	}
	if len(ex.calls) != 1 || !reflect.DeepEqual(ex.calls[0].Args, []string{"inv-list", "--dir", "fixture-bus", "--queue-max", "7", "--inv-slots", "3"}) {
		t.Fatalf("bus calls=%+v", ex.calls)
	}
}

// Rule: sleeping and prelaunch jobs reserve agents, while inactive runs do not.
func TestAgentsBusyConditions(t *testing.T) {
	for _, tc := range []struct {
		name   string // name describes the legacy busy criterion.
		status string // status is a literal metadata value.
		run    string // run is the persisted run counter.
		job    string // job activates prelaunch reservations.
		active bool   // active controls the systemctl reply.
		busy   bool   // busy is the expected visible allocation.
	}{
		{"sleeping", "sleeping", "2", "", false, true},
		{"active", "running", "2", "", true, true},
		{"starting", "starting", "2", "", true, true},
		{"inactive", "running", "2", "", false, false},
		{"finished", "done", "2", "", true, false},
		{"prelaunch-new", "new", "0", "job1", false, true},
		{"prelaunch-starting", "starting", "0", "job1", false, true},
		{"prelaunch-running", "running", "0", "job1", false, true},
		{"unprepared", "new", "0", "", false, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			root := t.TempDir()
			writeAckFixture(t, root, "task", tc.status)
			meta := "ID=task\nSTATUS=" + tc.status + "\nAGENT=main\nRUN=" + tc.run + "\nJOB_ID=" + tc.job + "\n" // meta contains only literal values.
			if err := os.WriteFile(filepath.Join(root, "task", "meta.env"), []byte(meta), 0o600); err != nil {
				t.Fatal(err)
			}
			cfg := agentsFixtureConfig() // cfg keeps one agent so formatting has one allocation to check.
			cfg.Associative["AGENT_TYPE"] = map[string]string{"main": "codex"}
			ex := &agentsExecutor{t: t, active: map[string]int{}} // ex controls active units explicitly.
			if tc.active {
				ex.active["agent-task-r"+tc.run+".service"] = 0
			}
			var out bytes.Buffer // out captures the allocation field.
			if err := (&App{Executor: ex}).cmdAgents(&out, cfg, root, "fixture-bus"); err != nil {
				t.Fatal(err)
			}
			if strings.Contains(out.String(), "busy: task") != tc.busy {
				t.Fatalf("busy=%v output=%q", tc.busy, out.String())
			}
		})
	}
}

// Rule: output failures retain the storage cause and underlying writer failure.
func TestAgentsOutputErrors(t *testing.T) {
	if err := (&App{}).cmdAgents(ackFailWriter{}, agentsFixtureConfig(), t.TempDir(), "fixture-bus"); err == nil || err.Cause != AgentsStorage || !errors.Is(err, io.ErrClosedPipe) {
		t.Fatalf("text output error=%v", err)
	}
}

// Rule: a denied bus subprocess fails with a typed process error instead of succeeding.
func TestAgentsProcessError(t *testing.T) {
	err := (&App{}).cmdAgents(io.Discard, agentsFixtureConfig(), t.TempDir(), "fixture-bus")
	if err == nil || err.Cause != AgentsProcess || !errors.Is(err, ErrProcess) {
		t.Fatalf("want AgentsProcess, got %v", err)
	}
}

// Rule: caller-supplied bus directories are forwarded without reading ambient state.
func TestAgentsCustomBusDirectory(t *testing.T) {
	root := t.TempDir()
	ex := &agentsExecutor{t: t} // ex records the explicitly supplied directory.
	if err := (&App{Executor: ex}).cmdAgents(io.Discard, agentsFixtureConfig(), root, root); err != nil {
		t.Fatal(err)
	}
	if len(ex.calls) != 1 || len(ex.calls[0].Args) < 3 || ex.calls[0].Args[2] != root {
		t.Fatalf("bus directory not forwarded: %+v", ex.calls)
	}
}
