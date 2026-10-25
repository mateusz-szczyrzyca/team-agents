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
