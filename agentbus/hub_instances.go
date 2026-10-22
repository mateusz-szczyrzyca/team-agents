// Instance leases live on the hub, separately from the legacy single-instance state.
package main

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// hubInstanceTTL is the default hub-authoritative manifest lifetime.
const hubInstanceTTL = 180 * time.Second

// hubPeerContextKey prevents client-supplied headers from setting the authenticated node identity.
type hubPeerContextKey struct{}

// setHubAuthenticatedPeer attaches identity only after successful transport authentication.
func setHubAuthenticatedPeer(r *http.Request, peer string) {
	*r = *r.WithContext(context.WithValue(r.Context(), hubPeerContextKey{}, peer))
}

// hubAuthenticatedPeer returns the authenticated node identity, never a claimed instance header.
func hubAuthenticatedPeer(r *http.Request) string {
	peer, _ := r.Context().Value(hubPeerContextKey{}).(string)
	return peer
}

// HubLeaseCheck is a lease validation result; storage failures remain separate errors.
type HubLeaseCheck int

const (
	HubLeaseValid     HubLeaseCheck = iota // active lease with matching owner and credentials
	HubLeaseUnknown                        // no registration exists
	HubLeaseExpired                        // hub time has reached the lease deadline
	HubLeaseStale                          // generation or secret token no longer matches
	HubLeaseWrongPeer                      // a different authenticated node owns the instance name
)

// HubInstanceLease is returned only to the authenticated owning node.
type HubInstanceLease struct {
	Version    int       `json:"version"`     // protocol version
	Token      string    `json:"lease_token"` // unguessable credential, not a transport identity
	Generation int64     `json:"generation"`  // advances on replacement, persists across hub restarts
	ExpiresAt  Timestamp `json:"expires_at"`  // hub-authoritative UTC expiry
}

// MarshalJSON preserves UTC instants across machines without changing legacy Timestamp.
func (l HubInstanceLease) MarshalJSON() ([]byte, error) {
	type leaseData HubInstanceLease
	return json.Marshal(struct {
		leaseData        // all fields except overridden expiry
		ExpiresAt string `json:"expires_at"` // RFC 3339 UTC
	}{leaseData: leaseData(l), ExpiresAt: hubTimeString(l.ExpiresAt)})
}

// HubInstanceRegisterRequest starts or reattaches one daemon session.
type HubInstanceRegisterRequest struct {
	Version        int              `json:"version"`         // protocol version
	RegistrationID string           `json:"registration_id"` // random daemon identity, retained for lost-response retries
	Sequence       uint64           `json:"sequence"`        // first manifest report sequence, positive
	Manifest       InstanceManifest `json:"manifest"`        // node capabilities and capacity
}

// HubInstanceHeartbeatRequest renews a registered daemon's lease.
type HubInstanceHeartbeatRequest struct {
	Version    int              `json:"version"`       // protocol version
	Instance   string           `json:"instance_name"` // must match transport authentication and manifest
	Token      string           `json:"lease_token"`   // current session secret
	Generation int64            `json:"generation"`    // current fencing generation
	Sequence   uint64           `json:"sequence"`      // strictly increasing manifest report sequence
	Manifest   InstanceManifest `json:"manifest"`      // fresh capacity and capabilities
}

// hubInstanceRecord is a new, versioned file entry, independent of state.json.
type hubInstanceRecord struct {
	Peer           string           `json:"peer"`            // authenticated owner; client-selected aliases cannot replace it
	RegistrationID string           `json:"registration_id"` // daemon session identity
	Lease          HubInstanceLease `json:"lease"`           // active or expired lease, retained for fencing
	Sequence       uint64           `json:"sequence"`        // greatest accepted report sequence
	Manifest       InstanceManifest `json:"manifest"`        // last accepted report
}

// hubInstanceState is persisted under the existing bus lock.
type hubInstanceState struct {
	Version   int                           `json:"version"`   // file schema version
	Instances map[string]*hubInstanceRecord `json:"instances"` // records retained after expiry
}

