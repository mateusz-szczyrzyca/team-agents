package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// testHub trusts exactly one registered run, independently of request-supplied identity.
func testHub(t *testing.T) (*Hub, HubRequest) {
	t.Helper()
	b := newBus(t)
	s := server(b, "codex@mac")
	h := &Hub{Bus: b, Authenticate: func(*http.Request) (string, *HubAuthError) { return "mac", nil }}
	h.Resolve = func(c HubCaller) (*Server, *HubCallError) {
		if c.Agent != "codex" || c.Task != s.info.Task || c.Team != s.team || c.Tier != s.tier || c.Role != s.role {
			return nil, opErr(HubCallIdentity, "run is not registered")
		}
		return s, nil
	}
	req := HubRequest{Version: hubProtocolVersion, RequestID: "test-request", IssuedAt: Timestamp{time.Now().UTC()},
		Caller: HubCaller{Agent: "codex", Instance: "mac", Task: s.info.Task, Tier: TierNormal, Role: RoleAgent},
		Kind:   HubToolCall, Op: "send_message", Tool: "send_message", Args: map[string]any{"to": "reviewer", "text": "ready"}}
	return h, req
}

// hubHTTP sends an actual JSON request through the hub HTTP dispatcher.
func hubHTTP(t *testing.T, h *Hub, req HubRequest) (HubResponse, int) {
	t.Helper()
	data, err := json.Marshal(req)
	if err != nil {
		t.Fatal(err)
	}
	w := httptest.NewRecorder()
	h.ServeHTTP(w, httptest.NewRequest(http.MethodPost, "/v1/tools/"+req.Op, bytes.NewReader(data)))
	var response HubResponse
	if err := json.Unmarshal(w.Body.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	return response, w.Code
}

func TestHubReplaysLostMessageResponse(t *testing.T) {
	h, req := testHub(t)
	first, code := hubHTTP(t, h, req)
	if code != http.StatusOK || first.Result == nil || first.Error != nil {
		t.Fatalf("first call: %#v status %d", first, code)
	}
	second, code := hubHTTP(t, h, req)
	if code != http.StatusOK || second.Result == nil || *first.Result != *second.Result {
		t.Fatalf("replay: %#v status %d", second, code)
	}
	msgs, serr := h.Bus.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	if len(msgs) != 1 || msgs[0].From != "codex@mac" || msgs[0].To != "reviewer@mac" {
		t.Fatalf("message executed more than once or wrong instance: %#v", msgs)
	}
	// A new Hub has no in-memory response cache and must recover the durable result.
	restarted := &Hub{Bus: h.Bus, Resolve: h.Resolve, Authenticate: h.Authenticate}
	third, code := hubHTTP(t, restarted, req)
	if code != http.StatusOK || third.Result == nil || *third.Result != *first.Result {
		t.Fatalf("restart replay: %#v", third)
	}
}

func TestHubAdmissionCauses(t *testing.T) {
	tests := []struct {
		name   string
		change func(*Hub, *HubRequest)
		cause  HubCallCause
		status int
	}{
		{"version", func(_ *Hub, r *HubRequest) { r.Version++ }, HubCallMalformed, http.StatusBadRequest},
		{"kind", func(_ *Hub, r *HubRequest) { r.Kind = "invalid" }, HubCallMalformed, http.StatusBadRequest},
		{"missing ID", func(_ *Hub, r *HubRequest) { r.RequestID = "" }, HubCallMalformed, http.StatusBadRequest},
		{"tool mismatch", func(_ *Hub, r *HubRequest) { r.Tool = "read_messages" }, HubCallMalformed, http.StatusBadRequest},
		{"wrong instance", func(_ *Hub, r *HubRequest) { r.Caller.Instance = "other" }, HubCallIdentity, http.StatusForbidden},
		{"forged tier", func(_ *Hub, r *HubRequest) { r.Caller.Tier = TierWorker }, HubCallIdentity, http.StatusForbidden},
		{"forged task", func(_ *Hub, r *HubRequest) { r.Caller.Task = "another-task" }, HubCallIdentity, http.StatusForbidden},
		{"missing resolver", func(h *Hub, _ *HubRequest) { h.Resolve = nil }, HubCallIdentity, http.StatusForbidden},
		{"expired", func(_ *Hub, r *HubRequest) { r.IssuedAt = Timestamp{time.Now().Add(-hubReplayWindow)} }, HubCallExpired, http.StatusBadRequest},
		{"future", func(_ *Hub, r *HubRequest) { r.IssuedAt = Timestamp{time.Now().Add(2 * hubClockSkew)} }, HubCallExpired, http.StatusBadRequest},
		{"unknown operation", func(_ *Hub, r *HubRequest) { r.Op, r.Tool = "unknown", "unknown" }, HubCallUnknownOperation, http.StatusNotFound},
		{"missing CLI dispatcher", func(_ *Hub, r *HubRequest) { r.Kind, r.Tool = HubCLICall, "" }, HubCallUnknownOperation, http.StatusNotFound},
		{"storage", func(h *Hub, _ *HubRequest) {
			path := filepath.Join(h.Bus.dir, "replay")
			if err := os.WriteFile(path, []byte("blocked"), 0o600); err != nil {
				t.Fatal(err)
			}
		}, HubCallStorage, http.StatusServiceUnavailable},
		{"overloaded", func(h *Hub, _ *HubRequest) {
			h.active = make(map[string]*hubFlight)
			for i := range hubMaxPending {
				h.active[fmt.Sprint(i)] = &hubFlight{}
			}
		}, HubCallOverloaded, http.StatusServiceUnavailable},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			h, req := testHub(t)
			tt.change(h, &req)
			response, status := h.Execute(context.Background(), "mac", req)
			if status != tt.status || response.Error == nil || response.Error.Cause != tt.cause.String() || response.Error.Op != tt.cause.op() {
				t.Fatalf("got status %d response %#v; want %s", status, response, tt.cause)
			}
		})
	}
}

