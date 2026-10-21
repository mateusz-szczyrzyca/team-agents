// Hub execution keeps the bus logic and replay protection on the lead instance.
package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

const (
	// hubProtocolVersion is the version of the private instance protocol.
	hubProtocolVersion = 1
	// hubReplayWindow is the human-approved maximum age of a replayable request.
	hubReplayWindow = 24 * time.Hour
	// hubClockSkew permits bounded differences between instance clocks.
	hubClockSkew = time.Minute
	// hubPollDuration releases an HTTP connection without extending the underlying call.
	hubPollDuration = 25 * time.Second
	// hubBodyLimit bounds JSON allocations before dispatch.
	hubBodyLimit = 1 << 20
	// hubMaxPending bounds concurrent executing operations, including long polls.
	hubMaxPending = 128
)

// HubCallKind distinguishes MCP tools from allowlisted bus CLI operations.
type HubCallKind string

const (
	HubToolCall HubCallKind = "tool" // an existing MCP handler
	HubCLICall  HubCallKind = "cli"  // an allowlisted shared-state CLI handler
)

// Valid reports whether k is a supported client kind.
func (k HubCallKind) Valid() bool { return k == HubToolCall || k == HubCLICall }

// UnmarshalJSON implements json.Unmarshaler; failures contain *ValueError.
func (k *HubCallKind) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, k, "hub call kind").asError()
}

// HubCaller identifies a run; the hub resolver validates it against registered runs.
type HubCaller struct {
	Agent    string `json:"agent"`    // base name, without an instance suffix
	Instance string `json:"instance"` // authenticated instance name
	Task     string `json:"task"`     // owning task identifier
	Team     string `json:"team"`     // empty for solo work
	Tier     Tier   `json:"tier"`     // registered model tier
	Role     Role   `json:"role"`     // registered agent role, distinct from team coordinator
}

// HubRequest is the retry-stable input to one bus operation.
type HubRequest struct {
	Version   int            `json:"version"`        // private protocol version
	RequestID string         `json:"request_id"`     // random ID, fixed for all retries
	IssuedAt  Timestamp      `json:"issued_at"`      // original timestamp, never refreshed on retry
	Caller    HubCaller      `json:"caller"`         // identity to verify, never an authorization grant
	Kind      HubCallKind    `json:"kind"`           // tool or cli
	Op        string         `json:"op"`             // route operation name
	Tool      string         `json:"tool,omitempty"` // same as Op for MCP calls only
	Args      map[string]any `json:"args"`           // named arguments, never executable text
}

// MarshalJSON implements json.Marshaler; its error is the JSON encoder's concrete error.
// Explicit UTC preserves an instant across instances with different time zones without changing Timestamp's legacy format.
func (r HubRequest) MarshalJSON() ([]byte, error) {
	type requestData HubRequest
	return json.Marshal(struct {
		requestData        // all fields except the overridden timestamp
		IssuedAt    string `json:"issued_at"` // RFC 3339 in UTC rather than legacy local wall time
	}{requestData: requestData(r), IssuedAt: hubTimeString(r.IssuedAt)})
}

// HubWireError is the lossless public portion of a typed operation error.
type HubWireError struct {
	Op      string `json:"op"`      // generated operation name
	Cause   string `json:"cause"`   // generated cause name, never enum ordinal
	Message string `json:"message"` // message without the operation prefix
}

// HubPending signals reattachment to the original operation with its original deadline.
type HubPending struct {
	PollAfterMS int `json:"poll_after_ms"` // optional delay before reattachment, in milliseconds
}

// HubResponse contains exactly one of Result, Error, Pending, or Stdout.
type HubResponse struct {
	Version     int           `json:"version"`                // private protocol version
	RequestID   string        `json:"request_id"`             // original request identifier
	Result      *string       `json:"result,omitempty"`       // exact MCP handler result, including empty strings
	Error       *HubWireError `json:"error,omitempty"`        // exact typed failure
	Pending     *HubPending   `json:"pending,omitempty"`      // HTTP poll finished, operation still running
	UnreadCount int           `json:"unread_count,omitempty"` // hub-computed MCP footer count
	Stdout      *[]byte       `json:"stdout,omitempty"`       // losslessly encoded CLI output, including empty output
}

