package main

import (
	"context"
	"net"
	"net/netip"
	"testing"
	"time"
)

// TestLeadPartialBindFailureReleasesEarlierAddress verifies that failure of the required local listener
// does not leave the successfully opened configured address reserved by an unsuccessful startup.
func TestLeadPartialBindFailureReleasesEarlierAddress(t *testing.T) {
	occupied, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := occupied.Close(); err != nil {
			t.Error(err)
		}
	}()
	port := netip.MustParseAddrPort(occupied.Addr().String()).Port()
	configured := netip.AddrPortFrom(netip.MustParseAddr("127.0.0.2"), port)
	failure := runLead(context.Background(), leadConfig{dir: t.TempDir(), listen: configured, node: "lead", ttl: time.Minute})
	failsWith(t, LeadServeListenFailed)("required local listener occupied", failure)
	reopened, err := net.Listen("tcp", configured.String())
	if err != nil {
		t.Fatalf("failed startup retained %s: %v", configured, err)
	}
	if err := reopened.Close(); err != nil {
		t.Fatal(err)
	}
}
