package main

import (
	"net/netip"
	"time"
)

// leadTailnetPrefixes are the ranges Tailscale assigns to devices: 100.64.0.0/10 (CGNAT,
// IPv4) and fd7a:115c:a1e0::/48 (IPv6). The lead listens only on these or on loopback.
var leadTailnetPrefixes = []netip.Prefix{
	netip.MustParsePrefix("100.64.0.0/10"),
	netip.MustParsePrefix("fd7a:115c:a1e0::/48"),
}

// validateLeadListen checks the --listen address of `agentbus lead` and returns it parsed.
// The address must be an IP literal with a fixed numeric port. Unspecified and public
// addresses are refused: the lead listens on loopback or on its Tailscale address only.
func validateLeadListen(addr string) (netip.AddrPort, *LeadServeError) {
	if addr == "" {
		return netip.AddrPort{}, opErr(LeadServeMissingArgument,
			"--listen is required: the Tailscale address of this device, e.g. 100.x.y.z:7780")
	}
	ap, err := netip.ParseAddrPort(addr)
	if err != nil || ap.Port() == 0 {
		return netip.AddrPort{}, opErr(LeadServeInvalidListen,
			"--listen %q must be an IP address and a fixed port, e.g. 100.x.y.z:7780 (%v)", addr, err)
	}
	ip := ap.Addr().Unmap()
	if ip.IsLoopback() {
		return ap, nil
	}
	for _, p := range leadTailnetPrefixes {
		if p.Contains(ip) {
			return ap, nil
		}
	}
	return netip.AddrPort{}, opErr(LeadServeUntrustedBind,
		"--listen %s is neither loopback nor a Tailscale address (100.64.0.0/10, fd7a:115c:a1e0::/48); use this device's tailnet address", ap)
}

// leadListenAddrs returns the addresses the lead serves on: the validated address and, so the
// lead's own node can connect, 127.0.0.1 on the same port (once, when ap is that address).
func leadListenAddrs(ap netip.AddrPort) []netip.AddrPort {
	out := []netip.AddrPort{ap}
	if loop := netip.AddrPortFrom(netip.MustParseAddr("127.0.0.1"), ap.Port()); loop != ap {
		out = append(out, loop)
	}
	return out
}

// leadConfig is the parsed command line of `agentbus lead`.
type leadConfig struct {
	dir     string         // the bus directory holding the lead's job registry
	listen  netip.AddrPort // the validated --listen address (Tailscale or loopback)
	node    string         // this instance's name on the lead; the lead is also a node
	allowed []string       // devices allowed to take jobs: names or tag:…; loopback is always the lead's own node
	ttl     time.Duration  // how long a silent node stays online
}

// parseLeadConfig parses the arguments of `agentbus lead`. RED: not implemented yet.
func parseLeadConfig(_ []string) (leadConfig, *LeadServeError) {
	return leadConfig{}, nil
}