// HubInstanceRegistry owns lease state; all reads and updates use the bus's process-shared lock.
type HubInstanceRegistry struct {
	Bus    *Bus             // trusted local hub state directory
	TTL    time.Duration    // zero uses 180 seconds; nodes cannot extend it
	Now    func() time.Time // optional deterministic clock, installed before serving
	Random io.Reader        // optional secure-entropy fault injection; nil uses crypto/rand.Reader
}

// now is the hub-authoritative lease clock.
func (g *HubInstanceRegistry) now() time.Time {
	if g.Now != nil {
		return g.Now().UTC()
	}
	return time.Now().UTC()
}

// deadline ignores a node's requested TTL and uses only hub policy.
func (g *HubInstanceRegistry) deadline() Timestamp {
	ttl := g.TTL
	if ttl <= 0 {
		ttl = hubInstanceTTL
	}
	return Timestamp{Time: g.now().Add(ttl)}
}

// withState serializes instance operations with all other hub state accesses.
func (g *HubInstanceRegistry) withState(fn func(*hubInstanceState) *StorageError) *StorageError {
	return g.withBusState(func(state *hubInstanceState, _ *State) *StorageError { return fn(state) })
}

// withBusState is withState for operations that also change the teams (the bus state), under the same lock. The
// bus state is saved after fn returns, like every other locked change.
func (g *HubInstanceRegistry) withBusState(fn func(*hubInstanceState, *State) *StorageError) *StorageError {
	if g == nil || g.Bus == nil {
		return storageErr(valueErr("missing hub bus"), "instance registry is not configured")
	}
	var result *StorageError
	if serr := g.Bus.locked(func(bus *State) {
		state, serr := g.load()
		if serr != nil {
			result = serr
			return
		}
		result = fn(state, bus)
	}); serr != nil {
		return serr
	}
	return result
}

// load treats an absent instances.json as an empty registry for existing installations.
func (g *HubInstanceRegistry) load() (*hubInstanceState, *StorageError) {
	data, err := os.ReadFile(g.Bus.path("instances.json"))
	if errors.Is(err, os.ErrNotExist) {
		return &hubInstanceState{Version: hubProtocolVersion, Instances: make(map[string]*hubInstanceRecord)}, nil
	}
	if err != nil {
		return nil, storageErr(err, "cannot read instance registry")
	}
	var state hubInstanceState
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, storageErr(err, "cannot decode instance registry")
	}
	if state.Version != hubProtocolVersion || state.Instances == nil {
		return nil, storageErr(valueErr("invalid instance registry version or entries"), "cannot validate instance registry")
	}
	for name, record := range state.Instances {
		if !validInstanceName(name) || record == nil || record.Peer == "" || record.RegistrationID == "" || record.Lease.Generation <= 0 || record.Lease.Token == "" || record.Lease.ExpiresAt.IsZero() {
			return nil, storageErr(valueErr("invalid instance record %q", name), "cannot validate instance registry")
		}
	}
	return &state, nil
}

// save durably replaces the new registry file before delivering lease credentials.
func (g *HubInstanceRegistry) save(state *hubInstanceState) *StorageError {
	data, err := json.Marshal(state)
	if err != nil {
		return storageErr(err, "cannot encode instance registry")
	}
	f, err := os.CreateTemp(g.Bus.dir, "instances-*.tmp")
	if err != nil {
		return storageErr(err, "cannot create instance registry temporary file")
	}
	defer func() {
		if err := os.Remove(f.Name()); err != nil && !errors.Is(err, os.ErrNotExist) {
			warnSideEffect("remove instance registry temporary file", storageErr(err, "cannot remove temporary registry"))
		}
	}()
	if _, err := f.Write(data); err != nil {
		closeHubFile(f)
		return storageErr(err, "cannot write instance registry")
	}
	if err := f.Sync(); err != nil {
		closeHubFile(f)
		return storageErr(err, "cannot sync instance registry")
	}
	if err := f.Close(); err != nil {
		return storageErr(err, "cannot close instance registry")
	}
	if err := os.Rename(f.Name(), filepath.Join(g.Bus.dir, "instances.json")); err != nil {
		return storageErr(err, "cannot replace instance registry")
	}
	return syncHubDirectory(g.Bus.dir)
}

