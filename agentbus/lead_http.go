package main

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"time"
)

// leadAPI adapts authenticated operations to the durable registry without touching ordinary bus state.
type leadAPI struct {
	bus          *Bus          // registry storage and shared flock
	ttl          time.Duration // maximum node silence before unstarted assignments expire
	notifyDir    string        // optional directory for completion notification files to human requesters
	notifyCmd    string        // optional delivery command run with the notification file path
	questionsCmd string        // optional command run with _remote_questions when questions arrive
}

// leadHeartbeatBody adds pending reports to the node's current declaration.
type leadHeartbeatBody struct {
	NodeHeartbeat                 // declaration, with identity supplied by the socket peer
	Reports       []JobReport     // latest unacknowledged snapshots, applied atomically
	AnswerAcks    []LeadAnswerAck `json:",omitempty"` // answers acknowledged by the node
	KickAcks      []LeadKickAck   `json:",omitempty"` // kicks acknowledged by the node
}

// MarshalJSON implements json.Marshaler; encoding/json supplies its interface error.
// Pending reports must use precise UTC instants rather than the ordinary bus's legacy local layout.
func (b leadHeartbeatBody) MarshalJSON() ([]byte, error) {
	type wireReport struct {
		JobReport                      // report payload except for shadowed timestamps
		At          jobTime            // precise scalar report time
		Transitions []storedTransition // precise cumulative history
	}
	reports := make([]wireReport, 0, len(b.Reports))
	for _, rep := range b.Reports {
		transitions := make([]storedTransition, 0, len(rep.Transitions))
		for _, tr := range rep.Transitions {
			transitions = append(transitions, storedTransition{State: tr.State, At: jobTime{tr.At}})
		}
		reports = append(reports, wireReport{JobReport: rep, At: jobTime{rep.At}, Transitions: transitions})
	}
	return json.Marshal(struct {
		NodeHeartbeat                 // current node declaration
		Reports       []wireReport    // snapshots with lossless times
		AnswerAcks    []LeadAnswerAck `json:",omitempty"`
		KickAcks      []LeadKickAck   `json:",omitempty"`
	}{b.NodeHeartbeat, reports, b.AnswerAcks, b.KickAcks})
}

// maxHeartbeatResponseBytes is the maximum encoded bytes of a heartbeat response (1 MiB).
const maxHeartbeatResponseBytes = 1 << 20

// leadHeartbeatResponse represents the wire payload returned by POST /v1/heartbeat.
type leadHeartbeatResponse struct {
	Job      *leadJobView  `json:"Job"`      // optional assignment for the node
	Stops    []leadJobView `json:"Stops"`    // bounded stop requests for the node
	Wakes    []string      `json:"Wakes"`    // jobs whose remote team members the node must wake
	WakesNow []string      `json:"WakesNow"` // jobs whose paused remote team members the node must resume now
	Answers  []LeadAnswer  `json:"Answers"`  // queued question answers for the node
	Kicks    []LeadKick    `json:"Kicks"`    // queued kicks for the node
}

// boundedControls holds delivered answers and kicks that fit within the wire budget.
type boundedControls struct {
	Answers []LeadAnswer // delivered answers within the budget
	Kicks   []LeadKick   // delivered kicks within the budget
}

// measureHeartbeatReplyBytes measures the encoded JSON length of a candidate heartbeat response.
func measureHeartbeatReplyBytes(resp leadHeartbeatResponse) (int, *StorageError) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(resp); err != nil {
		return 0, storageErr(err, "cannot encode heartbeat response")
	}
	return buf.Len(), nil
}

