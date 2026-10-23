package main

import (
	"regexp"
	"sort"
	"strings"
)

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
	switch ns {
	case CapNamespaceOS, CapNamespaceArch, CapNamespaceBuild, CapNamespaceTest, CapNamespaceHW, CapNamespaceNet:
		return true
	default:
		return false
	}
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
	return string(c.Namespace) + ":" + c.Value
}

// capValuePattern matches valid capability value strings ([a-z0-9._-]+).
var capValuePattern = regexp.MustCompile(`^[a-z0-9._-]+$`)

// isValidCapabilityValue reports whether v matches [a-z0-9._-]+.
func isValidCapabilityValue(v string) bool {
	return capValuePattern.MatchString(v)
}

// parseCapability parses a single capability string in namespace:value form.
func parseCapability(s string) (Capability, *CapsParseError) {
	parts := strings.Split(s, ":")
	if len(parts) != 2 || parts[0] == "" {
		return Capability{}, opErr(CapsParseMalformed, "capability %q must have namespace:value form", s)
	}
	ns := CapabilityNamespace(parts[0])
	if !ns.Valid() {
		return Capability{}, opErr(CapsParseUnknownNamespace, "unknown capability namespace %q", parts[0])
	}
	val := parts[1]
	if val == "" {
		return Capability{}, opErr(CapsParseInvalidValue, "capability value in %q cannot be empty", s)
	}
	if !isValidCapabilityValue(val) {
		return Capability{}, opErr(CapsParseInvalidValue, "capability value %q contains invalid characters", val)
	}
	return Capability{
		Namespace: ns,
		Value:     val,
	}, nil
}

// parseCapabilities parses, validates, deduplicates and canonically sorts a slice of capability strings.
func parseCapabilities(raw []string) ([]Capability, *CapsParseError) {
	if len(raw) == 0 {
		return nil, nil
	}
	seen := make(map[string]bool, len(raw))
	res := make([]Capability, 0, len(raw))
	for _, s := range raw {
		c, err := parseCapability(s)
		if err != nil {
			return nil, err
		}
		str := c.String()
		if seen[str] {
			continue
		}
		seen[str] = true
		res = append(res, c)
	}
	sort.Slice(res, func(i, j int) bool {
		return res[i].String() < res[j].String()
	})
	return res, nil
}

// hasAllCapabilities reports whether available contains every capability in required.
func hasAllCapabilities(available, required []Capability) bool {
	if len(required) == 0 {
		return true
	}
	set := make(map[string]struct{}, len(available))
	for _, c := range available {
		set[c.String()] = struct{}{}
	}
	for _, r := range required {
		if _, ok := set[r.String()]; !ok {
			return false
		}
	}
	return true
}
