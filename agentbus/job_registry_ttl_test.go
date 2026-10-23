package main

import (
	"testing"
	"time"
)

// TestRegistryWaitReasonUsesConfiguredTTL checks both the default 90-second window and a shorter override.
func TestRegistryWaitReasonUsesConfiguredTTL(t *testing.T) {
	r := newLeadRegistry()
	regHeartbeat(t, r, regBeat("node-a", []string{"hw:rp2350"}), regAt(0))
	req := regRequest("job-hw")
	req.Requires = []string{"hw:rp2350"}
	regSubmit(t, r, req, regAt(1))
	cases := []struct {
		ttl  time.Duration
		now  Timestamp
		want string
	}{
		{60 * time.Second, regAt(61), "no node with hw:rp2350"},
		{90 * time.Second, regAt(61), "no free normal agent"},
		{90 * time.Second, regAt(90), "no node with hw:rp2350"},
	}
	for _, c := range cases {
		if got := r.waitReason(req.ID, c.now, c.ttl); got != c.want {
			t.Errorf("ttl %v at %v: reason %q, want %q", c.ttl, c.now, got, c.want)
		}
	}
}