// boundHeartbeatControls selects queued answers and kicks that fit within the 1 MiB wire limit.
func boundHeartbeatControls(view *leadJobView, stops []leadJobView, wakes, wakesNow []string, answers []LeadAnswer, kicks []LeadKick) (boundedControls, *StorageError) {
	currentResp := leadHeartbeatResponse{
		Job:      view,
		Stops:    stops,
		Wakes:    wakes,
		WakesNow: wakesNow,
	}
	var deliveredAnswers []LeadAnswer
	for _, ans := range answers {
		if len(deliveredAnswers) >= MaxHeartbeatAnswers {
			break
		}
		candidate := currentResp
		candidate.Answers = append(candidate.Answers, ans)
		size, err := measureHeartbeatReplyBytes(candidate)
		if err != nil {
			return boundedControls{}, err
		}
		if size <= maxHeartbeatResponseBytes {
			currentResp = candidate
			deliveredAnswers = append(deliveredAnswers, ans)
		}
	}
	var deliveredKicks []LeadKick
	for _, kick := range kicks {
		if len(deliveredKicks) >= MaxHeartbeatKicks {
			break
		}
		candidate := currentResp
		candidate.Kicks = append(candidate.Kicks, kick)
		size, err := measureHeartbeatReplyBytes(candidate)
		if err != nil {
			return boundedControls{}, err
		}
		if size <= maxHeartbeatResponseBytes {
			currentResp = candidate
			deliveredKicks = append(deliveredKicks, kick)
		}
	}
	return boundedControls{
		Answers: deliveredAnswers,
		Kicks:   deliveredKicks,
	}, nil
}

// leadJobView exposes job data without registry counters or bus internals.
type leadJobView struct {
	Request       JobRequest         // submitted data
	State         JobState           // current state
	Node          string             // assigned instance
	Assignment    uint64             // launch fencing evidence
	History       []storedTransition // lossless transition times
	TaskID        string             // node's local task
	Agent         string             `json:",omitempty"` // optional actual chosen agent on the node
	Branch        string             // result branch
	PRURL         string             // optional pull request
	Result        string             // result head or failure reason
	StopRequested bool               // cancellation awaits the node
	WaitReason    string             // why a queued job cannot currently run
	NodeOffline   bool               // assigned instance exceeded its TTL
}

// leadNodeView exposes only a node's declaration, receipt time and liveness.
type leadNodeView struct {
	NodeHeartbeat         // last accepted declaration
	LastSeen      jobTime // precise heartbeat receipt time
	Offline       bool    // silence reached the configured TTL
}