// validInstanceName enforces the approved qualified-name component grammar.
func validInstanceName(name string) bool {
	if name == "" || len(name) > 63 || name[0] < 'a' || name[0] > 'z' {
		return false
	}
	for _, c := range name {
		if (c < 'a' || c > 'z') && (c < '0' || c > '9') && c != '-' {
			return false
		}
	}
	return true
}

// check compares credentials only after validating the authenticated peer and hub deadline.
func (g *HubInstanceRegistry) check(record *hubInstanceRecord, peer, token string, generation int64) HubLeaseCheck {
	if record == nil {
		return HubLeaseUnknown
	}
	if record.Peer != peer || peer == "" {
		return HubLeaseWrongPeer
	}
	if record.Lease.Generation != generation || subtle.ConstantTimeCompare([]byte(record.Lease.Token), []byte(token)) != 1 {
		return HubLeaseStale
	}
	if !g.now().Before(record.Lease.ExpiresAt.Time) {
		return HubLeaseExpired
	}
	return HubLeaseValid
}

// CheckLease validates a request without exposing credentials or swallowing storage failures.
func (g *HubInstanceRegistry) CheckLease(instance, peer, token string, generation int64) (HubLeaseCheck, *StorageError) {
	result := HubLeaseUnknown
	serr := g.withState(func(state *hubInstanceState) *StorageError {
		result = g.check(state.Instances[instance], peer, token, generation)
		return nil
	})
	return result, serr
}

// CheckReachable reports whether the hub currently holds an unexpired registration.
func (g *HubInstanceRegistry) CheckReachable(instance string) (bool, *StorageError) {
	reachable := false
	serr := g.withState(func(state *hubInstanceState) *StorageError {
		record := state.Instances[instance]
		reachable = record != nil && g.now().Before(record.Lease.ExpiresAt.Time)
		return nil
	})
	return reachable, serr
}

// Register rejects concurrent daemons and makes same-session retries idempotent.
func (g *HubInstanceRegistry) Register(instance, peer string, req HubInstanceRegisterRequest) (HubInstanceLease, *InstanceRegisterError) {
	var lease HubInstanceLease
	if req.Version != hubProtocolVersion || req.RegistrationID == "" || len(req.RegistrationID) > 128 || req.Sequence == 0 || validateHubManifest(req.Manifest) != nil {
		return lease, opErr(InstanceRegisterMalformed, "invalid registration version, session, sequence, or manifest")
	}
	if instance != req.Manifest.InstanceName || peer == "" {
		return lease, opErr(InstanceRegisterIdentity, "manifest must match the authenticated instance and peer")
	}
	var result *InstanceRegisterError
	serr := g.withBusState(func(state *hubInstanceState, bus *State) *StorageError {
		prior := state.Instances[instance]
		if prior != nil && prior.Peer != peer {
			result = opErr(InstanceRegisterIdentity, "instance name is owned by another authenticated peer")
			return nil
		}
		if prior != nil && g.now().Before(prior.Lease.ExpiresAt.Time) {
			if prior.RegistrationID != req.RegistrationID {
				result = opErr(InstanceRegisterDuplicateInstanceName, "instance %q already has a live daemon lease", instance)
				return nil
			}
			lease = prior.Lease
			return nil // Replaying registration never rolls back a newer heartbeat manifest.
		}
		entropy := g.Random
		if entropy == nil {
			entropy = rand.Reader
		}
		var secret [32]byte
		if _, err := io.ReadFull(entropy, secret[:]); err != nil {
			result = opErr(InstanceRegisterEntropy, "cannot generate secure lease credential: %v", err)
			return nil
		}
		generation := int64(1)
		if prior != nil {
			generation = prior.Lease.Generation + 1
		}
		lease = HubInstanceLease{Version: hubProtocolVersion, Token: hex.EncodeToString(secret[:]), Generation: generation, ExpiresAt: g.deadline()}
		state.Instances[instance] = &hubInstanceRecord{Peer: peer, RegistrationID: req.RegistrationID, Lease: lease, Sequence: req.Sequence, Manifest: req.Manifest}
		if serr := g.save(state); serr != nil {
			return serr
		}
		restoreReturnedMembers(g.Bus, bus, instance) // a fresh lease: its members can resume
		return nil
	})
	if serr != nil {
		return HubInstanceLease{}, fromStorage(InstanceRegisterStorage, serr)
	}
	return lease, result
}

