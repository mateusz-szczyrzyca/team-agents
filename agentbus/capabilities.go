// Capability discovery: which instances can run work, what they provide, and the find_capability tool. The board
// uses the same view to decide whether an agent's instance may claim an item with requires.
package main

import (
	"fmt"
	"runtime"
	"sort"
	"strings"
)

// InstanceView is one instance as seen by capability discovery and board claims.
type InstanceView struct {
	Name     string       // instance name, unique within the installation
	OS       string       // operating system family reported by the instance
	Arch     string       // cpu architecture reported by the instance
	Provided []Capability // capabilities the instance offers: declared ones plus its operating system, sorted
	Free     int          // currently idle runner slots
	Total    int          // total runner slots; zero when the instance does not report slots
	Agents   []string     // agents on the instance with their reported status, e.g. "codex-1 (busy)"
}

// Provides reports whether the instance offers capability c.
func (v InstanceView) Provides(c Capability) bool {
	for _, p := range v.Provided {
		if p == c {
			return true
		}
	}
	return false
}

// CapabilityDirectory lists the instances that can run work. The hub implements it with the instance registry
// (live leases only); a single-instance installation implements it with its own configuration.
type CapabilityDirectory interface {
	// Instances returns the live instances. A failed read is a storage error, never an empty list, so a claim
	// cannot be decided on partial data.
	Instances() ([]InstanceView, *StorageError)
}

// localInstanceNameEnv names the environment variable that gives the local instance name.
const localInstanceNameEnv = "INSTANCE_NAME"

// localDirectory describes the one instance of a single-instance installation.
type localDirectory struct {
	view InstanceView // the local instance, fixed when the server starts
}

// Instances returns the local instance as the only live instance.
func (d localDirectory) Instances() ([]InstanceView, *StorageError) {
	return []InstanceView{d.view}, nil
}

// newLocalDirectory builds the local instance from its name, its declared capabilities (comma or space separated,
// as in INSTANCE_CAPABILITIES) and the running platform. An empty name is "local". An unrecognized capability is a
// *ValueError, so the server does not start with a misspelled capability.
func newLocalDirectory(name, declared string) (localDirectory, *ValueError) {
	if name == "" {
		name = "local"
	}
	caps, verr := parseCapabilityList(declared)
	if verr != nil {
		return localDirectory{}, verr
	}
	provided := map[Capability]bool{}
	for _, c := range caps {
		provided[c] = true
	}
	if os := Capability("os:" + runtime.GOOS); os.Valid() {
		provided[os] = true
	}
	return localDirectory{view: InstanceView{Name: name, OS: runtime.GOOS, Arch: runtime.GOARCH, Provided: sortedCaps(provided)}}, nil
}

// parseCapabilityList splits a comma or space separated list and validates every entry against the closed set.
func parseCapabilityList(raw string) ([]Capability, *ValueError) {
	fields := strings.FieldsFunc(raw, func(r rune) bool { return r == ',' || r == ' ' || r == '\t' || r == '\n' })
	out := make([]Capability, 0, len(fields))
	for _, f := range fields {
		c, verr := ParseCapability(f)
		if verr != nil {
			return nil, verr
		}
		out = append(out, c)
	}
	return out, nil
}

// sortedCaps returns the members of a capability set in lexical order, for stable output.
func sortedCaps(set map[Capability]bool) []Capability {
	out := make([]Capability, 0, len(set))
	for c := range set {
		out = append(out, c)
	}
	sort.Slice(out, func(i, j int) bool { return out[i] < out[j] })
	return out
}

// manifestView converts a registered manifest into the view. Only recognized capabilities count, and the
// operating system the node detected always counts as provided.
func manifestView(name string, m InstanceManifest) InstanceView {
	provided := map[Capability]bool{}
	for _, c := range m.Declared {
		if c.Valid() {
			provided[c] = true
		}
	}
	if os := Capability("os:" + m.Detected.OS); os.Valid() {
		provided[os] = true
	}
	agents := make([]string, 0, len(m.CurrentState.Agents))
	for _, a := range m.CurrentState.Agents {
		agents = append(agents, fmt.Sprintf("%s (%s)", a.Name, a.Status))
	}
	return InstanceView{Name: name, OS: m.Detected.OS, Arch: m.Detected.Arch, Provided: sortedCaps(provided),
		Free: m.CurrentState.Slots.Free, Total: m.CurrentState.Slots.Total, Agents: agents}
}

