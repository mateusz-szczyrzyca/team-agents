package main

import (
	"context"
	"net/netip"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"
)

// leadTailnetPrefixes are the ranges Tailscale assigns to devices: 100.64.0.0/10 (CGNAT,
// IPv4) and fd7a:115c:a1e0::/48 (IPv6). The lead listens only on these or on loopback.
// Set once at package init and never modified: validateLeadListen only reads it.
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
	dir       string         // the bus directory holding the lead's job registry
	listen    netip.AddrPort // the validated --listen address (Tailscale or loopback)
	node      string         // this instance's name on the lead; the lead is also a node
	allowed   []string       // devices allowed to take jobs: names or tag:…; loopback is always the lead's own node
	ttl       time.Duration  // how long a silent node stays online
	notifyDir string         // directory where completion notification markdown files are written
	notifyCmd string         // optional delivery command run with the notification file
}

// leadDevices collects the repeated --allow flag.
type leadDevices []string

// String renders the collected devices for flag's help output.
func (d *leadDevices) String() string { return strings.Join(*d, ",") }

// Set appends one device name or tag:… (flag.Value).
func (d *leadDevices) Set(v string) error {
	*d = append(*d, v)
	return nil
}

// parseLeadConfig parses the arguments of `agentbus lead`. --listen and --node have no default;
// the liveness window --ttl must be positive.
func parseLeadConfig(args []string) (leadConfig, *LeadServeError) {
	fs := newFlagSet("lead")
	dir := fs.String("dir", defaultDir(), "the bus directory (the lead keeps jobs.json there)")
	listen := fs.String("listen", "", "the lead's Tailscale address, e.g. 100.x.y.z:7780 (loopback is served too)")
	node := fs.String("node", "", "this instance's name on the lead, the lead's own node")
	var allow leadDevices
	fs.Var(&allow, "allow", "a device allowed to take jobs: its name or tag:… (repeat for each)")
	ttl := fs.Duration("ttl", 90*time.Second, "how long a silent node stays online")
	notifyDir := fs.String("notify-dir", "", "directory where completion notification files are written")
	notifyCmd := fs.String("notify-cmd", "", "optional delivery command run with the notification file")
	if err := fs.Parse(args); err != nil {
		return leadConfig{}, opErr(LeadServeBadFlags, "%v", err)
	}
	if fs.NArg() > 0 {
		return leadConfig{}, opErr(LeadServeBadFlags, "unexpected argument %q: agentbus lead takes only flags", fs.Arg(0))
	}
	if *node == "" {
		return leadConfig{}, opErr(LeadServeMissingArgument, "--node is required: this instance's name on the lead")
	}
	if !validJobIdentifier(*node) {
		return leadConfig{}, opErr(LeadServeBadFlags, "--node %q must be a name of letters, digits, '.', '_' or '-', starting with a letter or digit (e.g. lead-pi)", *node)
	}
	addr, err := validateLeadListen(*listen)
	if err != nil {
		return leadConfig{}, err
	}
	if *ttl <= 0 {
		return leadConfig{}, opErr(LeadServeBadFlags, "--ttl %v must be a positive duration, e.g. 90s", *ttl)
	}
	return leadConfig{dir: *dir, listen: addr, node: *node, allowed: allow, ttl: *ttl, notifyDir: *notifyDir, notifyCmd: *notifyCmd}, nil
}

// cmdLead runs `agentbus lead` in the foreground (agent lead runs it under agentlead.service).
// SIGINT and SIGTERM stop the listeners cleanly.
func cmdLead(args []string) {
	cfg, err := parseLeadConfig(args)
	if err != nil {
		exitOn(err.asError())
		return
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	serr := runLead(ctx, cfg)
	stop()
	exitOn(serr.asError())
}