// newLeadHandler constructs the HTTP boundary shared by configured listeners.
func newLeadHandler(bus *Bus, localNode string, allowed []string, ttl time.Duration, notify ...string) http.Handler {
	var notifyDir, notifyCmd, questionsCmd string
	if len(notify) > 0 {
		notifyDir = notify[0]
	}
	if len(notify) > 1 {
		notifyCmd = notify[1]
	}
	if len(notify) > 2 {
		questionsCmd = notify[2]
	}
	a := &leadAPI{bus: bus, ttl: ttl, notifyDir: notifyDir, notifyCmd: notifyCmd, questionsCmd: questionsCmd}
	mux := http.NewServeMux()
	mux.HandleFunc("POST /v1/sync", func(w http.ResponseWriter, r *http.Request) {
		serveLead(w, r, localNode, allowed, LeadSyncUnauthorized, func(peer leadIdentity) (any, *LeadSyncError) {
			body, err := decodeSyncBody(r)
			if err != nil {
				return nil, err
			}
			err = a.sync(peer.Node, body)
			return struct {
				OK bool /* all supplied changes were stored */
			}{err == nil}, err
		})
	})
	mux.HandleFunc("POST /v1/jobs", func(w http.ResponseWriter, r *http.Request) {
		serveLead(w, r, localNode, allowed, LeadSubmitUnauthorized, func(peer leadIdentity) (any, *LeadSubmitError) {
			var body JobRequest // untrusted submission, validated by the registry
			if err := decodeLeadBody(r, &body); err != nil {
				return nil, fromValue(LeadSubmitMalformedBody, err)
			}
			job, err := a.submit(peer.Node, peer.Local, body)
			return publicLeadJob(job), err
		})
	})
	mux.HandleFunc("GET /v1/events", func(w http.ResponseWriter, r *http.Request) {
		peer, werr := identifyLeadPeer(r.RemoteAddr, localNode, allowed)
		if werr != nil {
			logWarn(LogLeadHTTPError, "operation", "lead/events", "cause", werr.Cause.String(), "error", werr.Message)
			writeLeadError(w, opErr(LeadEventsUnauthorized, "socket peer is not an allowed node"))
			return
		}
		a.doorbell(w, r, peer.Node)
	})
	mux.HandleFunc("POST /v1/mcp", func(w http.ResponseWriter, r *http.Request) {
		serveLead(w, r, localNode, allowed, LeadMCPUnauthorized, func(peer leadIdentity) (any, *LeadMCPError) {
			var body leadMCPBody // one remote member's JSON-RPC message
			if err := decodeLeadBody(r, &body); err != nil {
				return nil, fromValue(LeadMCPMalformedBody, err)
			}
			reply, err := a.mcp(peer.Node, body)
			return struct {
				Message json.RawMessage /* the JSON-RPC reply; null for a notification */
			}{reply}, err
		})
	})
	mux.HandleFunc("POST /v1/heartbeat", func(w http.ResponseWriter, r *http.Request) {
		serveLead(w, r, localNode, allowed, LeadHeartbeatUnauthorized, func(peer leadIdentity) (any, *LeadHeartbeatError) {
			var body leadHeartbeatBody // declaration and reports share one transaction
			if err := decodeLeadBody(r, &body); err != nil {
				return nil, fromValue(LeadHeartbeatMalformedBody, err)
			}
			var stops []leadJobView // durable requests from the report/assignment transaction
			var answers []LeadAnswer
			var kicks []LeadKick
			job, err := a.heartbeat(peer.Node, body, &stops, &answers, &kicks)
			var view *leadJobView // no assignment is represented by JSON null
			if job != nil {
				v := publicLeadJob(*job)
				view = &v
			}
			var wakes []string // jobs whose remote members the node must wake
			if err == nil {
				ids := make([]string, 0, len(body.Reports))
				for _, rep := range body.Reports {
					ids = append(ids, rep.ID)
				}
				a.memberJobsChanged(ids)
				var werr *LeadHeartbeatError
				if wakes, werr = a.memberActivity(peer.Node, body.Activity); werr != nil {
					warnSideEffect("apply remote members' activity", werr)
				}
			}
			wakesNow := a.wakesNowFor(peer.Node)
			bounded, berr := boundHeartbeatControls(view, stops, wakes, wakesNow, answers, kicks)
			if berr != nil {
				return nil, fromStorage(LeadHeartbeatStorage, berr)
			}
			return leadHeartbeatResponse{
				Job:      view,
				Stops:    stops,
				Wakes:    wakes,
				WakesNow: wakesNow,
				Answers:  bounded.Answers,
				Kicks:    bounded.Kicks,
			}, err
		})
	})
	mux.HandleFunc("POST /v1/jobs/{id}/state", func(w http.ResponseWriter, r *http.Request) {
		serveLead(w, r, localNode, allowed, LeadReportUnauthorized, func(peer leadIdentity) (any, *LeadReportError) {
			var body JobReport // caller-supplied identity never grants authority
			if err := decodeLeadBody(r, &body); err != nil {
				return nil, fromValue(LeadReportMalformedBody, err)
			}
			job, err := a.report(peer.Node, r.PathValue("id"), body)
			return publicLeadJob(job), err
		})
	})
	mux.HandleFunc("POST /v1/jobs/{id}/cancel", func(w http.ResponseWriter, r *http.Request) {
		serveLead(w, r, localNode, allowed, LeadCancelUnauthorized, func(peer leadIdentity) (any, *LeadCancelError) {
			job, err := a.cancel(peer.Local, r.PathValue("id"))
			return publicLeadJob(job), err
		})
	})
	mux.HandleFunc("GET /v1/jobs/{id}", func(w http.ResponseWriter, r *http.Request) {
		serveLead(w, r, localNode, allowed, LeadGetJobUnauthorized, func(_ leadIdentity) (any, *LeadGetJobError) { return a.get(r.PathValue("id")) })
	})
	mux.HandleFunc("GET /v1/jobs", func(w http.ResponseWriter, r *http.Request) {
		serveLead(w, r, localNode, allowed, LeadListJobsUnauthorized, func(_ leadIdentity) (any, *LeadListJobsError) {
			query := r.URL.Query()
			for key, values := range query {
				if key != "node" || len(values) != 1 {
					return nil, opErr(LeadListJobsInvalidArgument, "use only one optional node filter")
				}
			}
			return a.listJobs(query.Get("node"))
		})
	})
	mux.HandleFunc("GET /v1/nodes", func(w http.ResponseWriter, r *http.Request) {
		serveLead(w, r, localNode, allowed, LeadListNodesUnauthorized, func(_ leadIdentity) (any, *LeadListNodesError) { return a.listNodes() })
	})
	return mux
}

