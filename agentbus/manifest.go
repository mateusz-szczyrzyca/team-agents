// Package main provides node manifest validation, capability checking, and heartbeat lifecycle tracking.
package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"sync"
	"time"
)

const (
	// DefaultManifestTTL is the default time-to-live for an instance manifest (180 s, or 3 missed heartbeats).
	DefaultManifestTTL = 180 * time.Second
	// HeartbeatInterval is the expected cadence between periodic heartbeats (60 s).
	HeartbeatInterval = 60 * time.Second
)

// recognizedNamespaces is the closed set of allowed capability namespaces.
var recognizedNamespaces = map[string]struct{}{
	"os":      {},
	"build":   {},
	"test":    {},
	"hw":      {},
	"model":   {},
	"sign":    {},
	"publish": {},
}

// recognizedCapabilities is the closed set of predefined typed capabilities.
var recognizedCapabilities = map[string]struct{}{
	"os:linux":                {},
	"os:darwin":               {},
	"build:ios":               {},
	"build:android":           {},
	"build:rust":              {},
	"build:go":                {},
	"build:c":                 {},
	"test:ios-simulator":      {},
	"test:android-emulator":   {},
	"test:docker":             {},
	"hw:rp2350-probe":         {},
	"hw:jlink":                {},
	"hw:apple-silicon":        {},
	"hw:x86-64":               {},
	"model:claude-3-7-sonnet": {},
	"model:codex":             {},
	"model:gemini-flash":      {},
	"sign:apple-development":  {},
	"sign:apple-distribution": {},
	"sign:android-keystore":   {},
	"publish:npm":             {},
	"publish:pypi":            {},
	"publish:github-release":  {},
}

// ValidCapabilityNamespace reports whether ns is one of the allowed capability namespaces.
func ValidCapabilityNamespace(ns string) bool {
	_, ok := recognizedNamespaces[ns]
	return ok
}

// ParseCapability splits a capability string into its namespace and identifier.
func ParseCapability(capStr string) (string, string, error) {
	parts := strings.Split(capStr, ":")
	if len(parts) != 2 || parts[0] == "" || parts[1] == "" {
		return "", "", fmt.Errorf("malformed capability %q: must be <namespace>:<identifier>", capStr)
	}
	if !ValidCapabilityNamespace(parts[0]) {
		return "", "", fmt.Errorf("unknown capability namespace %q in %q", parts[0], capStr)
	}
	return parts[0], parts[1], nil
}

// IsRecognizedCapability reports whether capStr belongs to the closed set of capabilities or valid namespace.
func IsRecognizedCapability(capStr string) bool {
	if _, ok := recognizedCapabilities[capStr]; ok {
		return true
	}
	ns, id, err := ParseCapability(capStr)
	return err == nil && ns != "" && id != ""
}

// IsSensitiveCapability reports whether capStr represents an approval-gated capability (sign:* or publish:*).
func IsSensitiveCapability(capStr string) bool {
	return strings.HasPrefix(capStr, "sign:") || strings.HasPrefix(capStr, "publish:")
}

// ValidateCapabilities returns an error if any capability is malformed or invalid.
func ValidateCapabilities(caps []string) error {
	for _, c := range caps {
		ns, id, err := ParseCapability(c)
		if err != nil {
			return err
		}
		if ns == "" || id == "" {
			return fmt.Errorf("malformed capability %q", c)
		}
	}
	return nil
}

// DetectedCapabilities holds hardware, OS, and toolchain capabilities detected automatically by agent node.
type DetectedCapabilities struct {
	OS             string   `json:"os"`                        // operating system: "linux" or "darwin"
	Arch           string   `json:"arch"`                      // cpu architecture: "amd64" or "arm64"
	Toolchains     []string `json:"toolchains"`                // discovered toolchains (e.g. ['go1.23', 'rust1.82'])
	XcodeVersion   string   `json:"xcode_version,omitempty"`   // Xcode version string if present (e.g. '16.1')
	IOSSimulators  []string `json:"ios_simulators,omitempty"`  // installed iOS simulator runtimes
	HardwareProbes []string `json:"hardware_probes,omitempty"` // attached hardware debug probes
}