func TestHubPayloadConflict(t *testing.T) {
	h, req := testHub(t)
	if response, _ := hubHTTP(t, h, req); response.Error != nil {
		t.Fatal(response.Error)
	}
	req.Args["text"] = "different"
	response, status := hubHTTP(t, h, req)
	if status != http.StatusConflict || response.Error == nil || response.Error.Cause != HubCallRequestConflict.String() {
		t.Fatalf("changed payload replay: %#v status %d", response, status)
	}
}

func TestHubOperationErrorRetainsCauseAndMessage(t *testing.T) {
	h, req := testHub(t)
	req.Args["to"] = "codex"
	response, status := hubHTTP(t, h, req)
	if status != http.StatusOK || response.Error == nil || response.Error.Op != SendMessageToSelf.op() || response.Error.Cause != SendMessageToSelf.String() {
		t.Fatalf("handler error lost concrete cause: %#v", response)
	}
	if response.Error.Message != opErr(SendMessageToSelf, "do not send messages to yourself").Message {
		t.Fatal("wire must contain the original message without an extra operation prefix")
	}
}

func TestHubConcurrentDuplicateAndPending(t *testing.T) {
	h, req := testHub(t)
	h.PollDuration = time.Millisecond
	req.Kind, req.Tool = HubCLICall, ""
	entered, release := make(chan struct{}), make(chan struct{})
	h.CLI = func(HubRequest) HubResponse {
		close(entered) // A second execution would panic and fail this test.
		<-release
		out := []byte{0xff, 0x00, '\n'}
		return HubResponse{Stdout: &out}
	}
	response, code := hubHTTP(t, h, req)
	if response.Pending == nil || code != http.StatusOK {
		t.Fatalf("missing pending: %#v", response)
	}
	<-entered
	var wg sync.WaitGroup
	for range 5 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			r, _ := h.Execute(context.Background(), "mac", req)
			if r.Pending == nil {
				t.Errorf("duplicate must attach to original: %#v", r)
			}
		}()
	}
	wg.Wait()
	h.mu.Lock()
	var flight *hubFlight
	for _, f := range h.active {
		flight = f
	}
	h.mu.Unlock()
	close(release)
	select {
	case <-flight.done:
	case <-time.After(5 * time.Second):
		t.Fatal("original operation did not complete")
	}
	response, code = hubHTTP(t, h, req)
	if code != http.StatusOK || response.Stdout == nil || !bytes.Equal(*response.Stdout, []byte{0xff, 0x00, '\n'}) {
		t.Fatalf("binary CLI output was not preserved: %#v", response)
	}
}

