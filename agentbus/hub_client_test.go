package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestHubClientTypedReleaseErrors(t *testing.T) {
	for _, kind := range []HubCallKind{HubToolCall, HubCLICall} {
		ok, err := decodeHubWireError(kind, &HubWireError{Op: "release", Cause: "Storage", Message: "disk unavailable"})
		if !ok || !errors.Is(err, ErrStorage) {
			t.Fatalf("lost storage sentinel: %v", err)
		}
		if kind == HubToolCall {
			var typed *ReleaseError
			if !errors.As(err, &typed) || typed.Cause != ReleaseStorage {
				t.Fatalf("wrong MCP error type: %T", err)
			}
		} else {
			var typed *CmdReleaseError
			if !errors.As(err, &typed) || typed.Cause != CmdReleaseStorage {
				t.Fatalf("wrong CLI error type: %T", err)
			}
		}
	}
	if ok, _ := decodeHubWireError(HubToolCall, &HubWireError{Op: "release", Cause: "Unknown"}); ok {
		t.Fatal("unknown release cause accepted")
	}
}

func TestHubClientCallsHubAndPreservesError(t *testing.T) {
	h, req := testHub(t)
	endpoint := httptest.NewServer(h)
	defer endpoint.Close()
	c := &HubClient{URL: endpoint.URL, Instance: "mac"}
	req.Args["to"] = "codex"
	response, err := c.Call(context.Background(), req)
	var typed *SendMessageError
	if !errors.As(err, &typed) || typed.Cause != SendMessageToSelf || !errors.Is(err, ErrInvalidArgument) || response.Error == nil {
		t.Fatalf("typed failure lost: %#v %v", response, err)
	}
}

func TestHubClientRetriesLostResponseWithSameEnvelope(t *testing.T) {
	h, req := testHub(t)
	var calls atomic.Int32
	first := make(chan []byte, 1)
	endpoint := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Error(err)
			return
		}
		if calls.Add(1) == 1 {
			first <- body
			// Execute the mutation, then close before its durable response reaches the client.
			recorder := httptest.NewRecorder()
			r.Body = io.NopCloser(bytes.NewReader(body))
			h.ServeHTTP(recorder, r)
			conn, _, err := w.(http.Hijacker).Hijack()
			if err != nil {
				t.Error(err)
				return
			}
			if err := conn.Close(); err != nil {
				t.Error(err)
			}
			return
		}
		if !bytes.Equal(<-first, body) {
			t.Error("retry changed envelope")
		}
		r.Body = io.NopCloser(bytes.NewReader(body))
		h.ServeHTTP(w, r)
	}))
	defer endpoint.Close()
	c := &HubClient{URL: endpoint.URL, Instance: "mac", RetryDuration: time.Second}
	response, err := c.Call(context.Background(), req)
	if err != nil || response.Result == nil || calls.Load() != 2 {
		t.Fatalf("retry failed: %#v %v calls=%d", response, err, calls.Load())
	}
	msgs, serr := h.Bus.messages()
	if serr != nil || len(msgs) != 1 {
		t.Fatalf("mutation repeated: %v %v", msgs, serr)
	}
}

func TestHubClientUnavailableIsBoundedAndTyped(t *testing.T) {
	h, req := testHub(t)
	endpoint := httptest.NewServer(h)
	endpoint.Close()
	c := &HubClient{URL: endpoint.URL, Instance: "mac", RetryDuration: 20 * time.Millisecond}
	started := time.Now()
	_, err := c.Call(context.Background(), req)
	var typed *SendMessageError
	if !errors.As(err, &typed) || typed.Cause != SendMessageUnavailable || !errors.Is(err, ErrUnavailable) {
		t.Fatalf("wrong exhausted-retry error: %T %v", err, err)
	}
	if time.Since(started) > time.Second {
		t.Fatal("retry budget exceeded")
	}
}

func TestHubClientPendingDoesNotBecomeToolResult(t *testing.T) {
	h, req := testHub(t)
	var calls atomic.Int32
	endpoint := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if calls.Add(1) <= 2 {
			if err := json.NewEncoder(w).Encode(HubResponse{Version: 1, RequestID: req.RequestID, Pending: &HubPending{}}); err != nil {
				t.Error(err)
			}
			return
		}
		h.ServeHTTP(w, r)
	}))
	defer endpoint.Close()
	c := &HubClient{URL: endpoint.URL, Instance: "mac", RetryDuration: time.Second}
	response, err := c.Call(context.Background(), req)
	if err != nil || response.Result == nil || response.Pending != nil || calls.Load() != 3 {
		t.Fatalf("pending was not reattached: %#v %v", response, err)
	}
}