// SlotState reports task execution capacity on an instance.
type SlotState struct {
	Total int `json:"total"` // total concurrent runner slots
	Free  int `json:"free"`  // currently unallocated runner slots
}

// AgentState reports the execution state of one agent on an instance.
type AgentState struct {
	Name        string `json:"name"`                   // agent name
	Tier        Tier   `json:"tier"`                   // normal or worker
	Role        Role   `json:"role"`                   // agent role
	Status      string `json:"status"`                 // idle, busy, or sleeping
	CurrentTask string `json:"current_task,omitempty"` // currently executing task ID
	Repo        string `json:"repo,omitempty"`         // repository name
	Branch      string `json:"branch,omitempty"`       // branch name
	Worktree    string `json:"worktree,omitempty"`     // worktree path
}

// SubscriptionState reports model provider or API quota status.
type SubscriptionState struct {
	Exhausted      bool      `json:"exhausted"`                 // true when model or provider quota is exhausted
	ExhaustedUntil Timestamp `json:"exhausted_until,omitempty"` // time when quota exhaustion is expected to reset
}

// InstanceState represents the live status, slots, and agents running on an instance.
type InstanceState struct {
	Slots         SlotState                    `json:"slots"`                   // execution capacity
	Agents        []AgentState                 `json:"agents"`                  // configured and running agents
	Subscriptions map[string]SubscriptionState `json:"subscriptions,omitempty"` // quota states
}

// InstanceManifest is the payload reported by agent node during registration and heartbeats.
type InstanceManifest struct {
	InstanceName string               `json:"instance_name"` // unique identifier of the instance (e.g. 'mac', 'pi5')
	Timestamp    Timestamp            `json:"timestamp"`     // instant when the manifest was generated
	TTLSeconds   int                  `json:"ttl_seconds"`   // time-to-live before marking instance unreachable
	Detected     DetectedCapabilities `json:"detected"`      // detected capabilities
	Declared     []string             `json:"declared"`      // explicitly configured capabilities
	CurrentState InstanceState        `json:"current_state"` // execution capacity and active agents
}

// InstanceStatus is the lifecycle state of a registered instance.
type InstanceStatus string

const (
	// InstanceActive means the instance is registered and sending regular heartbeats.
	InstanceActive InstanceStatus = "active"
	// InstanceUnreachable means the instance missed heartbeats beyond its TTL window.
	InstanceUnreachableStatus InstanceStatus = "unreachable"
)

// Valid reports whether s is a recognized instance status.
func (s InstanceStatus) Valid() bool {
	return s == InstanceActive || s == InstanceUnreachableStatus
}

// InstanceRecord persists the lease, manifest, and heartbeat times for an instance.
type InstanceRecord struct {
	InstanceName   string           `json:"instance_name"`    // unique name of the instance
	Status         InstanceStatus   `json:"status"`           // active or unreachable
	Lease          uint64           `json:"lease"`            // current active lease generation
	LeaseExpiresAt Timestamp        `json:"lease_expires_at"` // instant when lease expires
	LastHeartbeat  Timestamp        `json:"last_heartbeat"`   // instant of most recent heartbeat
	LastSequence   uint64           `json:"last_sequence"`    // monotonic heartbeat sequence counter
	Manifest       InstanceManifest `json:"manifest"`         // latest valid manifest
}

// InstanceStore holds all registered instances and the monotonic lease generator.
type InstanceStore struct {
	Instances map[string]*InstanceRecord `json:"instances"`  // keyed by instance name
	NextLease uint64                     `json:"next_lease"` // next lease generation ID
}

// InstanceRegisterRequest is the payload for POST /v1/instances/register.
type InstanceRegisterRequest struct {
	Version  int              `json:"version"`  // protocol version (1)
	Manifest InstanceManifest `json:"manifest"` // initial manifest
}