func TestHubCursorResponseSurvivesLostDelivery(t *testing.T) {
	h, req := testHub(t)
	sender := server(h.Bus, "reviewer@mac")
	if _, err := sender.send("codex@mac", "answer"); err != nil {
		t.Fatal(err)
	}
	req.Op, req.Tool, req.Args = "read_messages", "read_messages", map[string]any{}
	first, _ := hubHTTP(t, h, req)
	second, _ := hubHTTP(t, h, req)
	if first.Result == nil || second.Result == nil || *first.Result != *second.Result || !strings.Contains(*second.Result, "answer") {
		t.Fatalf("cursor replay lost delivery: %#v %#v", first, second)
	}
	req.RequestID = "new-read"
	third, _ := hubHTTP(t, h, req)
	if third.Result == nil || strings.Contains(*third.Result, "answer") {
		t.Fatalf("new read must advance: %#v", third)
	}
}

func TestHubCrashAfterAcceptanceDoesNotExecuteAgain(t *testing.T) {
	h, req := testHub(t)
	h.PollDuration = time.Millisecond
	req.Kind, req.Tool = HubCLICall, ""
	release := make(chan struct{})
	h.CLI = func(HubRequest) HubResponse {
		<-release
		out := []byte("done")
		return HubResponse{Stdout: &out}
	}
	if response, _ := hubHTTP(t, h, req); response.Pending == nil {
		t.Fatal("expected pending")
	}
	restarted := &Hub{Bus: h.Bus, Resolve: h.Resolve, Authenticate: h.Authenticate,
		CLI: func(HubRequest) HubResponse { t.Error("unresolved request re-executed"); return HubResponse{} }}
	response, code := hubHTTP(t, restarted, req)
	if code != http.StatusOK || response.Error == nil || response.Error.Op != req.Op || response.Error.Cause != HubCallOutcomeUnknown.String() {
		t.Fatalf("missing explicit unknown outcome: %#v", response)
	}
	h.mu.Lock()
	var flight *hubFlight
	for _, f := range h.active {
		flight = f
	}
	h.mu.Unlock()
	close(release)
	<-flight.done
}

func TestHubHTTPRejectsMalformedBeforeDispatch(t *testing.T) {
	h, _ := testHub(t)
	for _, body := range []string{"{", "{} {}", `{"kind":"future"}`, `{"surprise":1}`} {
		w := httptest.NewRecorder()
		h.ServeHTTP(w, httptest.NewRequest(http.MethodPost, "/v1/tools/send_message", strings.NewReader(body)))
		if w.Code != http.StatusBadRequest {
			t.Fatalf("malformed body: %d", w.Code)
		}
	}
	wLarge := httptest.NewRecorder()
	h.ServeHTTP(wLarge, httptest.NewRequest(http.MethodPost, "/v1/tools/send_message", strings.NewReader(strings.Repeat(" ", hubBodyLimit+1))))
	if wLarge.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("oversized body: %d", wLarge.Code)
	}
	h.Authenticate = nil
	w := httptest.NewRecorder()
	h.ServeHTTP(w, httptest.NewRequest(http.MethodPost, "/v1/tools/send_message", strings.NewReader("not JSON")))
	if w.Code != http.StatusForbidden {
		t.Fatal("unauthenticated body was parsed")
	}
}

func TestHubHTTPEndpoint(t *testing.T) {
	h, req := testHub(t)
	endpoint := httptest.NewServer(h)
	defer endpoint.Close()
	data, err := json.Marshal(req)
	if err != nil {
		t.Fatal(err)
	}
	response, err := http.Post(endpoint.URL+"/v1/tools/send_message", "application/json", bytes.NewReader(data))
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := response.Body.Close(); err != nil {
			t.Error(err)
		}
	}()
	body, err := io.ReadAll(response.Body)
	if err != nil {
		t.Fatal(err)
	}
	if response.StatusCode != http.StatusOK || !bytes.Contains(body, []byte("Sent:")) {
		t.Fatalf("HTTP response %d %s", response.StatusCode, body)
	}
}

// hubRecordPath finds the single request record created by a test.
func hubRecordPath(t *testing.T, h *Hub) string {
	t.Helper()
	paths, err := filepath.Glob(filepath.Join(h.Bus.dir, "replay", "*.json"))
	if err != nil {
		t.Fatal(err)
	}
	if len(paths) != 1 {
		t.Fatalf("expected one replay record, got %v", paths)
	}
	return paths[0]
}

