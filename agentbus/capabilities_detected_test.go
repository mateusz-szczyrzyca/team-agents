package main

import "testing"

// darwinDetection is a Mac with Xcode, an iOS simulator runtime, Go and Apple clang, and an attached rp2350 probe.
func darwinDetection() DetectedCapabilities {
	return DetectedCapabilities{OS: "darwin", Arch: "arm64", Toolchains: []string{"go:go1.23", "clang:Apple clang 16"},
		XcodeVersion: "Xcode 16.1", IOSSimulators: []string{"iOS 18.1"}, HardwareProbes: []string{"rp2350-probe"}}
}

// TestDetectedCapabilitiesFromFacts checks that every recognized detection fact yields its capability, and that
// sensitive namespaces are never derived from detection.
func TestDetectedCapabilitiesFromFacts(t *testing.T) {
	got := map[Capability]bool{}
	for _, c := range detectedCapabilities(darwinDetection()) {
		got[c] = true
	}
	for _, want := range []Capability{"os:darwin", "hw:apple-silicon", "build:go", "build:c", "build:ios", "test:ios-simulator", "hw:rp2350-probe"} {
		if !got[want] {
			t.Errorf("detection did not provide %s: %v", want, got)
		}
	}
	for _, never := range []Capability{"sign:apple", "publish:npm", "build:rust", "hw:jlink"} {
		if got[never] {
			t.Errorf("detection must not provide %s: %v", never, got)
		}
	}
}

// TestDetectedCapabilitiesIgnoreUnknownFacts checks that unknown toolchains, probes and an Xcode-less or
// non-Darwin host do not produce iOS capabilities.
func TestDetectedCapabilitiesIgnoreUnknownFacts(t *testing.T) {
	got := map[Capability]bool{}
	d := DetectedCapabilities{OS: "linux", Arch: "amd64", Toolchains: []string{"node:v22", "python:3.12"},
		XcodeVersion: "Xcode 16.1", IOSSimulators: []string{"iOS 18.1"}, HardwareProbes: []string{"stlink"}}
	for _, c := range detectedCapabilities(d) {
		got[c] = true
	}
	if !got["os:linux"] || !got["hw:x86-64"] {
		t.Fatalf("platform not derived: %v", got)
	}
	for _, never := range []Capability{"build:ios", "test:ios-simulator", "build:go", "hw:jlink"} {
		if got[never] {
			t.Errorf("%s derived from a fact it does not match: %v", never, got)
		}
	}
}

// TestRegistryDiscoversDetectedCapabilities is the regression from the review: a manifest with Xcode and Go but no
// declarations must make build:ios and build:go discoverable.
func TestRegistryDiscoversDetectedCapabilities(t *testing.T) {
	b := newBus(t)
	reg := &HubInstanceRegistry{Bus: b}
	req := instanceRegisterFixture("mac", "s1")
	req.Manifest.Declared = nil
	req.Manifest.Detected = darwinDetection()
	if _, err := reg.Register("mac", "token:mac", req); err != nil {
		t.Fatal(err)
	}
	views, serr := registryDirectory{registry: reg}.Instances()
	if serr != nil || len(views) != 1 {
		t.Fatalf("registry view: %+v %v", views, serr)
	}
	if !views[0].Provides("build:ios") || !views[0].Provides("build:go") || views[0].Provides("sign:apple") {
		t.Fatalf("detected capabilities not discoverable: %v", views[0].Provided)
	}
}

// TestRegistryDirectoryIncludesLeadInstance checks that the lead's own instance is discoverable next to the
// registry's instances, and that a registry entry with the same name is not duplicated.
func TestRegistryDirectoryIncludesLeadInstance(t *testing.T) {
	b := newBus(t)
	reg := &HubInstanceRegistry{Bus: b}
	if _, err := reg.Register("mac", "token:mac", instanceRegisterFixture("mac", "s1")); err != nil {
		t.Fatal(err)
	}
	lead := InstanceView{Name: "pi5", OS: "linux", Arch: "arm64", Provided: []Capability{"build:c", "os:linux"}}
	views, serr := registryDirectory{registry: reg, local: &lead}.Instances()
	if serr != nil || len(views) != 2 || views[0].Name != "mac" || views[1].Name != "pi5" {
		t.Fatalf("lead not listed beside the registry: %+v %v", views, serr)
	}
	if !views[1].Provides("build:c") {
		t.Fatalf("lead capabilities missing: %v", views[1].Provided)
	}

	dup := InstanceView{Name: "mac", Provided: []Capability{"build:c"}}
	views, serr = registryDirectory{registry: reg, local: &dup}.Instances()
	if serr != nil || len(views) != 1 || views[0].Provides("build:c") {
		t.Fatalf("a registry entry must win over the local view of the same name: %+v %v", views, serr)
	}
}

// TestNewLocalDirectoryAddsDetectedCapabilities checks that the single-instance view includes the platform.
func TestNewLocalDirectoryAddsDetectedCapabilities(t *testing.T) {
	d, verr := newLocalDirectory("solo", "")
	if verr != nil {
		t.Fatalf("unexpected error: %v", verr)
	}
	if !d.view.Provides(Capability("os:" + d.view.OS)) {
		t.Fatalf("platform missing from the local view: %v", d.view.Provided)
	}
}
