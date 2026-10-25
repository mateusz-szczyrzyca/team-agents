package main

import (
	"net/netip"
)

// validateLeadListen checks the --listen address of `agentbus lead` and returns it parsed.
// The address must be an IP literal with a numeric port. Unspecified and public addresses
// are refused: the lead listens on loopback or on its Tailscale address only.
func validateLeadListen(_ string) (netip.AddrPort, *LeadServeError) {
	return netip.AddrPort{}, nil // RED: not implemented yet
}