func TestHubCompletionWriteFailureHasUnknownOutcome(t *testing.T) {
	h, req := testHub(t)
	h.PollDuration = time.Millisecond
	req.Kind, req.Tool = HubCLICall, ""
	release := make(chan struct{})
	h.CLI = func(HubRequest) HubResponse {
		<-release
		out := []byte("executed")
		return HubResponse{Stdout: &out}
	}
	if response, _ := hubHTTP(t, h, req); response.Pending == nil {
		t.Fatal("expected running call")
	}
	path := hubRecordPath(t, h)
	if err := os.Mkdir(path+".tmp", 0o700); err != nil {
		t.Fatal(err)
	}
	h.mu.Lock()
	var flight *hubFlight
	for _, f := range h.active {
		flight = f
	}
	h.mu.Unlock()
	close(release)
	<-flight.done
	if flight.response.Error == nil || flight.response.Error.Cause != HubCallOutcomeUnknown.String() || flight.response.Stdout != nil {
		t.Fatalf("unrecorded completion was reported as success: %#v", flight.response)
	}
	response, _ := hubHTTP(t, h, req)
	if response.Error == nil || response.Error.Cause != HubCallOutcomeUnknown.String() {
		t.Fatalf("lost acceptance record: %#v", response)
	}
}

func TestHubReplayRetentionKeepsRequestIDTombstones(t *testing.T) {
	h, req := testHub(t)
	if response, _ := hubHTTP(t, h, req); response.Error != nil {
		t.Fatal(response.Error)
	}
	path := hubRecordPath(t, h)
	record, serr := loadHubReplay(path)
	if serr != nil {
		t.Fatal(serr)
	}
	record.IssuedAt = Timestamp{time.Now().Add(-hubReplayWindow - time.Second)}
	if serr := saveHubReplay(path, record, false); serr != nil {
		t.Fatal(serr)
	}
	if err := h.PruneReplay(); err != nil {
		t.Fatal(err)
	}
	record, serr = loadHubReplay(path)
	if serr != nil {
		t.Fatal(serr)
	}
	if record.Response != nil || !record.Expired {
		t.Fatal("expired response was not reclaimed")
	}
	response, _ := hubHTTP(t, h, req)
	if response.Error == nil || response.Error.Cause != HubCallExpired.String() {
		t.Fatalf("expired request replayed: %#v", response)
	}
	// Refreshing an old ID's timestamp must conflict even after its response is reclaimed.
	req.IssuedAt = Timestamp{time.Now()}
	response, status := hubHTTP(t, h, req)
	if status != http.StatusConflict || response.Error == nil || response.Error.Cause != HubCallRequestConflict.String() {
		t.Fatalf("old request ID reused after pruning: %#v", response)
	}
}

func TestHubCorruptReplayFailsClosed(t *testing.T) {
	h, req := testHub(t)
	if response, _ := hubHTTP(t, h, req); response.Error != nil {
		t.Fatal(response.Error)
	}
	path := hubRecordPath(t, h)
	if err := os.WriteFile(path, []byte("invalid"), 0o600); err != nil {
		t.Fatal(err)
	}
	response, status := hubHTTP(t, h, req)
	if status != http.StatusServiceUnavailable || response.Error == nil || response.Error.Cause != HubCallStorage.String() {
		t.Fatalf("corrupt replay ignored: %#v", response)
	}
	msgs, serr := h.Bus.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	if len(msgs) != 1 {
		t.Fatal("corrupt replay caused duplicate execution")
	}
}

func TestHubWaitForMessageUsesQualifiedSender(t *testing.T) {
	h, req := testHub(t)
	sender := server(h.Bus, "reviewer@mac")
	if _, err := sender.send("codex@mac", "reply"); err != nil {
		t.Fatal(err)
	}
	req.Op, req.Tool = "wait_for_message", "wait_for_message"
	req.Args = map[string]any{"from": "reviewer", "timeout_seconds": float64(1)}
	response, status := hubHTTP(t, h, req)
	if status != http.StatusOK || response.Result == nil || !strings.Contains(*response.Result, "reply") {
		t.Fatalf("wait did not resolve local sender: %#v", response)
	}
	second, _ := hubHTTP(t, h, req)
	if second.Result == nil || *second.Result != *response.Result {
		t.Fatal("lost wait response was not replayed")
	}
}

func TestHubMCPUnreadCountComesFromHub(t *testing.T) {
	h, req := testHub(t)
	sender := server(h.Bus, "reviewer@mac")
	if _, err := sender.send("codex@mac", "unread"); err != nil {
		t.Fatal(err)
	}
	req.Op, req.Tool = "set_status", "set_status"
	req.Args = map[string]any{"status": "working"}
	response, status := hubHTTP(t, h, req)
	if status != http.StatusOK || response.UnreadCount != 1 {
		t.Fatalf("footer count missing: %#v", response)
	}
}