// serveLead authenticates the socket peer before decoding any operation data. Errors stay typed until encoded.
func serveLead[C opCause](w http.ResponseWriter, r *http.Request, localNode string, allowed []string, unauthorized C, run func(leadIdentity) (any, *OpError[C])) {
	if r.Header.Get("Origin") != "" || r.Header.Get("Sec-Fetch-Site") != "" {
		writeLeadError(w, opErr(unauthorized, "browser requests are not allowed"))
		return
	}
	peer, whoErr := identifyLeadPeer(r.RemoteAddr, localNode, allowed)
	if whoErr != nil {
		// Denials and broken tailscaled answers look the same to the peer; the journal keeps the real cause.
		logWarn(LogLeadPeerRejected, "remote", r.RemoteAddr, "error", whoErr.Error())
		writeLeadError(w, opErr(unauthorized, "socket peer is not an allowed node"))
		return
	}
	result, err := run(peer)
	if err != nil {
		writeLeadError(w, err)
		return
	}
	writeLeadJSON(w, http.StatusOK, result)
}

// decodeLeadBody accepts exactly one bounded JSON object, with no unknown fields or trailing values.
func decodeLeadBody(r *http.Request, out any) *ValueError {
	const limit = 256 * 1024 // maximum request bytes, independent of the task-text limit
	data, err := io.ReadAll(io.LimitReader(r.Body, limit+1))
	if err != nil {
		return valueWrap(err, "cannot read request body")
	}
	if len(data) > limit {
		return valueErr("request exceeds 262144 bytes")
	}
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return valueErr("request must be one JSON object of at most 262144 bytes")
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(out); err != nil {
		return valueWrap(err, "invalid request JSON")
	}
	if decoder.Decode(new(any)) != io.EOF {
		return valueErr("request must contain exactly one JSON object")
	}
	return nil
}

// writeLeadError exposes stable operation/cause names without reconstructing Go errors from the wire.
func writeLeadError[C opCause](w http.ResponseWriter, err *OpError[C]) {
	if err.Cause.op() == LeadSyncStorage.op() {
		logWarn(LogLeadSyncRejected, "cause", err.CauseName(), "error", err.Message)
	}
	status := http.StatusInternalServerError
	switch err.Class() {
	case ClassInvalidArgument:
		status = http.StatusBadRequest
	case ClassPermission:
		status = http.StatusForbidden
	case ClassNotFound:
		status = http.StatusNotFound
	case ClassConflict, ClassInvalidState:
		status = http.StatusConflict
	}
	writeLeadJSON(w, status, struct {
		Operation string // stable operation identifier
		Cause     string // operation-specific failure cause
		Message   string // human-readable explanation
	}{err.Cause.op(), err.CauseName(), err.Message})
}

// writeLeadJSON logs a disconnected client's write failure after the durable operation has completed.
func writeLeadJSON(w http.ResponseWriter, status int, body any) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(status)
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(body); err != nil {
		warnSideEffect("lead HTTP response", storageErr(err, "cannot write lead response"))
	}
}