// Heartbeat renews only the current generation and refuses out-of-order report replacement.
func (g *HubInstanceRegistry) Heartbeat(instance, peer string, req HubInstanceHeartbeatRequest) (HubInstanceLease, *InstanceHeartbeatError) {
	var lease HubInstanceLease
	if req.Version != hubProtocolVersion || req.Sequence == 0 || req.Generation <= 0 || req.Token == "" || validateHubManifest(req.Manifest) != nil {
		return lease, opErr(InstanceHeartbeatMalformed, "invalid heartbeat version, sequence, credentials, or manifest")
	}
	if instance != req.Instance || instance != req.Manifest.InstanceName || peer == "" {
		return lease, opErr(InstanceHeartbeatIdentity, "heartbeat must match the authenticated instance and peer")
	}
	var result *InstanceHeartbeatError
	serr := g.withBusState(func(state *hubInstanceState, bus *State) *StorageError {
		record := state.Instances[instance]
		switch g.check(record, peer, req.Token, req.Generation) {
		case HubLeaseUnknown:
			result = opErr(InstanceHeartbeatUnknownInstance, "register the instance before sending heartbeats")
		case HubLeaseExpired:
			result = opErr(InstanceHeartbeatLeaseExpired, "lease expired; register again before renewing")
		case HubLeaseStale:
			result = opErr(InstanceHeartbeatStaleLease, "lease token or generation is obsolete")
		case HubLeaseWrongPeer:
			result = opErr(InstanceHeartbeatIdentity, "lease belongs to a different authenticated peer")
		case HubLeaseValid:
			if req.Sequence <= record.Sequence {
				result = opErr(InstanceHeartbeatStaleSequence, "heartbeat sequence must advance beyond %d", record.Sequence)
				return nil
			}
			record.Sequence, record.Manifest = req.Sequence, req.Manifest
			record.Lease.ExpiresAt = g.deadline()
			lease = record.Lease
			if serr := g.save(state); serr != nil {
				return serr
			}
			restoreReturnedMembers(g.Bus, bus, instance) // normally a no-op: a live lease has no unreachable members
			return nil
		}
		return nil
	})
	if serr != nil {
		return HubInstanceLease{}, fromStorage(InstanceHeartbeatStorage, serr)
	}
	return lease, result
}