// hubReplayRecord persists acceptance before execution and the response before delivery.
type hubReplayRecord struct {
	Version  int          `json:"version"`            // replay file version
	Digest   string       `json:"digest"`             // canonical request hash, detects changed payloads
	IssuedAt Timestamp    `json:"issued_at"`          // original age used for retention
	Response *HubResponse `json:"response,omitempty"` // nil means accepted with an unknown outcome after restart
	Expired  bool         `json:"expired,omitempty"`  // completed payload reclaimed; its digest still fences request-ID reuse
}

// MarshalJSON implements json.Marshaler; its error is the JSON encoder's concrete error.
func (r hubReplayRecord) MarshalJSON() ([]byte, error) {
	type recordData hubReplayRecord
	return json.Marshal(struct {
		recordData        // replay fields except the overridden timestamp
		IssuedAt   string `json:"issued_at"` // stable instant if a backup is restored in another time zone
	}{recordData: recordData(r), IssuedAt: hubTimeString(r.IssuedAt)})
}

// hubTimeString encodes a new-protocol instant without changing any existing persisted Timestamp field.
func hubTimeString(t Timestamp) string {
	if t.IsZero() {
		return ""
	}
	return t.UTC().Format(time.RFC3339Nano)
}

// hubFlight is a running operation shared by concurrent HTTP reattachments.
type hubFlight struct {
	digest   string        // immutable validated payload hash
	done     chan struct{} // closed only after response is finalized
	response HubResponse   // published by closing done
}

// Hub executes validated requests on one local bus; callbacks are installed before serving.
type Hub struct {
	Bus          *Bus                                        // the hub's own state directory, never selected by a request
	Authenticate func(*http.Request) (string, *HubAuthError) // verifies each HTTP request; nil denies access
	Resolve      func(HubCaller) (*Server, *HubCallError)    // checks registered run identity and returns its trusted server template
	CLI          func(HubRequest) HubResponse                // allowlisted CLI dispatcher; nil rejects CLI requests
	PollDuration time.Duration                               // zero uses the 25-second production poll interval
	mu           sync.Mutex                                  // protects active operations and acceptance, never held across handler execution
	active       map[string]*hubFlight                       // in-process operations; durable records survive a restart
}

// hubFailure converts an operation's concrete error without parsing human-readable text.
func hubFailure[C opCause](err *OpError[C]) *HubWireError {
	return &HubWireError{Op: err.Cause.op(), Cause: err.CauseName(), Message: err.Message}
}

// hubErrorResponse builds an admission error; execution errors retain their own operation names.
func hubErrorResponse(id string, err *HubCallError) HubResponse {
	return HubResponse{Version: hubProtocolVersion, RequestID: id, Error: hubFailure(err)}
}