// InstanceRegisterResponse is the result of POST /v1/instances/register.
type InstanceRegisterResponse struct {
	Version      int       `json:"version"`       // protocol version (1)
	InstanceName string    `json:"instance_name"` // registered instance name
	Lease        uint64    `json:"lease"`         // assigned lease generation
	ExpiresAt    Timestamp `json:"expires_at"`    // lease expiration instant
	TTLSeconds   int       `json:"ttl_seconds"`   // effective TTL in seconds
}

// InstanceHeartbeatRequest is the payload for POST /v1/instances/heartbeat.
type InstanceHeartbeatRequest struct {
	Version      int              `json:"version"`       // protocol version (1)
	InstanceName string           `json:"instance_name"` // registered instance name
	Lease        uint64           `json:"lease"`         // current lease generation
	Sequence     uint64           `json:"sequence"`      // monotonic heartbeat counter
	Manifest     InstanceManifest `json:"manifest"`      // updated manifest
}

// InstanceHeartbeatResponse is the result of POST /v1/instances/heartbeat.
type InstanceHeartbeatResponse struct {
	Version         int       `json:"version"`          // protocol version (1)
	InstanceName    string    `json:"instance_name"`    // instance name
	Lease           uint64    `json:"lease"`            // renewed lease generation
	ExpiresAt       Timestamp `json:"expires_at"`       // updated expiration instant
	AcknowledgedSeq uint64    `json:"acknowledged_seq"` // sequence acknowledged
}

// InstanceManager coordinates instance registration, heartbeats, and TTL tracking.
type InstanceManager struct {
	bus *Bus       // underlying bus directory
	mu  sync.Mutex // guards in-memory state updates
}

// NewInstanceManager creates an instance manager for the given bus.
func NewInstanceManager(b *Bus) *InstanceManager {
	return &InstanceManager{bus: b}
}

// load reads bus/instances.json or returns an empty store if it does not exist.
func (m *InstanceManager) load() (*InstanceStore, *StorageError) {
	p := m.bus.path("instances.json")
	data, err := os.ReadFile(p) //nolint:gosec // p is inside the managed bus state directory
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return &InstanceStore{
				Instances: make(map[string]*InstanceRecord),
				NextLease: 1,
			}, nil
		}
		return nil, storageErr(err, "cannot read %s", p)
	}
	var store InstanceStore
	if err := json.Unmarshal(data, &store); err != nil {
		return nil, storageErr(err, "cannot decode %s", p)
	}
	if store.Instances == nil {
		store.Instances = make(map[string]*InstanceRecord)
	}
	if store.NextLease == 0 {
		store.NextLease = 1
	}
	return &store, nil
}

// save atomically writes bus/instances.json using a temporary file.
func (m *InstanceManager) save(store *InstanceStore) *StorageError {
	p := m.bus.path("instances.json")
	data, err := json.MarshalIndent(store, "", "  ")
	if err != nil {
		return storageErr(err, "cannot marshal instances")
	}
	tmp := fmt.Sprintf("%s.tmp.%d", p, time.Now().UnixNano())
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return storageErr(err, "cannot write %s", tmp)
	}
	if err := os.Rename(tmp, p); err != nil {
		if rerr := os.Remove(tmp); rerr != nil {
			warnSideEffect("clean temporary instances file", storageErr(rerr, "cannot remove %s", tmp))
		}
		return storageErr(err, "cannot rename %s to %s", tmp, p)
	}
	return nil
}

