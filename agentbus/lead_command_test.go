package main

import (
	"net/netip"
	"testing"
)

// TestValidateLeadListenAccepts checks the addresses the lead may listen on: loopback (its
// own node and tests) and the Tailscale ranges (IPv4 CGNAT 100.64.0.0/10, IPv6 fd7a:115c:a1e0::/48).
func TestValidateLeadListenAccepts(t *testing.T) {
	cases := map[string]string{
		"100.101.102.103:7780":     "100.101.102.103:7780",
		"127.0.0.1:7780":           "127.0.0.1:7780",
		"127.0.0.2:7780":           "127.0.0.2:7780",
		"[::1]:7780":               "[::1]:7780",
		"[fd7a:115c:a1e0::1]:7780": "[fd7a:115c:a1e0::1]:7780",
	}
	for in, want := range cases {
		got, err := validateLeadListen(in)
		if err != nil {
			t.Errorf("validateLeadListen(%q): unexpected error %v", in, err)
			continue
		}
		if got != netip.MustParseAddrPort(want) {
			t.Errorf("validateLeadListen(%q) = %v, want %v", in, got, want)
		}
	}
}

// TestValidateLeadListenRefuses checks each refusal with its exact cause.
func TestValidateLeadListenRefuses(t *testing.T) {
	cases := []struct {
		addr string
		want LeadServeCause
	}{
		{"", LeadServeMissingArgument},
		{"100.101.102.103", LeadServeInvalidListen},
		{"100.101.102.103:99999", LeadServeInvalidListen},
		{"lead.example.com:7780", LeadServeInvalidListen},
		{"0.0.0.0:7780", LeadServeUntrustedBind},
		{"[::]:7780", LeadServeUntrustedBind},
		{"203.0.113.5:7780", LeadServeUntrustedBind},
		{"[2001:db8::1]:7780", LeadServeUntrustedBind},
	}
	for _, c := range cases {
		_, err := validateLeadListen(c.addr)
		failsWith(t, c.want)(c.addr, err)
	}
}