// ServeHTTP authenticates before decoding and exposes only operation-specific POST routes.
func (h *Hub) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if h.Authenticate == nil {
		h.writeResponse(w, http.StatusForbidden, hubErrorResponse("", opErr(HubCallIdentity, "hub authentication is not configured")))
		return
	}
	instance, authErr := h.Authenticate(r)
	if authErr != nil {
		status := http.StatusForbidden
		switch authErr.Class() {
		case ClassConflict:
			status = http.StatusConflict
		case ClassProcess, ClassStorage, ClassUnavailable:
			status = http.StatusServiceUnavailable
		case ClassInvalidArgument:
			status = http.StatusBadRequest
		case ClassPermission:
			if authErr.Cause == HubAuthMissingCredentials || authErr.Cause == HubAuthWrongToken {
				status = http.StatusUnauthorized
			}
		}
		h.writeResponse(w, status, HubResponse{Version: hubProtocolVersion, Error: hubFailure(authErr)})
		return
	}
	if r.Method != http.MethodPost || !strings.HasPrefix(r.URL.Path, "/v1/tools/") {
		h.writeResponse(w, http.StatusBadRequest, hubErrorResponse("", opErr(HubCallMalformed, "use POST /v1/tools/{op}")))
		return
	}
	var req HubRequest
	r.Body = http.MaxBytesReader(w, r.Body, hubBodyLimit)
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields()
	if err := dec.Decode(&req); err != nil {
		status := http.StatusBadRequest
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			status = http.StatusRequestEntityTooLarge
		}
		h.writeResponse(w, status, hubErrorResponse("", opErr(HubCallMalformed, "invalid hub request: %v", err)))
		return
	}
	var extra any
	if err := dec.Decode(&extra); !errors.Is(err, io.EOF) {
		status := http.StatusBadRequest
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			status = http.StatusRequestEntityTooLarge
		}
		h.writeResponse(w, status, hubErrorResponse(req.RequestID, opErr(HubCallMalformed, "request must contain exactly one JSON object")))
		return
	}
	if req.Op != strings.TrimPrefix(r.URL.Path, "/v1/tools/") {
		h.writeResponse(w, http.StatusBadRequest, hubErrorResponse(req.RequestID, opErr(HubCallMalformed, "route and operation differ")))
		return
	}
	response, status := h.Execute(r.Context(), instance, req)
	h.writeResponse(w, status, response)
}

// writeResponse reports delivery failures as side effects; durable responses remain replayable.
func (h *Hub) writeResponse(w http.ResponseWriter, status int, response HubResponse) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(response); err != nil {
		warnSideEffect("deliver hub response", storageErr(err, "cannot encode or write hub response"))
	}
}

// validate checks the envelope and age before touching the replay ledger.
func (h *Hub) validate(instance string, req HubRequest) *HubCallError {
	if req.Version != hubProtocolVersion || req.RequestID == "" || len(req.RequestID) > 128 || req.Op == "" ||
		!req.Kind.Valid() || (req.Kind == HubToolCall && req.Tool != req.Op) || (req.Kind == HubCLICall && req.Tool != "") {
		return opErr(HubCallMalformed, "invalid version, request ID, kind, or operation")
	}
	if instance == "" || req.Caller.Instance != instance || req.Caller.Agent == "" || strings.Contains(req.Caller.Agent, "@") ||
		req.Caller.Task == "" || !req.Caller.Role.Valid() || !req.Caller.Tier.Valid() || h.Resolve == nil {
		return opErr(HubCallIdentity, "caller must match an authenticated registered run")
	}
	now := time.Now()
	if req.IssuedAt.IsZero() || req.IssuedAt.After(now.Add(hubClockSkew)) || !req.IssuedAt.After(now.Add(-hubReplayWindow)) {
		return opErr(HubCallExpired, "request is outside the 24-hour replay window or clock-skew allowance")
	}
	if h.Bus == nil {
		return opErr(HubCallStorage, "hub bus is not configured")
	}
	if req.Kind == HubToolCall {
		if _, known := toolSchemas()[req.Op]; !known {
			return opErr(HubCallUnknownOperation, "unknown tool operation %q", req.Op)
		}
	} else if h.CLI == nil {
		return opErr(HubCallUnknownOperation, "CLI operations are not configured")
	}
	return nil
}