// Register registers an instance with its initial manifest and returns an active lease.
func (m *InstanceManager) Register(authenticatedInstance string, req InstanceRegisterRequest) (*InstanceRegisterResponse, *InstanceRegisterError) {
	m.mu.Lock()
	defer m.mu.Unlock()

	if req.Version != hubProtocolVersion {
		return nil, opErr(InstanceRegisterProtocolVersion, "unsupported protocol version %d", req.Version)
	}
	if req.Manifest.InstanceName == "" {
		return nil, opErr(InstanceRegisterMalformed, "instance_name cannot be empty")
	}
	if authenticatedInstance != "" && req.Manifest.InstanceName != authenticatedInstance {
		return nil, opErr(InstanceRegisterInstanceMismatch, "manifest instance_name %q does not match authenticated instance %q", req.Manifest.InstanceName, authenticatedInstance)
	}
	if err := ValidateCapabilities(req.Manifest.Declared); err != nil {
		return nil, opErr(InstanceRegisterInvalidCapability, "invalid declared capability: %v", err)
	}

	ttl := time.Duration(req.Manifest.TTLSeconds) * time.Second
	if ttl <= 0 {
		ttl = DefaultManifestTTL
		req.Manifest.TTLSeconds = int(DefaultManifestTTL.Seconds())
	}

	store, serr := m.load()
	if serr != nil {
		return nil, fromStorage(InstanceRegisterStorage, serr)
	}

	now := time.Now()
	existing, ok := store.Instances[req.Manifest.InstanceName]
	if ok && existing.Status == InstanceActive && existing.LeaseExpiresAt.After(now) {
		return nil, opErr(InstanceRegisterInstanceNameTaken, "instance %q has an active live lease", req.Manifest.InstanceName)
	}

	lease := store.NextLease
	store.NextLease++
	expiresAt := Timestamp{now.Add(ttl)}

	rec := &InstanceRecord{
		InstanceName:   req.Manifest.InstanceName,
		Status:         InstanceActive,
		Lease:          lease,
		LeaseExpiresAt: expiresAt,
		LastHeartbeat:  Timestamp{now},
		LastSequence:   0,
		Manifest:       req.Manifest,
	}
	store.Instances[req.Manifest.InstanceName] = rec

	if serr := m.save(store); serr != nil {
		return nil, fromStorage(InstanceRegisterStorage, serr)
	}

	return &InstanceRegisterResponse{
		Version:      hubProtocolVersion,
		InstanceName: rec.InstanceName,
		Lease:        lease,
		ExpiresAt:    expiresAt,
		TTLSeconds:   req.Manifest.TTLSeconds,
	}, nil
}

// Heartbeat renews an instance lease and updates its manifest.
func (m *InstanceManager) Heartbeat(authenticatedInstance string, req InstanceHeartbeatRequest) (*InstanceHeartbeatResponse, *InstanceHeartbeatError) {
	m.mu.Lock()
	defer m.mu.Unlock()

	if req.Version != hubProtocolVersion {
		return nil, opErr(InstanceHeartbeatMalformed, "unsupported protocol version %d", req.Version)
	}
	if req.InstanceName == "" {
		return nil, opErr(InstanceHeartbeatMalformed, "instance_name cannot be empty")
	}
	if authenticatedInstance != "" && req.InstanceName != authenticatedInstance {
		return nil, opErr(InstanceHeartbeatInstanceMismatch, "heartbeat instance_name %q does not match authenticated instance %q", req.InstanceName, authenticatedInstance)
	}
	if err := ValidateCapabilities(req.Manifest.Declared); err != nil {
		return nil, opErr(InstanceHeartbeatInvalidManifest, "invalid declared capability: %v", err)
	}

	store, serr := m.load()
	if serr != nil {
		return nil, fromStorage(InstanceHeartbeatStorage, serr)
	}

	rec, ok := store.Instances[req.InstanceName]
	if !ok {
		return nil, opErr(InstanceHeartbeatInstanceNotFound, "instance %q is not registered", req.InstanceName)
	}
	if rec.Lease != req.Lease {
		return nil, opErr(InstanceHeartbeatStaleLease, "heartbeat lease %d does not match active lease %d", req.Lease, rec.Lease)
	}
	if req.Sequence > 0 && req.Sequence <= rec.LastSequence {
		return nil, opErr(InstanceHeartbeatStaleHeartbeat, "stale heartbeat sequence %d (current %d)", req.Sequence, rec.LastSequence)
	}

	ttl := time.Duration(req.Manifest.TTLSeconds) * time.Second
	if ttl <= 0 {
		ttl = DefaultManifestTTL
		req.Manifest.TTLSeconds = int(DefaultManifestTTL.Seconds())
	}

	now := time.Now()
	expiresAt := Timestamp{now.Add(ttl)}

	rec.Status = InstanceActive
	rec.LeaseExpiresAt = expiresAt
	rec.LastHeartbeat = Timestamp{now}
	if req.Sequence > 0 {
		rec.LastSequence = req.Sequence
	}
	rec.Manifest = req.Manifest

	if serr := m.save(store); serr != nil {
		return nil, fromStorage(InstanceHeartbeatStorage, serr)
	}

	return &InstanceHeartbeatResponse{
		Version:         hubProtocolVersion,
		InstanceName:    rec.InstanceName,
		Lease:           rec.Lease,
		ExpiresAt:       expiresAt,
		AcknowledgedSeq: req.Sequence,
	}, nil
}

