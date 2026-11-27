package main

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"
)

func TestBusFindingToolsAreAvailableToEveryRole(t *testing.T) {
	for _, setup := range []func(*Server){func(*Server) {}, asWorker, func(s *Server) { s.role = RoleInvestigator }, func(s *Server) { s.team = "T" }, func(s *Server) { s.remote = "https://example.com" }} {
		s := server(newBus(t), "a", setup)
		names := map[string]bool{}
		for _, def := range s.tools() {
			names[def["name"].(string)] = true
		}
		if !names["finding_add"] || !names["finding_resolve"] {
			t.Fatalf("missing finding tools for role %s tier %s team %s remote %s", s.role, s.tier, s.team, s.remote)
		}
	}
}

func TestBusFindingDispatcherAndTypedFailures(t *testing.T) {
	s := server(newBus(t), "a")
	got, err := s.call("finding_add", map[string]any{"kind": "trap", "paths": []any{"./src/a/"}, "text": "note"})
	if err != nil {
		t.Fatal(err)
	}
	if got != "ADDED F-1" {
		t.Fatalf("got %q", got)
	}
	got, err = s.call("finding_resolve", map[string]any{"id": "F-1", "reason": "fixed"})
	if err != nil {
		t.Fatal(err)
	}
	if got != "RESOLVED F-1" {
		t.Fatalf("got %q", got)
	}
	_, err = s.call("finding_add", map[string]any{"kind": "unknown", "text": "note"})
	expectToolError(t, err, FindingAddInvalidKind)
	_, err = s.call("finding_resolve", map[string]any{"id": "F-99", "reason": "fixed"})
	expectToolError(t, err, FindingResolveUnknownFinding)
	for _, paths := range []any{"src/a", []any{1}} {
		_, err = s.call("finding_add", map[string]any{"kind": "trap", "paths": paths, "text": "note"})
		expectToolError(t, err, ToolCallInvalidArgument)
	}
}

func TestBusFindingEventsAndLogsReachOnlyTheTasksTeam(t *testing.T) {
	logs := captureLogs(t)
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"a": {"normal", "", "active"}})
	s := server(b, "a", inTeam("T"))
	chk(t).ok(toolResult(s.findingAdd("trap", []string{"src/a"}, "note")))
	chk(t).ok(toolResult(s.findingResolve("F-1", "fixed")))
	chk(t).ok(toolResult(s.findingResolve("F-1", "fixed")))
	messages, serr := b.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	if len(messages) != 2 {
		t.Fatalf("got %#v", messages)
	}
	for i, kind := range []MessageKind{"finding_added", "finding_resolved"} {
		if messages[i].Kind != kind || !kind.Valid() || messages[i].To != teamRecipient("T") {
			t.Fatalf("got %#v", messages[i])
		}
	}
	for _, event := range []string{"finding_added", "finding_resolved"} {
		if !strings.Contains(logs.String(), `"event":"`+event+`"`) {
			t.Fatalf("missing %s: %s", event, logs.String())
		}
	}
}

func TestBusFindingCLIListsJSONAndExitClasses(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	home := t.TempDir()
	bin := filepath.Join(home, "agentbus")
	build := exec.CommandContext(ctx, "go", "build", "-o", bin, ".") // #nosec G204 -- fixed compiler builds this package into a test-owned directory.
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build: %v %s", err, out)
	}
	b := newBus(t)
	s := server(b, "a")
	chk(t).ok(toolResult(s.findingAdd("trap", []string{"src/a"}, "note")))
	chk(t).ok(toolResult(s.findingAdd("environment", nil, "resolved")))
	chk(t).ok(toolResult(s.findingResolve("F-2", "fixed")))
	want, ferr := b.findings("repo")
	if ferr != nil {
		t.Fatal(ferr)
	}
	for _, tc := range []struct {
		repo        string
		want        []Finding
		missingRepo bool
	}{
		{repo: "repo", want: want}, {repo: "missing", want: []Finding{}}, {repo: "", missingRepo: true},
	} {
		cmd := exec.CommandContext(ctx, bin, "findings", "--dir", b.dir, "--repo", tc.repo, "--json") // #nosec G204 -- executable and arguments belong to this temporary fixture.
		cmd.Env = append(os.Environ(), "AGENTS_HOME="+home)
		out, err := cmd.Output()
		if tc.missingRepo {
			var exit *exec.ExitError
			if !errors.As(err, &exit) || exit.ExitCode() != FindingsMissingRepo.info().class.ExitCode() {
				t.Fatalf("got %v output %q", err, out)
			}
			continue
		}
		if err != nil {
			t.Fatalf("command: %v", err)
		}
		var got []Finding
		if err := json.Unmarshal(out, &got); err != nil {
			t.Fatalf("JSON %q: %v", out, err)
		}
		if !reflect.DeepEqual(got, tc.want) {
			t.Fatalf("got %#v want %#v", got, tc.want)
		}
	}
}