// Execute is also the lead-local entry point; instance comes from trusted local registration or HTTP auth.
func (h *Hub) Execute(ctx context.Context, instance string, req HubRequest) (HubResponse, int) {
	if err := h.validate(instance, req); err != nil {
		return hubErrorResponse(req.RequestID, err), hubAdmissionStatus(err)
	}
	s, err := h.Resolve(req.Caller)
	if err != nil {
		return hubErrorResponse(req.RequestID, err), hubAdmissionStatus(err)
	}
	if s == nil || s.bus != h.Bus || s.me != req.Caller.Agent+"@"+instance || s.info.Task != req.Caller.Task ||
		s.team != req.Caller.Team || s.tier != req.Caller.Tier || s.role != req.Caller.Role {
		return hubErrorResponse(req.RequestID, opErr(HubCallIdentity, "registered run does not match caller")), http.StatusForbidden
	}
	flight, admission := h.accept(req, s)
	if admission != nil {
		return hubErrorResponse(req.RequestID, admission), hubAdmissionStatus(admission)
	}
	poll := h.PollDuration
	if poll <= 0 {
		poll = hubPollDuration
	}
	timer := time.NewTimer(poll)
	defer timer.Stop()
	select {
	case <-flight.done:
		return flight.response, http.StatusOK
	case <-timer.C:
		return HubResponse{Version: hubProtocolVersion, RequestID: req.RequestID, Pending: &HubPending{}}, http.StatusOK
	case <-ctx.Done():
		// Cancellation detaches this HTTP observer, not a potentially committed mutation.
		return HubResponse{Version: hubProtocolVersion, RequestID: req.RequestID, Pending: &HubPending{}}, http.StatusOK
	}
}

// hubAdmissionStatus maps this endpoint's causes to HTTP admission statuses.
func hubAdmissionStatus(err *HubCallError) int {
	switch err.Cause {
	case HubCallMalformed:
		return http.StatusBadRequest
	case HubCallIdentity:
		return http.StatusForbidden
	case HubCallRequestConflict:
		return http.StatusConflict
	case HubCallUnknownOperation:
		return http.StatusNotFound
	case HubCallExpired:
		return http.StatusBadRequest
	case HubCallOverloaded:
		return http.StatusServiceUnavailable
	default:
		return http.StatusServiceUnavailable
	}
}

// accept durably records a request before dispatch, or attaches to its previous outcome.
func (h *Hub) accept(req HubRequest, s *Server) (*hubFlight, *HubCallError) {
	data, jerr := json.Marshal(req)
	if jerr != nil {
		return nil, opErr(HubCallMalformed, "request cannot be encoded: %v", jerr)
	}
	// Lead-local callers retain their argument maps; freeze the exact payload whose digest is accepted.
	var frozen HubRequest
	if err := json.Unmarshal(data, &frozen); err != nil {
		return nil, opErr(HubCallMalformed, "request cannot be decoded: %v", err)
	}
	req = frozen
	digest := fmt.Sprintf("%x", sha256.Sum256(data))
	keyData, jerr := json.Marshal([]string{req.Caller.Instance, req.Caller.Task, req.RequestID})
	if jerr != nil {
		return nil, opErr(HubCallMalformed, "request identity cannot be encoded: %v", jerr)
	}
	key := fmt.Sprintf("%x", sha256.Sum256(keyData))
	h.mu.Lock()
	defer h.mu.Unlock()
	if flight := h.active[key]; flight != nil {
		if flight.digest != digest {
			return nil, opErr(HubCallRequestConflict, "request ID already belongs to another payload")
		}
		return flight, nil
	}
	path := filepath.Join(h.Bus.dir, "replay", key+".json")
	record, serr := loadHubReplay(path)
	if serr != nil {
		return nil, fromStorage(HubCallStorage, serr)
	}
	if record != nil {
		if record.Digest != digest {
			return nil, opErr(HubCallRequestConflict, "request ID already belongs to another payload")
		}
		if record.Expired {
			return nil, opErr(HubCallExpired, "request response has expired; its ID cannot be reused")
		}
		response := record.Response
		if response == nil {
			response = &HubResponse{Version: hubProtocolVersion, RequestID: req.RequestID,
				Error: &HubWireError{Op: req.Op, Cause: HubCallOutcomeUnknown.String(), Message: "hub restarted before recording the result; reconcile this request before submitting new work"}}
		}
		flight := &hubFlight{digest: digest, done: make(chan struct{}), response: *response}
		close(flight.done)
		return flight, nil
	}
	if len(h.active) >= hubMaxPending {
		return nil, opErr(HubCallOverloaded, "hub has too many pending operations; retry this request later")
	}
	record = &hubReplayRecord{Version: hubProtocolVersion, Digest: digest, IssuedAt: req.IssuedAt}
	if serr := saveHubReplay(path, record, true); serr != nil {
		return nil, fromStorage(HubCallStorage, serr)
	}
	flight := &hubFlight{digest: digest, done: make(chan struct{})}
	if h.active == nil {
		h.active = make(map[string]*hubFlight)
	}
	h.active[key] = flight
	go h.run(key, path, req, record, flight, s)
	return flight, nil
}