// LeaseStatus indicates the validity of an instance lease.
type LeaseStatus int

const (
	// LeaseValid indicates the instance is registered, active, and the lease matches.
	LeaseValid LeaseStatus = iota
	// LeaseNotFound indicates the instance is not registered.
	LeaseNotFound
	// LeaseExpired indicates the instance lease TTL has expired.
	LeaseExpired
	// LeaseStale indicates the lease generation does not match the active lease.
	LeaseStale
)

// ValidateLease checks whether the given lease generation is currently valid for the instance.
func (m *InstanceManager) ValidateLease(instance string, lease uint64) (LeaseStatus, *InstanceRecord) {
	m.mu.Lock()
	defer m.mu.Unlock()

	store, err := m.load()
	if err != nil {
		return LeaseNotFound, nil
	}
	rec, ok := store.Instances[instance]
	if !ok {
		return LeaseNotFound, nil
	}
	if rec.Status != InstanceActive || !rec.LeaseExpiresAt.After(time.Now()) {
		return LeaseExpired, rec
	}
	if rec.Lease != lease {
		return LeaseStale, rec
	}
	return LeaseValid, rec
}

// GetInstance returns a copy of the instance record if it exists and is active.
func (m *InstanceManager) GetInstance(name string) (*InstanceRecord, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()

	store, err := m.load()
	if err != nil {
		return nil, false
	}
	rec, ok := store.Instances[name]
	if !ok {
		return nil, false
	}
	return rec, true
}

// ListActiveInstances returns all currently registered active instances whose leases have not expired.
func (m *InstanceManager) ListActiveInstances() map[string]*InstanceRecord {
	m.mu.Lock()
	defer m.mu.Unlock()

	store, err := m.load()
	if err != nil {
		return nil
	}
	now := time.Now()
	res := make(map[string]*InstanceRecord)
	for name, rec := range store.Instances {
		if rec.Status == InstanceActive && rec.LeaseExpiresAt.After(now) {
			res[name] = rec
		}
	}
	return res
}

