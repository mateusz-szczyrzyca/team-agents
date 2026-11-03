package main

import (
	"bytes"
	"encoding/json"
	"strings"
	"testing"
)

// instructionTools are the tools the initialize instructions may name.
var instructionTools = []string{"list_agents", "read_messages", "reserve", "release", "request_investigation", "notebook_read"}

// initialized runs initialize and tools/list on s and returns the instructions and the names of the tools listed.
func initialized(t *testing.T, s *Server) (string, map[string]bool) {
	t.Helper()
	var out bytes.Buffer
	s.out = json.NewEncoder(&out)
	s.handle(rpc(1, "initialize", map[string]any{}))
	s.handle(rpc(2, "tools/list", map[string]any{}))
	dec := json.NewDecoder(&out)
	var init struct {
		Result struct {
			Instructions string `json:"instructions"` // the text under test
		} `json:"result"` // the initialize reply
	}
	var list struct {
		Result struct {
			Tools []struct {
				Name string `json:"name"` // a tool's name
			} `json:"tools"` // the tools offered
		} `json:"result"` // the tools/list reply
	}
	if err := dec.Decode(&init); err != nil {
		t.Fatal(err)
	}
	if err := dec.Decode(&list); err != nil {
		t.Fatal(err)
	}
	names := map[string]bool{}
	for _, tool := range list.Result.Tools {
		names[tool.Name] = true
	}
	return init.Result.Instructions, names
}

// Rule: the initialize instructions name only tools that the run has — an investigator is not told to hand a problem
// to an investigator, a remote member is not told to reserve resources.
func TestInstructionsNameOnlyTheRunsTools(t *testing.T) {
	b := newBus(t)
	cases := map[string]*Server{
		"agent":         server(b, "a1"),
		"worker":        server(b, "w1", asWorker),
		"investigator":  server(b, "i1", func(s *Server) { s.role = RoleInvestigator }),
		"remote member": server(b, "m1@node2", inTeam("T"), func(s *Server) { s.remote = "node2" }),
	}
	for name, s := range cases {
		text, tools := initialized(t, s)
		if !strings.Contains(text, "You are "+s.me) {
			t.Errorf("%s: the instructions do not say who the run is: %q", name, text)
		}
		for _, tool := range instructionTools {
			if strings.Contains(text, tool) && !tools[tool] {
				t.Errorf("%s: the instructions name %s, which the run does not have: %q", name, tool, text)
			}
		}
	}
	if text, _ := initialized(t, server(b, "a2")); !strings.Contains(text, "request_investigation") {
		t.Errorf("an agent is not told about request_investigation: %q", text)
	}
}
