package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"reflect"
	"strings"
	"testing"
)

// Rule: each JSON line exposes model, tier and provider with escaped string values.
func TestAgentsJSONRecords(t *testing.T) {
	var out bytes.Buffer // out captures the public line-delimited JSON contract.
	if err := (&App{}).cmdAgentsJSON(&out, agentsFixtureConfig()); err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(strings.TrimSuffix(out.String(), "\n"), "\n") // lines may list normal agents in any map order.
	if len(lines) != 7 {
		t.Fatalf("want 7 records, got %q", out.String())
	}
	want := map[string]map[string]string{ // want checks complete schemas without relying on JSON key order.
		"main":    {"name": "main", "type": "codex", "model": "", "tier": "normal", "provider": "openai"},
		"cheap":   {"name": "cheap", "type": "opencode", "model": "example/model", "tier": "worker", "provider": "example"},
		"anth":    {"name": "anth", "type": "claude", "model": "quoted\"model", "tier": "normal", "provider": "anthropic"},
		"google":  {"name": "google", "type": "gemini", "model": "", "tier": "normal", "provider": "google"},
		"unknown": {"name": "unknown", "type": "custom", "model": "", "tier": "normal", "provider": "other"},
		"inv-a":   {"name": "inv-a", "type": "claude", "model": "reviewer", "tier": "investigator", "provider": ""},
		"inv-z":   {"name": "inv-z", "type": "codex", "model": "", "tier": "investigator", "provider": ""},
	}
	for _, line := range lines {
		var got map[string]string // got decodes one independent JSON object.
		if err := json.Unmarshal([]byte(line), &got); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(got, want[got["name"]]) {
			t.Fatalf("record=%v want=%v", got, want[got["name"]])
		}
		delete(want, got["name"])
	}
	if len(want) != 0 || !strings.Contains(lines[5], `"inv-a"`) || !strings.Contains(lines[6], `"inv-z"`) {
		t.Fatalf("missing=%v output=%q", want, out.String())
	}
}

// Rule: output failures retain the storage cause and underlying writer failure.
func TestAgentsJSONOutputErrors(t *testing.T) {
	if err := (&App{}).cmdAgentsJSON(ackFailWriter{}, agentsFixtureConfig()); err == nil || err.Cause != AgentsJSONStorage || !errors.Is(err, io.ErrClosedPipe) {
		t.Fatalf("JSON output error=%v", err)
	}
}