// run executes one accepted operation and persists its response before releasing observers.
func (h *Hub) run(key, path string, req HubRequest, record *hubReplayRecord, flight *hubFlight, s *Server) {
	response := HubResponse{Version: hubProtocolVersion, RequestID: req.RequestID}
	if req.Kind == HubCLICall {
		if h.CLI == nil {
			response.Error = hubFailure(opErr(HubCallUnknownOperation, "CLI operation is not available"))
		} else {
			response = h.CLI(req)
			response.Version, response.RequestID = hubProtocolVersion, req.RequestID
		}
	} else {
		args := qualifyHubArgs(req.Args, req.Caller.Instance)
		out, callErr := s.call(req.Op, args)
		if callErr != nil {
			response.Error = encodeHubOperationError(callErr)
		} else {
			response.Result = &out
		}
		if req.Op != "read_messages" {
			n, serr := s.unreadCount()
			if serr != nil {
				warnSideEffect("count hub unread messages", serr)
			} else {
				response.UnreadCount = n
			}
		}
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	record.Response = &response
	if serr := saveHubReplay(path, record, false); serr != nil {
		// Effects may have happened; the accepted record remains unresolved on disk.
		warnSideEffect("record hub response", serr)
		response.Result, response.Stdout, response.Pending = nil, nil, nil
		response.Error = &HubWireError{Op: req.Op, Cause: HubCallOutcomeUnknown.String(), Message: "operation ran but its result could not be recorded; reconcile before submitting new work"}
	}
	flight.response = response
	close(flight.done)
	delete(h.active, key)
}

// encodeHubOperationError is the HTTP dispatcher boundary; err contains an operation's *OpError.
func encodeHubOperationError(err error) *HubWireError {
	type wireOperation interface {
		Operation() string
		CauseName() string
		ErrorMessage() string
	}
	var operation wireOperation
	if errors.As(err, &operation) {
		return &HubWireError{Op: operation.Operation(), Cause: operation.CauseName(), Message: operation.ErrorMessage()}
	}
	return hubFailure(opErr(HubCallStorage, "handler returned an error without a wire representation"))
}

// qualifyHubArgs resolves agent-address arguments relative to the caller's instance.
func qualifyHubArgs(args map[string]any, instance string) map[string]any {
	out := make(map[string]any, len(args))
	for name, value := range args {
		if name == "to" || name == "from" || name == "agent" {
			if target, ok := value.(string); ok && target != "" && target != "all" && target != "team" &&
				!strings.HasPrefix(target, "team:") && !strings.Contains(target, "@") {
				value = target + "@" + instance
			}
		}
		out[name] = value
	}
	return out
}

// loadHubReplay reads a trusted hub-owned record; absence means this request is new.
func loadHubReplay(path string) (*hubReplayRecord, *StorageError) {
	data, err := os.ReadFile(path) //nolint:gosec // path is derived only from the configured bus directory and a SHA-256 filename
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, storageErr(err, "cannot read hub replay record")
	}
	var record hubReplayRecord
	if err := json.Unmarshal(data, &record); err != nil {
		return nil, storageErr(err, "cannot decode hub replay record")
	}
	if record.Version != hubProtocolVersion || len(record.Digest) != hex.EncodedLen(sha256.Size) || record.IssuedAt.IsZero() {
		return nil, storageErr(valueErr("invalid replay record"), "cannot validate hub replay record")
	}
	return &record, nil
}

