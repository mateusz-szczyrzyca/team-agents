// Hub views expose read-only, credential-free summaries to authenticated instance operators.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"sort"
	"strings"
	"time"
)

// hubViewRequest selects the read-only overview; it never supplies a run identity or a filesystem path.
type hubViewRequest struct {
	Version  int    `json:"version"`  // control-plane protocol version
	Instance string `json:"instance"` // must match transport authentication
}

// hubViewResponse carries one overview or an operation-specific error.
type hubViewResponse struct {
	Version int              `json:"version"`         // control-plane protocol version
	View    *HubViewSnapshot `json:"view,omitempty"`  // coherent overview, absent on failure
	Error   *HubWireError    `json:"error,omitempty"` // exact endpoint or authentication error
}

// HubViewSnapshot contains display data only; lease tokens, daemon sessions and executable paths are omitted.
type HubViewSnapshot struct {
	Version        int                       `json:"version"`        // schema version shared with the endpoint
	At             string                    `json:"at"`             // hub snapshot instant in UTC RFC3339
	Instances      []HubViewInstance         `json:"instances"`      // sorted registered nodes, including expired leases
	Agents         map[string]*Presence      `json:"agents"`         // live MCP stage summaries
	Teams          map[string]HubViewTeam    `json:"teams"`          // public team state, without private quiz answers
	Investigations map[string]*Investigation `json:"investigations"` // queue and investigation summaries
	Messages       []Message                 `json:"messages"`       // most recent 300 bus messages
}

// HubViewInstance is a manifest projection with no session or lease credentials.
type HubViewInstance struct {
	Name         string                    `json:"name"`         // unique instance name
	Active       bool                      `json:"active"`       // lease is current at snapshot time
	ExpiresAt    string                    `json:"expires_at"`   // hub-authoritative expiry instant
	OS           string                    `json:"os"`           // detected operating system
	Arch         string                    `json:"arch"`         // detected architecture
	Capabilities []Capability              `json:"capabilities"` // detected and declared vocabulary
	Slots        SlotCapacity              `json:"slots"`        // last accepted capacity report
	Tasks        map[string]HubTaskSummary `json:"tasks"`        // node-owned task metadata, never local paths to open
}

// HubViewTeam preserves board/member views without exposing WakeCmd, Home or private qualification answers.
type HubViewTeam struct {
	ID      string                 `json:"id"`      // team identifier
	Lead    string                 `json:"lead"`    // coordinator agent, possibly qualified
	Done    bool                   `json:"done"`    // team has finished
	Members map[string]*TeamMember `json:"members"` // team-member display state
	Board   map[string]*BoardItem  `json:"board"`   // task board
}

// viewSnapshot reads the registry, team state and message tail under the existing local bus lock.
func (h *Hub) viewSnapshot() (*HubViewSnapshot, *HubViewError) {
	if h.Bus == nil || h.Instances == nil || h.Instances.Bus != h.Bus {
		return nil, opErr(HubViewStorage, "hub view requires its own bus and instance registry")
	}
	view := &HubViewSnapshot{Version: hubProtocolVersion, At: time.Now().UTC().Format(time.RFC3339Nano), Teams: map[string]HubViewTeam{}}
	var failed *HubViewError
	serr := h.Bus.locked(func(st *State) {
		registry, serr := h.Instances.load()
		if serr != nil {
			failed = fromStorage(HubViewStorage, serr)
			return
		}
		now := h.Instances.now()
		view.At = now.Format(time.RFC3339Nano)
		for name, record := range registry.Instances {
			manifest := record.Manifest
			provided := manifestView(name, manifest).Provided
			tasks := make(map[string]HubTaskSummary, len(manifest.CurrentState.Tasks))
			for id, task := range manifest.CurrentState.Tasks {
				task.Worktree = ""
				tasks[id] = task
			}
			view.Instances = append(view.Instances, HubViewInstance{Name: name, Active: now.Before(record.Lease.ExpiresAt.Time),
				ExpiresAt: hubTimeString(record.Lease.ExpiresAt), OS: manifest.Detected.OS, Arch: manifest.Detected.Arch,
				Capabilities: provided, Slots: manifest.CurrentState.Slots, Tasks: tasks})
		}
		view.Agents = make(map[string]*Presence, len(st.Agents))
		for name, presence := range st.Agents {
			public := *presence
			public.Worktree = ""
			view.Agents[name] = &public
		}
		view.Investigations = make(map[string]*Investigation, len(st.Investigations))
		for id, investigation := range st.Investigations {
			public := *investigation
			public.Worktree = ""
			public.File = "" // A record on the hub is never a local file on the viewing instance.
			view.Investigations[id] = &public
		}
		for id, team := range st.Teams {
			members := make(map[string]*TeamMember, len(team.Members))
			for name, member := range team.Members {
				public := *member
				public.Worktree = ""
				members[name] = &public
			}
			view.Teams[id] = HubViewTeam{ID: id, Lead: team.Lead, Done: team.Done, Members: members, Board: team.Board}
		}
		messages, serr := h.Bus.messages()
		if serr != nil {
			failed = fromStorage(HubViewStorage, serr)
			return
		}
		if len(messages) > 300 {
			messages = messages[len(messages)-300:]
		}
		view.Messages = messages
	})
	if serr != nil {
		return nil, fromStorage(HubViewStorage, serr)
	}
	if failed != nil {
		return nil, failed
	}
	sort.Slice(view.Instances, func(i, j int) bool { return view.Instances[i].Name < view.Instances[j].Name })
	return view, nil
}

