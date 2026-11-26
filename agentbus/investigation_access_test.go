package main

import (
	"encoding/json"
	"errors"
	"io"
	"strings"
	"testing"
)

// investigationServer parses access through the same flags used by serve.
func investigationServer(t *testing.T, access string) *Server {
	t.Helper()
	fs := newFlagSet("serve")
	flags := defineServeFlags(fs)
	args := []string{"--agent", "member", "--task", "task-member", "--dir", t.TempDir()}
	if access != "" {
		args = append(args, "--investigations="+access)
	}
	if err := fs.Parse(args); err != nil {
		t.Fatal(err)
	}
	return flags.server(RoleAgent, TierNormal, json.NewEncoder(io.Discard))
}

func TestInvestigationToolsHiddenWhenOff(t *testing.T) {
	s := investigationServer(t, "off")
	for _, tool := range s.tools() {
		switch tool["name"] {
		case "request_investigation", "investigation_status", "record_decision":
			t.Errorf("disabled tool remains visible: %s", tool["name"])
		}
	}
	if strings.Contains(s.instructions(), "request_investigation") {
		t.Error("initialize instructions advertise disabled investigations")
	}
}

func TestRequestInvestigationRefusedWhenOff(t *testing.T) {
	s := investigationServer(t, "off")
	failsWith(t, RequestInvestigationNotAllowed)(s.requestInvestigation("question", "", false))
	for _, name := range []string{"request_investigation", "investigation_status", "record_decision"} {
		_, err := s.callTool(name, map[string]any{"question": "question", "investigation_id": 1, "decision": "self", "justification": "known"})
		var denied *RequestInvestigationError // all disabled investigation entry points share the permission cause
		if !errors.As(err, &denied) || denied.Cause != RequestInvestigationNotAllowed {
			t.Errorf("%s: got %v, want RequestInvestigationNotAllowed", name, err)
		}
	}
}

func TestInvestigationToolsRemainAvailableByDefaultAndWhenOn(t *testing.T) {
	for _, access := range []string{"", "on"} {
		s := investigationServer(t, access)
		var names []string // only investigation tools, in catalog order
		for _, tool := range s.tools() {
			switch name := tool["name"].(string); name {
			case "request_investigation", "record_decision", "investigation_status":
				names = append(names, name)
			}
		}
		if strings.Join(names, ",") != "request_investigation,record_decision,investigation_status" {
			t.Fatalf("access %q: %v", access, names)
		}
		if _, err := s.requestInvestigation("question", "", false); err != nil {
			t.Fatal(err)
		}
	}
}

func TestInvestigationAccessRejectsInvalidFlags(t *testing.T) {
	for _, raw := range []string{"", "true", "ON", "未知"} {
		fs := newFlagSet("serve")
		defineServeFlags(fs)
		if err := fs.Parse([]string{"--investigations=" + raw}); err == nil {
			t.Errorf("accepted %q", raw)
		}
	}
}

func TestRepliesSaySleeping(t *testing.T) {
	for _, waiting := range []bool{false, true} {
		s := server(newBus(t), "member")
		if waiting {
			s.queueMax = 0
			other := server(s.bus, "other")
			if _, err := other.requestInvestigation("existing", "", false); err != nil {
				t.Fatal(err)
			}
		}
		for attempt := 0; attempt < 2; attempt++ {
			reply, err := s.requestInvestigation("question", "", waiting)
			if err != nil {
				t.Fatal(err)
			}
			if !strings.Contains(reply, "END YOUR TURN with STATUS: SLEEPING") || strings.Contains(reply, "STATUS: INVESTIGATING") {
				t.Errorf("waiting=%v attempt=%d: %s", waiting, attempt, reply)
			}
		}
	}
}

func TestInvestigatorTierTeamTaskCanRequestWhenExplicitlyOn(t *testing.T) {
	s := investigationServer(t, "on")
	s.role = RoleInvestigator
	var names []string // tool names visible to the investigator-tier team task
	for _, tool := range s.tools() {
		switch name := tool["name"].(string); name {
		case "request_investigation", "record_decision", "investigation_status":
			names = append(names, name)
		}
	}
	if strings.Join(names, ",") != "request_investigation,record_decision,investigation_status" {
		t.Fatalf("tools: %v", names)
	}
	if _, err := s.requestInvestigation("question", "", false); err != nil {
		t.Fatal(err)
	}
	failsWith(t, RecordDecisionNotFinished)(s.recordDecision(1, "self", "resolved"))
}
