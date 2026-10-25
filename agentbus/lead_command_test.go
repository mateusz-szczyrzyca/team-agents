package main

import (
	"context"
	"net"
	"net/http"
	"net/netip"
	"testing"
	"time"
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

// TestLeadListenAddrs checks that the lead serves on its configured address and on loopback
// for its own node, each address once.
func TestLeadListenAddrs(t *testing.T) {
	cases := map[string][]string{
		"100.101.102.103:7780": {"100.101.102.103:7780", "127.0.0.1:7780"},
		"127.0.0.2:7780":       {"127.0.0.2:7780", "127.0.0.1:7780"},
		"127.0.0.1:7780":       {"127.0.0.1:7780"},
	}
	for in, want := range cases {
		ap := netip.MustParseAddrPort(in)
		got := leadListenAddrs(ap)
		if len(got) != len(want) {
			t.Errorf("leadListenAddrs(%s) = %v, want %v", in, got, want)
			continue
		}
		for i := range want {
			if got[i] != netip.MustParseAddrPort(want[i]) {
				t.Errorf("leadListenAddrs(%s)[%d] = %v, want %s", in, i, got[i], want[i])
			}
		}
	}
}

// TestParseLeadConfig checks the flags of `agentbus lead`: required values, the repeated --allow,
// and the default node liveness window of 90 s.
func TestParseLeadConfig(t *testing.T) {
	cfg, err := parseLeadConfig([]string{"--dir", "/tmp/bus", "--listen", "100.101.102.103:7780",
		"--node", "lead-pi", "--allow", "pi5", "--allow", "tag:nodes"})
	if err != nil {
		t.Fatalf("parseLeadConfig: unexpected error %v", err)
	}
	if cfg.dir != "/tmp/bus" || cfg.node != "lead-pi" {
		t.Errorf("dir/node = %q/%q, want /tmp/bus/lead-pi", cfg.dir, cfg.node)
	}
	if cfg.listen != netip.MustParseAddrPort("100.101.102.103:7780") {
		t.Errorf("listen = %v, want 100.101.102.103:7780", cfg.listen)
	}
	if len(cfg.allowed) != 2 || cfg.allowed[0] != "pi5" || cfg.allowed[1] != "tag:nodes" {
		t.Errorf("allowed = %v, want [pi5 tag:nodes]", cfg.allowed)
	}
	if cfg.ttl != 90*time.Second {
		t.Errorf("ttl = %v, want 90s", cfg.ttl)
	}
}

// TestParseLeadConfigRequiresNodeAndListen checks the two arguments that have no default.
func TestParseLeadConfigRequiresNodeAndListen(t *testing.T) {
	_, err := parseLeadConfig([]string{"--listen", "100.101.102.103:7780"})
	failsWith(t, LeadServeMissingArgument)("no --node", err)
	_, err = parseLeadConfig([]string{"--node", "lead-pi"})
	failsWith(t, LeadServeMissingArgument)("no --listen", err)
}

// TestParseLeadConfigTTL checks that --ttl sets the liveness window.
func TestParseLeadConfigTTL(t *testing.T) {
	cfg, err := parseLeadConfig([]string{"--listen", "127.0.0.1:7780", "--node", "lead-pi", "--ttl", "5s"})
	if err != nil {
		t.Fatalf("parseLeadConfig: unexpected error %v", err)
	}
	if cfg.ttl != 5*time.Second {
		t.Errorf("ttl = %v, want 5s", cfg.ttl)
	}
}

// freeLoopbackPort returns a port nothing listens on at the moment of the call.
func freeLoopbackPort(t *testing.T) netip.AddrPort {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	addr := netip.MustParseAddrPort(ln.Addr().String())
	if err := ln.Close(); err != nil {
		t.Fatalf("close probe listener: %v", err)
	}
	return addr
}

// TestServeLeadAnswersAndStops checks that the listener serves the lead API on loopback and
// shuts down cleanly when its context ends, releasing the port.
func TestServeLeadAnswersAndStops(t *testing.T) {
	ap := freeLoopbackPort(t)
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel) // a failing readiness check must not leave the server running
	done := make(chan *LeadServeError, 1)
	go func() {
		done <- runLead(ctx, leadConfig{dir: t.TempDir(), listen: ap, node: "lead-pi", ttl: 90 * time.Second})
	}()
	client := &http.Client{Timeout: 5 * time.Second}
	url := "http://" + ap.String() + "/v1/nodes"
	var resp *http.Response
	var err error
	for i := 0; i < 50; i++ {
		resp, err = client.Get(url)
		if err == nil {
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	if err != nil {
		t.Fatalf("GET %s: %v", url, err)
	}
	if err := resp.Body.Close(); err != nil {
		t.Fatalf("close response: %v", err)
	}
	if resp.StatusCode != http.StatusOK {
		t.Errorf("GET %s: status %d, want 200", url, resp.StatusCode)
	}
	cancel()
	if serr := <-done; serr != nil {
		t.Fatalf("runLead after cancel: %v", serr)
	}
	if conn, derr := net.DialTimeout("tcp", ap.String(), time.Second); derr == nil {
		if err := conn.Close(); err != nil {
			t.Fatalf("close probe connection: %v", err)
		}
		t.Errorf("port %s still accepts connections after shutdown", ap)
	}
}

// TestServeLeadListenFailed checks that an address already in use is a ListenFailed cause.
func TestServeLeadListenFailed(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	t.Cleanup(func() {
		if err := ln.Close(); err != nil {
			t.Errorf("close listener: %v", err)
		}
	})
	ap := netip.MustParseAddrPort(ln.Addr().String())
	serr := runLead(context.Background(), leadConfig{dir: t.TempDir(), listen: ap, node: "lead-pi", ttl: time.Minute})
	failsWith(t, LeadServeListenFailed)(ap.String(), serr)
}

// TestParseLeadConfigRejectsBadNode checks that --node must be a logical name: it is the
// identity of the lead's own node, so it is validated before anything listens.
func TestParseLeadConfigRejectsBadNode(t *testing.T) {
	_, err := parseLeadConfig([]string{"--listen", "127.0.0.1:7780", "--node", "lead pi!"})
	failsWith(t, LeadServeBadFlags)("--node with a space", err)
}

// TestParseLeadConfigRejectsPositionalArguments checks that a stray argument is an error, not
// silently ignored (a mistyped flag value would otherwise shift the others).
func TestParseLeadConfigRejectsPositionalArguments(t *testing.T) {
	_, err := parseLeadConfig([]string{"--listen", "127.0.0.1:7780", "--node", "lead-pi", "extra"})
	failsWith(t, LeadServeBadFlags)("a positional argument", err)
}
