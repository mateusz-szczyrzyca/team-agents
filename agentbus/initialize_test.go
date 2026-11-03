package main

import (
	"bytes"
	"encoding/json"
	"strings"
	"testing"
)

func TestInitializeInstructionsOmitRequestInvestigationForInvestigator(t *testing.T) {
	b := newBus(t)
	var workerOut, invOut bytes.Buffer
	w := server(b, "solo", func(s *Server) { s.out = json.NewEncoder(&workerOut) })
	w.handle(rpc(1, "initialize", map[string]any{}))
	if !strings.Contains(workerOut.String(), "request_investigation") {
		t.Fatalf("worker initialize instructions should mention request_investigation: %s", workerOut.String())
	}

	inv := server(b, "inv-opus", func(s *Server) {
		s.role = RoleInvestigator
		s.out = json.NewEncoder(&invOut)
	})
	inv.handle(rpc(1, "initialize", map[string]any{}))
	if strings.Contains(invOut.String(), "request_investigation") {
		t.Fatalf("investigator initialize instructions must not offer request_investigation: %s", invOut.String())
	}
}
