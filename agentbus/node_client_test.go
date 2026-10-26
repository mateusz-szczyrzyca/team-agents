package main

import (
	"bytes"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// TestNodeClientFlags rejects ambiguous URLs and routes before making requests.
func TestNodeClientFlags(t *testing.T) {
	for _, args := range [][]string{
		{}, {"--url", "https://example.com"}, {"--url", "http://user@example.com"},
		{"--url", "http://example.com/path"}, {"--url", "http://example.com?query=x"},
		{"--url", "http://example.com", "--path", "//example.com"},
		{"--url", "http://example.com", "--path", "/v1/jobs/../cancel"},
		{"--url", "http://example.com", "--path", "/v1/jobs/x/cancel"},
		{"--url", "http://example.com", "--source", "not-an-ip"},
		{"--url", "http://example.com", "--unknown"},
	} {
		_, err := parseNodeRequest(args)
		failsWith(t, NodeClientInvalidArgument)(fmt.Sprint(args), err)
	}
}

// TestNodeClientStartup validates capabilities even while the lead is unreachable.
func TestNodeClientStartup(t *testing.T) {
	cfg, err := parseNodeRequest([]string{"--url", "http://127.0.0.1:1", "--validate"})
	if err != nil {
		t.Fatal(err)
	}
	for _, input := range []string{`{"Node":"node-a","Capabilities":["sign:key"]}`, `{"Node":"../x"}`, `{"Node":"node-a","FreeAgents":[{"Name":"x","Tier":"invalid"}]}`, `{}`} {
		_, err := nodeRequest(cfg, strings.NewReader(input))
		failsWith(t, NodeClientInvalidArgument)("startup declaration", err)
	}
	out, err := nodeRequest(cfg, strings.NewReader(`{"Node":"node-a","Capabilities":["hw:rp2350"]}`))
	if err != nil || !bytes.Equal(bytes.TrimSpace(out), []byte(`{}`)) {
		t.Fatalf("validate=%s, %v", out, err)
	}
}

// TestNodeClientHTTP checks bounded transport, local source binding, and typed wire failures.
func TestNodeClientHTTP(t *testing.T) {
	for _, tc := range []struct {
		name   string          // scenario being exercised
		status int             // response status from the fake lead
		body   string          // response bytes from the fake lead
		cause  NodeClientCause // zero denotes success
	}{
		{"ok", 200, `{"Job":null,"Stops":[]}`, 0},
		{"rejected", 403, `{"Message":"denied"}`, NodeClientRejected},
		{"server", 500, `{"Message":"disk"}`, NodeClientServerError},
		{"broken", 200, `{broken`, NodeClientMalformedReply},
		// Valid JSON one byte over the reply limit: only the size check may reject it.
		{"oversized", 200, `"` + strings.Repeat("x", 1048575) + `"`, NodeClientMalformedReply},
		{"redirect", 307, `{}`, NodeClientRejected},
	} {
		t.Run(tc.name, func(t *testing.T) {
			h := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if !strings.HasPrefix(r.RemoteAddr, "127.0.0.2:") {
					t.Errorf("source=%s", r.RemoteAddr)
				}
				if r.Method != http.MethodPost || r.URL.Path != "/v1/heartbeat" {
					t.Errorf("request=%s %s", r.Method, r.URL.Path)
				}
				if tc.status == 307 {
					w.Header().Set("Location", "http://127.0.0.1:1/should-not-follow")
				}
				w.WriteHeader(tc.status)
				if _, err := fmt.Fprint(w, tc.body); err != nil {
					t.Log(err)
				}
			}))
			defer h.Close()
			cfg, err := parseNodeRequest([]string{"--url", h.URL, "--path", "/v1/heartbeat", "--source", "127.0.0.2"})
			if err != nil {
				t.Fatal(err)
			}
			out, err := nodeRequest(cfg, strings.NewReader(`{"Node":"node-a"}`))
			if tc.cause != 0 {
				failsWith(t, tc.cause)(tc.name, err)
				return
			}
			if err != nil || string(out) != tc.body {
				t.Fatalf("reply=%q, %v", out, err)
			}
		})
	}
}

// TestNodeClientLocalLimits rejects oversized or malformed bodies before connecting.
func TestNodeClientLocalLimits(t *testing.T) {
	cfg, err := parseNodeRequest([]string{"--url", "http://127.0.0.1:1", "--path", "/v1/heartbeat"})
	if err != nil {
		t.Fatal(err)
	}
	for _, body := range []string{strings.Repeat(" ", 262145), `[]`, `{}`, `{bad`, `{} {}`} {
		_, err := nodeRequest(cfg, strings.NewReader(body))
		if body == `{}` {
			failsWith(t, NodeClientUnreachable)("unreachable lead", err)
		} else {
			failsWith(t, NodeClientInvalidArgument)("invalid request", err)
		}
	}
}