// ExpireStaleInstances marks instances with expired leases as unreachable and releases their claimed board items.
func (m *InstanceManager) ExpireStaleInstances(now time.Time) ([]string, *StorageError) {
	m.mu.Lock()
	defer m.mu.Unlock()

	store, serr := m.load()
	if serr != nil {
		return nil, serr
	}

	var expired []string
	changed := false
	for name, rec := range store.Instances {
		if rec.Status == InstanceActive && !rec.LeaseExpiresAt.After(now) {
			rec.Status = InstanceUnreachableStatus
			expired = append(expired, name)
			changed = true
		}
	}

	if changed {
		if serr := m.save(store); serr != nil {
			return nil, serr
		}
	}

	// For each expired instance, reconcile the shared bus state:
	// update team members to MemberUnreachable and release claimed board items.
	if len(expired) > 0 {
		serr := transact(m.bus, BoardReleaseStorage, func(st *State) *BoardReleaseError {
			for _, team := range st.Teams {
				if team == nil {
					continue
				}
				for agentName, member := range team.Members {
					if member == nil {
						continue
					}
					// An agent belongs to an instance if its name is qualified "agent@instance" or configured on it.
					parts := strings.Split(agentName, "@")
					agentInstance := ""
					if len(parts) == 2 {
						agentInstance = parts[1]
					}
					isExpired := false
					for _, exp := range expired {
						if agentInstance == exp {
							isExpired = true
							break
						}
					}
					if isExpired && (member.State == MemberActive || member.State == MemberSleeping) {
						member.State = MemberUnreachable
						// Release any claimed board item
						for _, item := range team.Board {
							if item != nil && item.Status == BoardClaimed && (item.Assignee == agentName || item.Assignee == parts[0]) {
								item.Status = BoardTodo
								item.Assignee = ""
								item.Updated = Timestamp{now}
							}
						}
						// Post notification to team coordinator
						if team.Lead != "" {
							noticeText := fmt.Sprintf("instance %q is unreachable; member %s marked unreachable and claimed items released", agentInstance, agentName)
							if _, perr := m.bus.post(st, "hub", team.Lead, EventChat{
								Body: noticeText,
							}); perr != nil {
								warnSideEffect("post unreachable notice", perr)
							}
						}
					}
				}
			}
			return nil
		})
		if serr != nil {
			warnSideEffect("expire stale instances", serr)
		}
	}

	return expired, nil
}

// CapabilityMatch describes an instance with matching capabilities found by find_capability.
type CapabilityMatch struct {
	Instance     string       `json:"instance"`                // instance name
	FreeSlots    int          `json:"free_slots"`              // currently unallocated slots
	TotalSlots   int          `json:"total_slots"`             // total configured runner slots
	ActiveAgents []AgentState `json:"active_agents,omitempty"` // active agents on this instance
}

// findCapability (MCP find_capability) queries registered active instances for a given capability.
func (s *Server) findCapability(capability string) (string, *FindCapabilityError) {
	capability = strings.TrimSpace(capability)
	if capability == "" {
		return "", opErr(FindCapabilityMissingCapability, "capability argument is required")
	}
	if !IsRecognizedCapability(capability) {
		return "", opErr(FindCapabilityInvalidCapability, "unknown or invalid capability %q", capability)
	}

	im := NewInstanceManager(s.bus)
	active := im.ListActiveInstances()

	var matches []CapabilityMatch
	for name, rec := range active {
		hasCap := false
		for _, decl := range rec.Manifest.Declared {
			if decl == capability {
				hasCap = true
				break
			}
		}
		if !hasCap {
			det := rec.Manifest.Detected
			if (det.OS != "" && "os:"+det.OS == capability) ||
				(det.Arch != "" && "hw:"+det.Arch == capability) {
				hasCap = true
			}
			for _, tc := range det.Toolchains {
				if "build:"+tc == capability {
					hasCap = true
					break
				}
			}
			for _, sim := range det.IOSSimulators {
				if capability == "test:ios-simulator" && sim != "" {
					hasCap = true
					break
				}
			}
			for _, probe := range det.HardwareProbes {
				if "hw:"+probe == capability {
					hasCap = true
					break
				}
			}
		}

		if hasCap {
			matches = append(matches, CapabilityMatch{
				Instance:     name,
				FreeSlots:    rec.Manifest.CurrentState.Slots.Free,
				TotalSlots:   rec.Manifest.CurrentState.Slots.Total,
				ActiveAgents: rec.Manifest.CurrentState.Agents,
			})
		}
	}

	data, err := json.MarshalIndent(matches, "", "  ")
	if err != nil {
		return "", fromStorage(FindCapabilityStorage, storageErr(err, "cannot encode capability match result"))
	}
	return string(data), nil
}
