package main

import (
	"bytes"
	"encoding/json"
	"strings"
	"testing"
)

// TestInitializeOnlyRecommendsAvailableTools checks the same wire contract clients receive.
func TestInitializeOnlyRecommendsAvailableTools(t *testing.T) {
	for _, tc := range []struct {
		name   string   // role combination under test
		role   Role     // bus permissions
		remote string   // nonempty for relayed members
		absent []string // unavailable tool names that instructions must not recommend
	}{
		{"investigator", RoleInvestigator, "", []string{"request_investigation"}},
		{"remote", RoleAgent, "node-a", []string{"reserve", "release", "request_investigation", "wait_for_message"}},
		{"worker", RoleAgent, "", nil},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s := server(newBus(t), "agent")
			s.role, s.remote = tc.role, tc.remote
			var out bytes.Buffer // captured initialize response
			s.out = json.NewEncoder(&out)
			s.handle(rpc(1, "initialize", map[string]any{}))
			var response struct {
				Result struct {
					Instructions string `json:"instructions"` /* prompt delivered to the client */
				} `json:"result"` // JSON-RPC payload
			} // typed envelope for the returned prompt
			if err := json.Unmarshal(out.Bytes(), &response); err != nil {
				t.Fatal(err)
			}
			instructions := response.Result.Instructions
			if !strings.Contains(instructions, "read_messages") {
				t.Fatal("missing startup instructions")
			}
			for _, name := range tc.absent {
				if strings.Contains(instructions, name) {
					t.Errorf("unavailable %s recommended: %s", name, instructions)
				}
			}
			if tc.name == "worker" && !strings.Contains(instructions, "request_investigation") {
				t.Fatal("workers lost escalation guidance")
			}
		})
	}
}