// liveViews reads the registry and returns every instance whose lease has not expired, sorted by name.
func (g *HubInstanceRegistry) liveViews() ([]InstanceView, *StorageError) {
	var views []InstanceView
	serr := g.withState(func(state *hubInstanceState) *StorageError {
		now := g.now()
		for name, rec := range state.Instances {
			if rec == nil || !now.Before(rec.Lease.ExpiresAt.Time) {
				continue
			}
			views = append(views, manifestView(name, rec.Manifest))
		}
		return nil
	})
	sort.Slice(views, func(i, j int) bool { return views[i].Name < views[j].Name })
	return views, serr
}

// registryDirectory is the hub's directory: the live instances of its registry.
type registryDirectory struct {
	registry *HubInstanceRegistry // the lease registry shared with the instance endpoints
}

// Instances returns the live instances of the registry.
func (d registryDirectory) Instances() ([]InstanceView, *StorageError) {
	return d.registry.liveViews()
}

// ownCapabilities returns the capabilities of the instance this server runs on. A server without a directory
// provides nothing. It must be called before any bus transaction: the registry takes the same bus lock.
func (s *Server) ownCapabilities() (map[Capability]bool, *StorageError) {
	provided := map[Capability]bool{}
	if s.directory == nil {
		return provided, nil
	}
	views, serr := s.directory.Instances()
	if serr != nil {
		return nil, serr
	}
	for _, v := range views {
		if v.Name != s.instance {
			continue
		}
		for _, c := range v.Provided {
			provided[c] = true
		}
	}
	return provided, nil
}

// missingCapabilities returns the required capabilities that the provided set lacks, in the order required.
func missingCapabilities(required []Capability, provided map[Capability]bool) []Capability {
	var missing []Capability
	for _, c := range required {
		if !provided[c] {
			missing = append(missing, c)
		}
	}
	return missing
}

// joinCapabilities renders capabilities as a comma-separated list for messages.
func joinCapabilities(caps []Capability) string {
	parts := make([]string, len(caps))
	for i, c := range caps {
		parts[i] = string(c)
	}
	return strings.Join(parts, ", ")
}

// findCapability (MCP find_capability) lists the live instances that offer one capability, with their platform,
// free slots and agents. It is a read: it never changes state.
func (s *Server) findCapability(name string) (string, *FindCapabilityError) {
	c, verr := ParseCapability(strings.TrimSpace(name))
	if verr != nil {
		return "", fromValue(FindCapabilityInvalidCapability, verr)
	}
	if s.directory == nil {
		return "", opErr(FindCapabilityNoDirectory, "capability discovery is not configured for this installation")
	}
	views, serr := s.directory.Instances()
	if serr != nil {
		return "", fromStorage(FindCapabilityStorage, serr)
	}
	var sb strings.Builder
	var names []string
	for _, v := range views {
		names = append(names, v.Name)
	}
	found := 0
	for _, v := range views {
		if !v.Provides(c) {
			continue
		}
		found++
		slots := "slots not reported"
		if v.Total > 0 {
			slots = fmt.Sprintf("%d of %d slots free", v.Free, v.Total)
		}
		agents := "no agents reported"
		if len(v.Agents) > 0 {
			agents = "agents " + strings.Join(v.Agents, ", ")
		}
		fmt.Fprintf(&sb, "- %s (%s/%s): %s; %s\n", v.Name, v.OS, v.Arch, slots, agents)
	}
	if found == 0 {
		live := "none"
		if len(names) > 0 {
			live = strings.Join(names, ", ")
		}
		return fmt.Sprintf("No live instance offers %s (live instances: %s).", c, live), nil
	}
	return fmt.Sprintf("Instances offering %s:\n%s", c, sb.String()), nil
}

// capabilityToolDefs returns the MCP definition of find_capability, available to every agent.
func capabilityToolDefs() []map[string]any {
	return []map[string]any{{
		"name": "find_capability",
		"description": "Find the live instances that offer a capability (e.g. build:ios, hw:rp2350-probe) with their platform, " +
			"free slots and agents. Use it before you board an item that requires a capability.",
		"inputSchema": map[string]any{
			"type": "object",
			"properties": map[string]any{
				"capability": map[string]any{"type": "string", "description": "A recognized capability, namespace:identifier"},
			},
			"required": []string{"capability"},
		},
	}}
}
