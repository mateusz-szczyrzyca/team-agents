package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A run's steps (its tool calls) are counted; at longRunSteps the normalizer marks the run long — the long-run file in
// its task's directory, read by the agent's bus server — and says so in the activity, once.
func TestALongRunIsMarkedAtItsStepLimit(t *testing.T) {
	dir := t.TempDir()
	var buf bytes.Buffer
	n := &normalizer{enc: json.NewEncoder(&buf), deltas: map[float64]*strings.Builder{}, longRunSteps: 2,
		longRunFlag: filepath.Join(dir, "long-run")}
	call := `{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}`
	for i := 0; i < 3; i++ {
		var ev map[string]any
		if err := json.Unmarshal([]byte(call), &ev); err != nil {
			t.Fatal(err)
		}
		n.claude(ev)
	}
	if _, err := os.Stat(filepath.Join(dir, "long-run")); err != nil {
		t.Fatalf("the run is not marked long after 3 steps (limit 2): %v", err)
	}
	if c := strings.Count(buf.String(), "long run"); c != 1 {
		t.Errorf("the activity says so %d times, want once:\n%s", c, buf.String())
	}
}

// While its run is marked long, every answer of the agent's bus server asks it to end the run; otherwise none does.
func TestTheBusAsksALongRunToEnd(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "w": {"normal", "b", "active"}})
	w := server(b, "w", inTeam("T"))
	w.taskDir = t.TempDir()
	if out, err := w.call("read_messages", map[string]any{}); err != nil || strings.Contains(out, "This run is long") {
		t.Fatalf("not long yet: %q, %v", out, err)
	}
	if err := os.WriteFile(filepath.Join(w.taskDir, "long-run"), []byte("120\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if out, err := w.call("read_messages", map[string]any{}); err != nil || !strings.Contains(out, "This run is long") {
		t.Errorf("the long run is not asked to end: %q, %v", out, err)
	}
}