func TestHubClientRejectsBadResponsesAndRedirects(t *testing.T) {
	_, req := testHub(t)
	for _, body := range []string{
		`{`, `{}`, `{"version":1,"request_id":"test-request","result":"x","pending":{}}`,
		`{"version":1,"request_id":"different","result":"x"}`,
		`{"version":1,"request_id":"test-request","error":{"op":"send_message","cause":"future","message":"x"}}`,
		`{"version":1,"request_id":"test-request","stdout":""}`,
		`{"version":1,"request_id":"test-request","error":{"op":"quiz_start","cause":"Storage","message":"x"}}`,
		`{"version":1,"request_id":"test-request","result":"x"} {}`,
	} {
		t.Run(body, func(t *testing.T) {
			endpoint := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				if _, err := io.WriteString(w, body); err != nil {
					t.Error(err)
				}
			}))
			defer endpoint.Close()
			c := &HubClient{URL: endpoint.URL, Instance: "mac"}
			_, err := c.Send(context.Background(), req)
			failsWith(t, HubClientMalformed)("", err)
		})
	}
	var redirected atomic.Bool
	target := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { redirected.Store(true) }))
	defer target.Close()
	redirect := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, target.URL, http.StatusTemporaryRedirect)
	}))
	defer redirect.Close()
	c := &HubClient{URL: redirect.URL, Instance: "mac"}
	_, err := c.Send(context.Background(), req)
	failsWith(t, HubClientMalformed)("", err)
	if redirected.Load() {
		t.Fatal("followed redirect to another peer")
	}
}

func TestHubClientConfigAndRequestValidation(t *testing.T) {
	_, req := testHub(t)
	for _, client := range []*HubClient{
		{URL: "http://example.com", Instance: "mac", Token: "private"},
		{URL: "http://user@example.com", Instance: "mac"},
		{URL: "http://example.com/path", Instance: "mac"},
		{URL: "http://example.com"},
		{URL: "http://example.com", Instance: "mac", RetryDuration: -1},
	} {
		_, err := client.Send(context.Background(), req)
		failsWith(t, HubClientInvalidConfig)("", err)
	}
	c := &HubClient{URL: "http://example.com", Instance: "mac"}
	req.Args["text"] = strings.Repeat("x", hubBodyLimit)
	_, err := c.Send(context.Background(), req)
	failsWith(t, HubClientMalformed)("", err)
	first, err := c.NewRequest(req.Caller, HubToolCall, "send_message", nil)
	if err != nil || first.RequestID == "" || first.IssuedAt.IsZero() || first.Tool != "send_message" {
		t.Fatalf("new request: %#v %v", first, err)
	}
	second, err := c.NewRequest(req.Caller, HubCLICall, "send", nil)
	if err != nil || first.RequestID == second.RequestID || second.Tool != "" {
		t.Fatalf("new CLI request: %#v %v", second, err)
	}
}

func TestHubForwardMCPDoesNotOpenLocalBus(t *testing.T) {
	h, req := testHub(t)
	sender := server(h.Bus, "reviewer@mac")
	if _, err := sender.send("codex@mac", "unread"); err != nil {
		t.Fatal(err)
	}
	endpoint := httptest.NewServer(h)
	defer endpoint.Close()
	local := filepath.Join(t.TempDir(), "never-create")
	var out bytes.Buffer
	s := &Server{bus: &Bus{dir: local}, me: "codex", out: json.NewEncoder(&out), hubCaller: req.Caller,
		hubClient: &HubClient{URL: endpoint.URL, Instance: "mac"}}
	s.handle([]byte(`{"id":1,"method":"tools/call","params":{"name":"set_status","arguments":{"status":"remote"}}}`))
	if !strings.Contains(out.String(), "1 unread messages") || !strings.Contains(out.String(), "Status set.") {
		t.Fatalf("remote footer lost: %s", out.String())
	}
	if _, err := os.Stat(local); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("forwarding opened local bus: %v", err)
	}
}

func TestHubClientLocalDispatcher(t *testing.T) {
	h, req := testHub(t)
	c := &HubClient{Local: h, Instance: "mac"}
	response, err := c.Call(context.Background(), req)
	if err != nil || response.Result == nil {
		t.Fatalf("in-process dispatch: %#v %v", response, err)
	}
}

