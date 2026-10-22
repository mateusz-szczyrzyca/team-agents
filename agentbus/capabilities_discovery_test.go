package main

import (
	"strings"
	"testing"
	"time"
)

// TestLocalLeadViewIsDiscoveryOnly checks that the lead's own instance is listed as not routable, and that an
// expired lease of the same name is not replaced by the local view (the lead is not connected to the hub).
func TestLocalLeadViewIsDiscoveryOnly(t *testing.T) {
	b := newBus(t)
	clock := time.Date(2026, 10, 22, 16, 0, 0, 0, time.UTC)
	reg := &HubInstanceRegistry{Bus: b, TTL: 180 * time.Second, Now: func() time.Time { return clock }}
	if _, err := reg.Register("mac", "node-mac", instanceRegisterFixture("mac", "session-mac")); err != nil {
		t.Fatal(err)
	}
	clock = clock.Add(time.Hour) // the mac lease has expired; its record is kept
	lead := InstanceView{Name: "mac", OS: "linux", Arch: "arm64", Provided: []Capability{"build:c"}}
	dir := registryDirectory{registry: reg, local: &lead}
	views, serr := dir.Instances()
	if serr != nil || len(views) != 0 {
		t.Fatalf("an expired lease must not be replaced by the local view: %+v %v", views, serr)
	}

	pi := InstanceView{Name: "pi5", OS: "linux", Arch: "arm64", Provided: []Capability{"build:c"}}
	views, serr = registryDirectory{registry: reg, local: &pi}.Instances()
	if serr != nil || len(views) != 1 || views[0].Name != "pi5" || !views[0].Discovery {
		t.Fatalf("a local view without a record must be listed as discovery only: %+v %v", views, serr)
	}

	s := &Server{directory: registryDirectory{registry: reg, local: &pi}}
	text, ferr := s.findCapability("build:c")
	if ferr != nil {
		t.Fatalf("find_capability failed: %v", ferr)
	}
	if !strings.Contains(text, "not routable") {
		t.Fatalf("find_capability must say the local view is not routable: %s", text)
	}
}
