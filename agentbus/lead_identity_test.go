package main

import (
	"errors"
	"fmt"
	"testing"
)

func init() {
	callTailscaleWhois = func(cli, ip string) (*leadWhoisResponse, *LeadWhoisError) {
		panic(fmt.Sprintf("callTailscaleWhois(%s, %s) without a stub — a test would have asked the real tailscaled", cli, ip))
	}
}

// stubWhois replaces callTailscaleWhois for the duration of a test and restores it during cleanup.
func stubWhois(t *testing.T, fn func(cli, ip string) (*leadWhoisResponse, *LeadWhoisError)) {
	t.Helper()
	prev := callTailscaleWhois
	callTailscaleWhois = fn
	t.Cleanup(func() {
		callTailscaleWhois = prev
	})
}

// TestIdentifyLeadPeerWhoisFailure verifies that Tailscale whois execution errors map to LeadWhoisTailscaleFailed.
func TestIdentifyLeadPeerWhoisFailure(t *testing.T) {
	stubWhois(t, func(_, ip string) (*leadWhoisResponse, *LeadWhoisError) {
		return nil, opErr(LeadWhoisTailscaleFailed, "tailscale whois %s: exit status 1", ip)
	})

	_, err := identifyLeadPeer("100.64.0.5:1234", "lead-node", []string{"node-a"})
	if err == nil {
		t.Fatal("expected failure on whois error, got nil")
	}
	var whoisErr *LeadWhoisError
	if !errors.As(err, &whoisErr) {
		t.Fatalf("expected *LeadWhoisError, got %v", err)
	}
	if whoisErr.Cause != LeadWhoisTailscaleFailed {
		t.Fatalf("cause = %v, want LeadWhoisTailscaleFailed", whoisErr.Cause)
	}
}

// TestIdentifyLeadPeerMalformedOutput verifies that invalid JSON or empty node names map to LeadWhoisMalformedOutput.
func TestIdentifyLeadPeerMalformedOutput(t *testing.T) {
	stubWhois(t, func(_, ip string) (*leadWhoisResponse, *LeadWhoisError) {
		return nil, opErr(LeadWhoisMalformedOutput, "tailscale whois %s: no device in answer", ip)
	})

	_, err := identifyLeadPeer("100.64.0.5:1234", "lead-node", []string{"node-a"})
	if err == nil {
		t.Fatal("expected failure on malformed whois output, got nil")
	}
	var whoisErr *LeadWhoisError
	if !errors.As(err, &whoisErr) {
		t.Fatalf("expected *LeadWhoisError, got %v", err)
	}
	if whoisErr.Cause != LeadWhoisMalformedOutput {
		t.Fatalf("cause = %v, want LeadWhoisMalformedOutput", whoisErr.Cause)
	}
}

// TestParseLeadWhois verifies parsing valid, malformed, and missing-device responses.
func TestParseLeadWhois(t *testing.T) {
	valid := []byte(`{"Node":{"Name":"box1.ts.net.","ComputedName":"box1","Tags":["tag:ci"]}}`)
	resp, err := parseLeadWhois("100.64.0.1", valid)
	if err != nil {
		t.Fatalf("unexpected error parsing valid JSON: %v", err)
	}
	if resp.Node.ComputedName != "box1" || resp.Node.Name != "box1.ts.net." {
		t.Fatalf("unexpected parsed node: %+v", resp.Node)
	}

	invalidJSON := []byte(`{invalid`)
	_, errJSON := parseLeadWhois("100.64.0.1", invalidJSON)
	if errJSON == nil || errJSON.Cause != LeadWhoisMalformedOutput {
		t.Fatalf("expected LeadWhoisMalformedOutput on invalid JSON, got %v", errJSON)
	}

	emptyNode := []byte(`{"Node":{"Name":"","ComputedName":""}}`)
	_, errEmpty := parseLeadWhois("100.64.0.1", emptyNode)
	if errEmpty == nil || errEmpty.Cause != LeadWhoisMalformedOutput {
		t.Fatalf("expected LeadWhoisMalformedOutput on empty node, got %v", errEmpty)
	}
}

// TestIdentifyLeadPeerEdgeCases covers fallback short name derivation and invalid IP formats.
func TestIdentifyLeadPeerEdgeCases(t *testing.T) {
	stubWhois(t, func(_, _ string) (*leadWhoisResponse, *LeadWhoisError) {
		return &leadWhoisResponse{
			Node: leadWhoisNode{
				ComputedName: "",
				Name:         "fallback-node.example.ts.net.",
				Tags:         nil,
			},
		}, nil
	})

	// Name derived from DNS name prefix when ComputedName is empty
	id, err := identifyLeadPeer("100.64.0.55:1234", "custom-lead", []string{"fallback-node"})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if id.Node != "fallback-node" {
		t.Fatalf("expected Node=fallback-node, got %q", id.Node)
	}

	// Local loopback peer with custom lead node name
	localID, errLocal := identifyLeadPeer("127.0.0.1:9999", "custom-lead", nil)
	if errLocal != nil {
		t.Fatalf("unexpected error on local peer: %v", errLocal)
	}
	if !localID.Local || localID.Node != "custom-lead" {
		t.Fatalf("expected custom-lead, got %+v", localID)
	}

	// Empty allowlist refuses any non-local node
	_, errEmptyAllow := identifyLeadPeer("100.64.0.55:1234", "lead-node", nil)
	if errEmptyAllow == nil || errEmptyAllow.Cause != LeadWhoisUntrustedNode {
		t.Fatalf("expected LeadWhoisUntrustedNode with nil allowed, got %v", errEmptyAllow)
	}

	// Invalid remote address
	_, errBadRemote := identifyLeadPeer("not-an-ip", "lead-node", []string{"any"})
	if errBadRemote == nil || errBadRemote.Cause != LeadWhoisUntrustedNode {
		t.Fatalf("expected LeadWhoisUntrustedNode for invalid remote, got %v", errBadRemote)
	}
}