// PruneReplay reclaims completed response payloads after 24 hours, retaining ID tombstones and unresolved work.
// It is safe to call periodically; the same acceptance mutex protects concurrent completion and pruning.
func (h *Hub) PruneReplay() *HubCallError {
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.Bus == nil {
		return opErr(HubCallStorage, "hub bus is not configured")
	}
	dir := filepath.Join(h.Bus.dir, "replay")
	entries, err := os.ReadDir(dir)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return fromStorage(HubCallStorage, storageErr(err, "cannot list hub replay records"))
	}
	for _, entry := range entries {
		name := entry.Name()
		if entry.IsDir() || len(name) != hex.EncodedLen(sha256.Size)+len(".json") || !strings.HasSuffix(name, ".json") {
			continue
		}
		if _, err := hex.DecodeString(strings.TrimSuffix(name, ".json")); err != nil {
			continue // Other files do not belong to the replay ledger.
		}
		path := filepath.Join(dir, name)
		record, serr := loadHubReplay(path)
		if serr != nil {
			return fromStorage(HubCallStorage, serr)
		}
		if record == nil || record.Response == nil || time.Since(record.IssuedAt.Time) < hubReplayWindow {
			continue
		}
		record.Response, record.Expired = nil, true
		if serr := saveHubReplay(path, record, false); serr != nil {
			return fromStorage(HubCallStorage, serr)
		}
	}
	return nil
}

// saveHubReplay syncs acceptance exclusively, or atomically replaces it with a completed response.
func saveHubReplay(path string, record *hubReplayRecord, exclusive bool) *StorageError {
	data, err := json.Marshal(record)
	if err != nil {
		return storageErr(err, "cannot encode hub replay record")
	}
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return storageErr(err, "cannot create hub replay directory")
	}
	if serr := syncHubDirectory(filepath.Dir(dir)); serr != nil {
		return serr
	}
	flags := os.O_WRONLY | os.O_CREATE | os.O_EXCL
	target := path
	if !exclusive {
		target += ".tmp"
	}
	f, err := os.OpenFile(target, flags, 0o600) //nolint:gosec // filename is a request hash under the configured private bus directory
	if err != nil {
		return storageErr(err, "cannot create hub replay record")
	}
	if _, err := f.Write(data); err != nil {
		closeHubFile(f)
		return storageErr(err, "cannot write hub replay record")
	}
	if err := f.Sync(); err != nil {
		closeHubFile(f)
		return storageErr(err, "cannot sync hub replay record")
	}
	if err := f.Close(); err != nil {
		return storageErr(err, "cannot close hub replay record")
	}
	if !exclusive {
		if err := os.Rename(target, path); err != nil {
			return storageErr(err, "cannot replace hub replay record")
		}
	}
	return syncHubDirectory(dir)
}

// syncHubDirectory persists new entries and replacements in a private hub-owned directory.
func syncHubDirectory(dir string) *StorageError {
	parent, err := os.Open(dir) //nolint:gosec // directory is derived from the operator's configured bus directory
	if err != nil {
		return storageErr(err, "cannot open hub replay directory")
	}
	if err := parent.Sync(); err != nil {
		closeHubFile(parent)
		return storageErr(err, "cannot sync hub replay directory")
	}
	if err := parent.Close(); err != nil {
		return storageErr(err, "cannot close hub replay directory")
	}
	return nil
}

// closeHubFile logs a cleanup failure after the main storage error has already been selected.
func closeHubFile(f *os.File) {
	if err := f.Close(); err != nil {
		warnSideEffect("close hub replay file", storageErr(err, "cannot close hub replay file"))
	}
}
