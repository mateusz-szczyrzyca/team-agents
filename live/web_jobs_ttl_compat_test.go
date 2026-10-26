package main

import (
	"testing"
	"time"
)

// TestWebNodeTTLMatchesLeadDurationSyntax preserves NODE_TTL_SEC values already accepted by agent lead.
func TestWebNodeTTLMatchesLeadDurationSyntax(t *testing.T) {
	for _, tc := range []struct {
		raw  string        // existing NODE_TTL_SEC spelling, to which agent lead appends s
		want time.Duration // heartbeat deadline used by the existing lead CLI
	}{{"0.5", 500 * time.Millisecond}, {"1m30", 90 * time.Second}, {"1m", time.Millisecond}} {
		t.Run(tc.raw, func(t *testing.T) {
			ttl, err := parseWebNodeTTL(tc.raw)
			if err != nil || ttl != tc.want {
				t.Fatalf("NODE_TTL_SEC=%s: got %s, %v; lead accepts %s", tc.raw, ttl, err, tc.want)
			}
		})
	}
}