// serveView validates the authenticated read-only request before reading the hub's own state.
func (h *Hub) serveView(w http.ResponseWriter, r *http.Request, instance string) {
	r.Body = http.MaxBytesReader(w, r.Body, hubBodyLimit)
	response := hubViewResponse{Version: hubProtocolVersion}
	status := http.StatusOK
	var request hubViewRequest
	if r.Method != http.MethodPost {
		response.Error = hubFailure(opErr(HubViewMalformed, "use POST /v1/view"))
		status = http.StatusBadRequest
	} else if verr := decodeCommandBody(r, &request); verr != nil || request.Version != hubProtocolVersion {
		response.Error = hubFailure(opErr(HubViewMalformed, "view request must be one versioned JSON object"))
		status = http.StatusBadRequest
	} else if request.Instance != instance {
		response.Error = hubFailure(opErr(HubViewIdentity, "view instance must match the authenticated peer"))
		status = http.StatusForbidden
	} else {
		view, err := h.viewSnapshot()
		if err != nil {
			response.Error = hubFailure(err)
			status = http.StatusServiceUnavailable
		} else {
			response.View = view
		}
	}
	data, err := json.Marshal(response)
	if err != nil || len(data) > hubResponseLimit {
		h.writeResponse(w, http.StatusServiceUnavailable, HubResponse{Version: hubProtocolVersion,
			Error: hubFailure(opErr(HubViewStorage, "hub view cannot be encoded within the response limit"))})
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if _, err := w.Write(append(data, '\n')); err != nil {
		warnSideEffect("deliver hub view", storageErr(err, "cannot write view response"))
	}
}

// Snapshot retries read-only transport failures within the configured outage budget and caller deadline.
func (c *HubClient) Snapshot(ctx context.Context) (*HubViewSnapshot, *CmdSnapshotError) {
	if err := c.validate(); err != nil {
		return nil, opErr(CmdSnapshotInvalidConfig, "%s", err.Message)
	}
	budget := c.RetryDuration
	if budget <= 0 {
		budget = 30 * time.Second
	}
	bounded, cancel := context.WithTimeout(ctx, budget)
	defer cancel()
	for {
		view, err := c.snapshotOnce(bounded)
		if err == nil || err.Cause != CmdSnapshotUnavailable || bounded.Err() != nil {
			return view, err
		}
		timer := time.NewTimer(hubRetryInterval)
		select {
		case <-bounded.Done():
			timer.Stop()
			return nil, err
		case <-timer.C:
		}
	}
}

// snapshotOnce preserves wire causes and never falls back to reading local state after a hub failure.
func (c *HubClient) snapshotOnce(ctx context.Context) (*HubViewSnapshot, *CmdSnapshotError) {
	data, err := json.Marshal(hubViewRequest{Version: hubProtocolVersion, Instance: c.Instance})
	if err != nil {
		return nil, opErr(CmdSnapshotProtocol, "cannot encode hub view request: %v", err)
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, strings.TrimRight(c.URL, "/")+"/v1/view", bytes.NewReader(data))
	if err != nil {
		return nil, opErr(CmdSnapshotInvalidConfig, "cannot construct hub view request: %v", err)
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("X-Instance-Name", c.Instance)
	if c.Token != "" {
		request.Header.Set("Authorization", "Bearer "+c.Token)
	}
	transport := http.Client{}
	if c.HTTP != nil {
		transport = *c.HTTP
	}
	// CheckRedirect is an HTTP interface callback; its error is the standard http.ErrUseLastResponse sentinel.
	transport.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	response, err := transport.Do(request)
	if err != nil {
		var certificate *HubCertificateError
		if errors.As(err, &certificate) {
			return nil, opErr(CmdSnapshotInvalidConfig, "hub TLS verification failed: %s", certificate.Value.Message)
		}
		return nil, opErr(CmdSnapshotUnavailable, "hub view is unavailable: %v", err)
	}
	defer func() {
		if err := response.Body.Close(); err != nil {
			warnSideEffect("close hub view response", storageErr(err, "cannot close view response"))
		}
	}()
	body, err := io.ReadAll(io.LimitReader(response.Body, hubResponseLimit+1))
	if err != nil {
		return nil, opErr(CmdSnapshotUnavailable, "hub view response was interrupted: %v", err)
	}
	if len(body) > hubResponseLimit {
		return nil, opErr(CmdSnapshotProtocol, "hub view exceeds the response limit")
	}
	var reply hubViewResponse
	if err := json.Unmarshal(body, &reply); err != nil || reply.Version != hubProtocolVersion {
		if response.StatusCode >= http.StatusInternalServerError {
			return nil, opErr(CmdSnapshotUnavailable, "hub view is temporarily unavailable")
		}
		return nil, opErr(CmdSnapshotProtocol, "hub returned an invalid view response")
	}
	if reply.Error != nil {
		remote, known := wireOperationError(reply.Error.Op, reply.Error.Cause, reply.Error.Message)
		if !known || (reply.Error.Op != "hub_view" && reply.Error.Op != "hub_auth") {
			return nil, opErr(CmdSnapshotProtocol, "hub view error names an unknown operation or cause")
		}
		cause := CmdSnapshotRejected
		if errors.Is(remote, ErrStorage) || errors.Is(remote, ErrUnavailable) || errors.Is(remote, ErrProcess) {
			cause = CmdSnapshotUnavailable
		}
		failed := opErr(cause, "%s", reply.Error.Message)
		failed.Err = errors.Join(failed.Err, remote)
		return nil, failed
	}
	if response.StatusCode != http.StatusOK || reply.View == nil || reply.View.Version != hubProtocolVersion {
		return nil, opErr(CmdSnapshotProtocol, "hub response has no successful view")
	}
	return reply.View, nil
}