func TestHubClientEnvironmentAndPinnedTLS(t *testing.T) {
	for _, key := range []string{"LEAD_INSTANCE", "INSTANCE_NAME", "HUB_LISTEN", "HUB_AUTH", "HUB_TOKEN", "HUB_CERT_FINGERPRINT", "HUB_RETRY_SECONDS"} {
		t.Setenv(key, "")
	}
	client, err := hubClientFromEnv()
	if client != nil || err != nil {
		t.Fatal("single-instance mode enabled forwarding")
	}
	t.Setenv("HUB_LISTEN", "127.0.0.1:7777")
	t.Setenv("INSTANCE_NAME", "mac")
	client, err = hubClientFromEnv()
	if client == nil || err != nil || client.URL != "http://local-hub" {
		t.Fatal("lead-local serve must select the Unix socket without file fallback")
	}
	t.Setenv("HUB_LISTEN", "")
	t.Setenv("INSTANCE_NAME", "mac")
	t.Setenv("LEAD_INSTANCE", "example.com:7777")
	t.Setenv("HUB_AUTH", "invalid")
	_, err = hubClientFromEnv()
	failsWith(t, HubClientInvalidConfig)("", err)
	t.Setenv("HUB_AUTH", "tailscale")
	t.Setenv("HUB_RETRY_SECONDS", "0")
	_, err = hubClientFromEnv()
	failsWith(t, HubClientInvalidConfig)("", err)
	t.Setenv("HUB_RETRY_SECONDS", "30")
	client, err = hubClientFromEnv()
	if err != nil || client.URL != "http://example.com:7777" || client.RetryDuration != 30*time.Second {
		t.Fatalf("Tailscale defaults: %#v %v", client, err)
	}
	t.Setenv("HUB_AUTH", "token")
	_, err = hubClientFromEnv()
	failsWith(t, HubClientInvalidConfig)("", err)
	t.Setenv("HUB_TOKEN", "test-instance-token")
	_, err = hubClientFromEnv()
	failsWith(t, HubClientInvalidConfig)("", err)
	h, req := testHub(t)
	auth := NewHubAuthenticator(HubAuthModeToken)
	auth.Token.Tokens["mac"] = "test-instance-token"
	auth.Token.AllowedAddrs["mac"] = "127.0.0.1"
	h.Authenticate = auth.Authenticate
	endpoint := httptest.NewTLSServer(h)
	defer endpoint.Close()
	fingerprint := sha256.Sum256(endpoint.Certificate().Raw)
	t.Setenv("HUB_CERT_FINGERPRINT", hex.EncodeToString(fingerprint[:]))
	t.Setenv("LEAD_INSTANCE", endpoint.URL)
	client, err = hubClientFromEnv()
	if err != nil {
		t.Fatal(err)
	}
	response, callErr := client.Call(context.Background(), req)
	if callErr != nil || response.Result == nil {
		t.Fatalf("pinned TLS forwarding: %#v %v", response, callErr)
	}
}

func TestHubClientDoesNotRetryUnknownOutcome(t *testing.T) {
	_, req := testHub(t)
	var calls atomic.Int32
	endpoint := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		calls.Add(1)
		response := HubResponse{Version: 1, RequestID: req.RequestID,
			Error: &HubWireError{Op: req.Op, Cause: "OutcomeUnknown", Message: "reconcile before sending new work"}}
		if err := json.NewEncoder(w).Encode(response); err != nil {
			t.Error(err)
		}
	}))
	defer endpoint.Close()
	c := &HubClient{URL: endpoint.URL, Instance: "mac"}
	_, err := c.Call(context.Background(), req)
	var typed *SendMessageError
	if calls.Load() != 1 || !errors.As(err, &typed) || typed.Cause != SendMessageOutcomeUnknown || !errors.Is(err, ErrInvalidState) {
		t.Fatalf("unknown outcome retried or changed: calls=%d error=%v", calls.Load(), err)
	}
}

func TestHubClientBlackholedPeerAndPendingCap(t *testing.T) {
	_, req := testHub(t)
	for _, pending := range []bool{false, true} {
		t.Run(strconv.FormatBool(pending), func(t *testing.T) {
			release := make(chan struct{})
			endpoint := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if !pending {
					select {
					case <-r.Context().Done():
					case <-release:
					}
					return
				}
				if err := json.NewEncoder(w).Encode(HubResponse{Version: 1, RequestID: req.RequestID, Pending: &HubPending{}}); err != nil {
					t.Error(err)
				}
			}))
			defer endpoint.Close()
			c := &HubClient{URL: endpoint.URL, Instance: "mac", RetryDuration: 40 * time.Millisecond, PendingDuration: 60 * time.Millisecond}
			_, err := c.Send(context.Background(), req)
			close(release)
			failsWith(t, HubClientUnavailable)("", err)
		})
	}
}

func TestHubUnavailableMCPIncludesReadGuidance(t *testing.T) {
	h, req := testHub(t)
	endpoint := httptest.NewServer(h)
	endpoint.Close()
	var out bytes.Buffer
	s := &Server{out: json.NewEncoder(&out), hubCaller: req.Caller,
		hubClient: &HubClient{URL: endpoint.URL, Instance: "mac", RetryDuration: 10 * time.Millisecond}}
	s.handle([]byte(`{"id":1,"method":"tools/call","params":{"name":"send_message","arguments":{"to":"reviewer","text":"ready"}}}`))
	if !strings.Contains(out.String(), "read tool") || !strings.Contains(out.String(), "new request ID") || !strings.Contains(out.String(), `"isError":true`) {
		t.Fatalf("MCP response did not explain reconciliation: %s", out.String())
	}
}
