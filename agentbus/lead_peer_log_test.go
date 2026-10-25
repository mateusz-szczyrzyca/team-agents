package main

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// TestServeLeadLogsRejectedPeer: a broken tailscaled must not look like a denied device. The rejection is still
// answered as unauthorized, and the journal records the event with the real cause.
func TestServeLeadLogsRejectedPeer(t *testing.T) {
	stubWhois(t, func(_, ip string) (*leadWhoisResponse, *LeadWhoisError) {
		return nil, opErr(LeadWhoisTailscaleFailed, "tailscale whois %s: exit status 1", ip)
	})
	logs := captureLogs(t)
	w := httptest.NewRecorder()
	r := httptest.NewRequest(http.MethodPost, "/v1/jobs", nil)
	r.RemoteAddr = "100.64.0.9:5555"
	serveLead(w, r, "lead-pi", []string{"node-a"}, LeadSubmitUnauthorized,
		func(leadIdentity) (any, *OpError[LeadSubmitCause]) {
			t.Fatal("run must not be called for a rejected peer")
			return nil, nil
		})
	if w.Code != http.StatusForbidden {
		t.Fatalf("status %d, want %d", w.Code, http.StatusForbidden)
	}
	out := logs.String()
	if !strings.Contains(out, `"event":"`+string(LogLeadPeerRejected)+`"`) {
		t.Fatalf("no %s record in logs: %s", LogLeadPeerRejected, out)
	}
	if !strings.Contains(out, "exit status 1") {
		t.Fatalf("log lacks the whois error: %s", out)
	}
}
