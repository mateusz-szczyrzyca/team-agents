package main

import (
	"runtime"
	"testing"
)

// TestDetectCapabilities tests detection of the running host environment.
func TestDetectCapabilities(t *testing.T) {
	detected := DetectCapabilities()
	if detected.OS != runtime.GOOS {
		t.Errorf("expected OS %q, got %q", runtime.GOOS, detected.OS)
	}
	if detected.Arch != runtime.GOARCH {
		t.Errorf("expected Arch %q, got %q", runtime.GOARCH, detected.Arch)
	}

	// Go toolchain must be present on test host
	hasGo := false
	for _, tc := range detected.Toolchains {
		if len(tc) > 3 && tc[:3] == "go:" {
			hasGo = true
			break
		}
	}
	if !hasGo {
		t.Error("expected go toolchain to be detected on test host")
	}
}

// TestParseIOSSimulators tests parsing simulator runtime outputs.
func TestParseIOSSimulators(t *testing.T) {
	sample := `== Runtimes ==
iOS 18.1 (18.1 - 22B81) - com.apple.CoreSimulator.SimRuntime.iOS-18-1
tvOS 18.1 (18.1 - 22J5567a) - com.apple.CoreSimulator.SimRuntime.tvOS-18-1
watchOS 11.1 (11.1 - 22R5567a) - com.apple.CoreSimulator.SimRuntime.watchOS-11-1
`
	sims := parseIOSSimulators(sample)
	if len(sims) != 1 || sims[0] != "iOS 18.1 (18.1 - 22B81) - com.apple.CoreSimulator.SimRuntime.iOS-18-1" {
		t.Errorf("unexpected parsed simulators: %+v", sims)
	}
}
