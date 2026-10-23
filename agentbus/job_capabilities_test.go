package main

import (
	"encoding/json"
	"errors"
	"testing"
)

// TestCapabilityNamespaceValid verifies valid and invalid capability namespaces.
func TestCapabilityNamespaceValid(t *testing.T) {
	valid := []CapabilityNamespace{
		CapNamespaceOS,
		CapNamespaceArch,
		CapNamespaceBuild,
		CapNamespaceTest,
		CapNamespaceHW,
		CapNamespaceNet,
	}
	for _, ns := range valid {
		if !ns.Valid() {
			t.Errorf("expected %q to be valid", ns)
		}
	}

	invalid := []CapabilityNamespace{
		"gpu",
		"sign",
		"publish",
		"linux",
		"",
		"OS",
	}
	for _, ns := range invalid {
		if ns.Valid() {
			t.Errorf("expected %q to be invalid", ns)
		}
	}
}

// TestCapabilityNamespaceUnmarshalJSON verifies JSON decoding into CapabilityNamespace.
func TestCapabilityNamespaceUnmarshalJSON(t *testing.T) {
	var ns CapabilityNamespace
	if err := json.Unmarshal([]byte(`"os"`), &ns); err != nil {
		t.Fatalf("unexpected error unmarshaling valid namespace: %v", err)
	}
	if ns != CapNamespaceOS {
		t.Fatalf("unmarshaled namespace = %q, want %q", ns, CapNamespaceOS)
	}

	var verr *ValueError
	err := json.Unmarshal([]byte(`"gpu"`), &ns)
	if err == nil {
		t.Fatal("expected error unmarshaling unknown namespace")
	}
	if !errors.As(err, &verr) {
		t.Fatalf("expected *ValueError, got %T: %v", err, err)
	}

	err = json.Unmarshal([]byte(`123`), &ns)
	if err == nil {
		t.Fatal("expected error unmarshaling non-string JSON")
	}
	if !errors.As(err, &verr) {
		t.Fatalf("expected *ValueError, got %T: %v", err, err)
	}
}

// TestParseCapabilityMalformed verifies malformed capability strings reject with CapsParseMalformed.
func TestParseCapabilityMalformed(t *testing.T) {
	cases := []string{
		"",
		"linux",
		":linux",
		"os:linux:extra",
		"::",
		":",
	}
	for _, tc := range cases {
		_, err := parseCapability(tc)
		expectCause(t, err, CapsParseMalformed)
	}
}

// TestParseCapabilityUnknownNamespace verifies unsupported namespaces reject with CapsParseUnknownNamespace.
func TestParseCapabilityUnknownNamespace(t *testing.T) {
	cases := []string{
		"gpu:rtx4090",
		"sign:production",
		"publish:crates",
		"unknown:val",
	}
	for _, tc := range cases {
		_, err := parseCapability(tc)
		expectCause(t, err, CapsParseUnknownNamespace)
	}
}

// TestParseCapabilityInvalidValue verifies invalid values reject with CapsParseInvalidValue.
func TestParseCapabilityInvalidValue(t *testing.T) {
	cases := []string{
		"os:",
		"os:Linux",
		"os:linux arm",
		"os:linux@1",
		"os:a/b",
		"os:a,b",
		"os:linux!",
	}
	for _, tc := range cases {
		_, err := parseCapability(tc)
		expectCause(t, err, CapsParseInvalidValue)
	}
}

// TestParseCapabilitySuccess verifies valid capabilities parse correctly.
func TestParseCapabilitySuccess(t *testing.T) {
	cases := []struct {
		input   string
		wantNS  CapabilityNamespace
		wantVal string
		wantStr string
	}{
		{"os:linux", CapNamespaceOS, "linux", "os:linux"},
		{"arch:arm64", CapNamespaceArch, "arm64", "arch:arm64"},
		{"build:go-1.22", CapNamespaceBuild, "go-1.22", "build:go-1.22"},
		{"test:unit_go", CapNamespaceTest, "unit_go", "test:unit_go"},
		{"hw:rp2350", CapNamespaceHW, "rp2350", "hw:rp2350"},
		{"net:tailscale", CapNamespaceNet, "tailscale", "net:tailscale"},
		{"os:debian-12.0", CapNamespaceOS, "debian-12.0", "os:debian-12.0"},
	}

	for _, tc := range cases {
		parsed, err := parseCapability(tc.input)
		if err != nil {
			t.Fatalf("unexpected error parsing %q: %v", tc.input, err)
		}
		if parsed.Namespace != tc.wantNS {
			t.Errorf("Namespace = %q, want %q", parsed.Namespace, tc.wantNS)
		}
		if parsed.Value != tc.wantVal {
			t.Errorf("Value = %q, want %q", parsed.Value, tc.wantVal)
		}
		if parsed.String() != tc.wantStr {
			t.Errorf("String() = %q, want %q", parsed.String(), tc.wantStr)
		}
	}
}

// TestParseCapabilitiesCanonicalAndDedup verifies parsing, deduplicating and canonical sorting.
func TestParseCapabilitiesCanonicalAndDedup(t *testing.T) {
	input := []string{"os:linux", "arch:arm64", "os:linux", "hw:rp2350", "arch:arm64"}
	caps, err := parseCapabilities(input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	want := []string{"arch:arm64", "hw:rp2350", "os:linux"}
	if len(caps) != len(want) {
		t.Fatalf("got %d capabilities, want %d", len(caps), len(want))
	}
	for i, w := range want {
		if caps[i].String() != w {
			t.Errorf("caps[%d] = %q, want %q", i, caps[i].String(), w)
		}
	}

	empty, err := parseCapabilities(nil)
	if err != nil {
		t.Fatalf("unexpected error on nil: %v", err)
	}
	if len(empty) != 0 {
		t.Errorf("expected empty slice on nil input, got %v", empty)
	}

	empty, err = parseCapabilities([]string{})
	if err != nil {
		t.Fatalf("unexpected error on empty: %v", err)
	}
	if len(empty) != 0 {
		t.Errorf("expected empty slice on empty input, got %v", empty)
	}
}

// TestParseCapabilitiesPropagatesError verifies that parseCapabilities stops on error.
func TestParseCapabilitiesPropagatesError(t *testing.T) {
	input := []string{"os:linux", "invalid_no_colon", "hw:rp2350"}
	_, err := parseCapabilities(input)
	expectCause(t, err, CapsParseMalformed)
}

// TestHasAllCapabilities verifies subset matching for capabilities.
func TestHasAllCapabilities(t *testing.T) {
	nodeCaps, err := parseCapabilities([]string{"os:linux", "arch:arm64", "hw:rp2350"})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	reqMatching, err := parseCapabilities([]string{"os:linux", "hw:rp2350"})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !hasAllCapabilities(nodeCaps, reqMatching) {
		t.Errorf("expected nodeCaps to satisfy reqMatching")
	}

	reqMissing, err := parseCapabilities([]string{"os:linux", "hw:rp2040"})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if hasAllCapabilities(nodeCaps, reqMissing) {
		t.Errorf("expected nodeCaps not to satisfy reqMissing")
	}

	if !hasAllCapabilities(nodeCaps, nil) {
		t.Errorf("expected empty requirements to always be satisfied")
	}
}