// serveInstances dispatches only the two typed registration operations, after transport authentication.
func (h *Hub) serveInstances(w http.ResponseWriter, r *http.Request, instance string) {
	register := r.URL.Path == "/v1/instances/register"
	if !register && r.URL.Path != "/v1/instances/heartbeat" {
		http.NotFound(w, r)
		return
	}
	r.Body = http.MaxBytesReader(w, r.Body, hubBodyLimit)
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields()
	peer := hubAuthenticatedPeer(r)
	var lease HubInstanceLease
	var failure *HubWireError
	status := http.StatusBadRequest
	var malformed bool
	if register {
		var req HubInstanceRegisterRequest
		if err := dec.Decode(&req); err != nil {
			malformed = true
		} else {
			var extra any
			malformed = !errors.Is(dec.Decode(&extra), io.EOF)
		}
		if r.Method != http.MethodPost || malformed {
			failure = hubFailure(opErr(InstanceRegisterMalformed, "use POST with one registration JSON object"))
		} else {
			var err *InstanceRegisterError
			lease, err = h.Instances.Register(instance, peer, req)
			if err != nil {
				failure = hubFailure(err)
				status = hubInstanceStatus(err.Class())
			}
		}
	} else {
		var req HubInstanceHeartbeatRequest
		if err := dec.Decode(&req); err != nil {
			malformed = true
		} else {
			var extra any
			malformed = !errors.Is(dec.Decode(&extra), io.EOF)
		}
		if r.Method != http.MethodPost || malformed {
			failure = hubFailure(opErr(InstanceHeartbeatMalformed, "use POST with one heartbeat JSON object"))
		} else {
			var err *InstanceHeartbeatError
			lease, err = h.Instances.Heartbeat(instance, peer, req)
			if err != nil {
				failure = hubFailure(err)
				status = hubInstanceStatus(err.Class())
			}
		}
	}
	if failure != nil {
		h.writeResponse(w, status, HubResponse{Version: hubProtocolVersion, Error: failure})
		return
	}
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(lease); err != nil {
		warnSideEffect("deliver instance lease", storageErr(err, "cannot write instance lease response"))
	}
}

// isInstanceRoute selects registration routes without accepting arbitrary RPC paths.
func isInstanceRoute(path string) bool { return strings.HasPrefix(path, "/v1/instances/") }

// validateHubManifest rejects malformed capacity and unrecognized declarations before persistence.
func validateHubManifest(m InstanceManifest) *ValueError {
	if !validInstanceName(m.InstanceName) || m.Timestamp.IsZero() || m.TTLSeconds <= 0 {
		return valueErr("invalid instance name, manifest timestamp, or TTL")
	}
	if m.Detected.OS != "linux" && m.Detected.OS != "darwin" {
		return valueErr("unsupported detected OS")
	}
	if m.Detected.Arch != "amd64" && m.Detected.Arch != "arm64" {
		return valueErr("unsupported detected architecture")
	}
	if m.CurrentState.Slots.Total < 0 || m.CurrentState.Slots.Free < 0 || m.CurrentState.Slots.Free > m.CurrentState.Slots.Total {
		return valueErr("invalid free or total capacity")
	}
	for _, capability := range m.Declared {
		if !capability.Valid() {
			return valueErr("unknown capability %q", capability)
		}
	}
	for _, agent := range m.CurrentState.Agents {
		if agent.Name == "" || strings.Contains(agent.Name, "@") || !agent.Tier.Valid() {
			return valueErr("invalid manifest agent identity or tier")
		}
		if agent.Role != string(RoleAgent) && agent.Role != string(RoleInvestigator) {
			return valueErr("invalid manifest agent role")
		}
		if agent.Status != "idle" && agent.Status != "busy" && agent.Status != "sleeping" {
			return valueErr("invalid manifest agent state")
		}
	}
	for id, task := range m.CurrentState.Tasks {
		if id == "" || id != task.TaskID || task.Agent == "" {
			return valueErr("invalid task summary identity")
		}
	}
	return nil
}

// hubInstanceStatus maps operation error classes without interpreting serialized cause strings.
func hubInstanceStatus(class ErrorClass) int {
	switch class {
	case ClassPermission:
		return http.StatusForbidden
	case ClassConflict, ClassInvalidState:
		return http.StatusConflict
	case ClassNotFound:
		return http.StatusNotFound
	case ClassStorage, ClassProcess:
		return http.StatusServiceUnavailable
	default:
		return http.StatusBadRequest
	}
}
