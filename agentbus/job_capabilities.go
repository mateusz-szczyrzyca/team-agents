package main

// CapabilityNamespace identifies the domain of a declared capability.
type CapabilityNamespace string

const (
	// CapNamespaceOS is the operating system namespace (e.g. linux).
	CapNamespaceOS CapabilityNamespace = "os"
	// CapNamespaceArch is the processor architecture namespace (e.g. arm64, amd64).
	CapNamespaceArch CapabilityNamespace = "arch"
	// CapNamespaceBuild is the toolchain or build environment namespace.
	CapNamespaceBuild CapabilityNamespace = "build"
	// CapNamespaceTest is the test environment capability namespace.
	CapNamespaceTest CapabilityNamespace = "test"
	// CapNamespaceHW is the hardware capability namespace (e.g. rp2350).
	CapNamespaceHW CapabilityNamespace = "hw"
	// CapNamespaceNet is the network capability namespace.
	CapNamespaceNet CapabilityNamespace = "net"
)

// Valid reports whether the capability namespace is one of the supported closed set.
func (ns CapabilityNamespace) Valid() bool {
	return false
}

// UnmarshalJSON implements json.Unmarshaler, returning a *ValueError on unknown or non-string values.
func (ns *CapabilityNamespace) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, ns, "capability namespace").asError()
}

// Capability is a single declared or required capability consisting of a closed namespace and a validated value.
type Capability struct {
	// Namespace is the capability's domain.
	Namespace CapabilityNamespace `json:"namespace"`
	// Value is the capability's specific identifier within its namespace.
	Value string `json:"value"`
}

// String formats the capability in its canonical namespace:value form.
func (c Capability) String() string {
	_ = c
	return ""
}

// parseCapability parses a single capability string in namespace:value form.
func parseCapability(s string) (Capability, *CapsParseError) {
	_ = s
	return Capability{}, nil
}

// parseCapabilities parses, validates, deduplicates and canonically sorts a slice of capability strings.
func parseCapabilities(raw []string) ([]Capability, *CapsParseError) {
	_ = raw
	return nil, nil
}

// hasAllCapabilities reports whether available contains every capability in required.
func hasAllCapabilities(available, required []Capability) bool {
	_ = available
	_ = required
	return false
}