func TestHubDispatcherErrorKeepsItsOwnOperation(t *testing.T) {
	h, req := testHub(t)
	req.Args["text"] = float64(4)
	response, status := hubHTTP(t, h, req)
	if status != http.StatusOK || response.Error == nil || response.Error.Op != ToolCallInvalidArgument.op() || response.Error.Cause != ToolCallInvalidArgument.String() {
		t.Fatalf("dispatcher cause changed: %#v", response)
	}
}

func TestHubAuthenticationHTTPStatuses(t *testing.T) {
	for _, tt := range []struct {
		cause  HubAuthCause
		status int
	}{
		{HubAuthMissingCredentials, http.StatusUnauthorized},
		{HubAuthWrongToken, http.StatusUnauthorized},
		{HubAuthUnknownNode, http.StatusForbidden},
		{HubAuthDuplicateInstanceName, http.StatusConflict},
		{HubAuthInvalidMode, http.StatusBadRequest},
		{HubAuthLocalAPIFailed, http.StatusServiceUnavailable},
	} {
		t.Run(tt.cause.String(), func(t *testing.T) {
			h, req := testHub(t)
			h.Authenticate = func(*http.Request) (string, *HubAuthError) { return "", opErr(tt.cause, "rejected") }
			response, code := hubHTTP(t, h, req)
			if code != tt.status || response.Error == nil || response.Error.Cause != tt.cause.String() {
				t.Fatalf("auth error: %d %#v", code, response)
			}
		})
	}
}

func TestHubPendingWaitContinuesAfterObserverCancellation(t *testing.T) {
	h, req := testHub(t)
	h.PollDuration = time.Millisecond
	req.Op, req.Tool = "wait_for_message", "wait_for_message"
	req.Args = map[string]any{"from": "reviewer", "timeout_seconds": float64(10)}
	response, _ := hubHTTP(t, h, req)
	if response.Pending == nil {
		t.Fatalf("expected pending message wait: %#v", response)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	response, _ = h.Execute(ctx, "mac", req)
	if response.Pending == nil {
		t.Fatalf("canceled observer must detach: %#v", response)
	}
	sender := server(h.Bus, "reviewer@mac")
	if _, err := sender.send("codex@mac", "after cancel"); err != nil {
		t.Fatal(err)
	}
	h.PollDuration = 5 * time.Second
	response, _ = h.Execute(context.Background(), "mac", req)
	if response.Result == nil || !strings.Contains(*response.Result, "after cancel") {
		t.Fatalf("original wait was not preserved: %#v", response)
	}
}

func TestHubTimestampsPreserveInstantsAcrossTimeZones(t *testing.T) {
	h, req := testHub(t)
	req.IssuedAt = Timestamp{time.Now().In(time.FixedZone("remote", 5*60*60))}
	data, err := json.Marshal(req)
	if err != nil {
		t.Fatal(err)
	}
	var decoded HubRequest
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatal(err)
	}
	if !decoded.IssuedAt.Equal(req.IssuedAt.Time) || decoded.IssuedAt.Location() != time.UTC {
		t.Fatal("request lost its UTC instant")
	}
	if response, _ := hubHTTP(t, h, req); response.Error != nil {
		t.Fatal(response.Error)
	}
	record, serr := loadHubReplay(hubRecordPath(t, h))
	if serr != nil {
		t.Fatal(serr)
	}
	if !record.IssuedAt.Equal(req.IssuedAt.Time) {
		t.Fatal("replay record lost time zone information")
	}
}

func TestHubFreezesLeadLocalArgumentsBeforeDispatch(t *testing.T) {
	h, req := testHub(t)
	h.PollDuration = time.Millisecond
	req.Kind, req.Tool = HubCLICall, ""
	entered, release := make(chan struct{}), make(chan struct{})
	h.CLI = func(accepted HubRequest) HubResponse {
		close(entered)
		<-release
		text := accepted.Args["text"].(string)
		return HubResponse{Result: &text}
	}
	response, _ := h.Execute(context.Background(), "mac", req)
	if response.Pending == nil {
		t.Fatal("expected pending call")
	}
	<-entered
	req.Args["text"] = "changed after acceptance"
	h.mu.Lock()
	var flight *hubFlight
	for _, f := range h.active {
		flight = f
	}
	h.mu.Unlock()
	close(release)
	<-flight.done
	if flight.response.Result == nil || *flight.response.Result != "ready" {
		t.Fatalf("accepted payload was mutated: %#v", flight.response)
	}
}
