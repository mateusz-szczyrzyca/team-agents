package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
	"testing"
)

// agyLines feeds agy's events to the normalizer (as normalize does for the CLI type gemini) and returns its activity.
func agyLines(t *testing.T, events ...string) []activity {
	t.Helper()
	var buf bytes.Buffer
	n := &normalizer{enc: json.NewEncoder(&buf), deltas: map[float64]*strings.Builder{}}
	for _, e := range events {
		var ev map[string]any
		if err := json.Unmarshal([]byte(e), &ev); err != nil {
			t.Fatal(err)
		}
		n.agy(ev)
	}
	var out []activity
	dec := json.NewDecoder(&buf)
	for dec.More() {
		var a activity
		if err := dec.Decode(&a); err != nil {
			t.Fatal(err)
		}
		out = append(out, a)
	}
	return out
}

// agy reports the tokens of each finished step; a run's are summed into one usage line at its result — its input with
// what the cache read, its output with the thinking.
func TestAgyStepsUsageIsOneLineAtTheResult(t *testing.T) {
	step := `{"event":"step_update","step_update":{"step_index":%d,"state":"DONE","step_type":"agent_response","text_delta":"x",` +
		`"usage":{"input_tokens":4738,"output_tokens":1116,"thinking_tokens":372,"cache_read_tokens":150213,"total_tokens":5854}}}`
	got := agyLines(t, fmt.Sprintf(step, 1), fmt.Sprintf(step, 2),
		`{"event":"result","result":{"status":"SUCCESS","response":"STATUS: DONE"}}`)
	n := 0
	for _, a := range got {
		if strings.HasPrefix(a.Text, "usage:") {
			n++
			if want := "usage: in 309902 (cached 300426), out 2976"; a.Text != want {
				t.Errorf("got %q, want %q", a.Text, want)
			}
		}
	}
	if n != 1 {
		t.Errorf("%d usage lines, want one for the run: %v", n, got)
	}
}
