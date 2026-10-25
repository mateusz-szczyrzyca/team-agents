package main

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

// leadErrResponse decodes the wire error JSON returned by writeLeadError.
type leadErrResponse struct {
	Operation string // operation name, e.g. "lead/submit"
	Cause     string // cause name, e.g. "Unauthorized"
	Message   string // diagnostic message
}

// TestLeadHTTPBrowserRequestsForbidden verifies that requests bearing Origin or Sec-Fetch-Site
// headers are rejected with the operation's Unauthorized cause, while headerless CLI requests pass.
func TestLeadHTTPBrowserRequestsForbidden(t *testing.T) {
	b := newBus(t)
	h := newLeadHandler(b, "lead-node", nil, time.Minute)

	type testCase struct {
		method    string
		path      string
		body      any
		wantCause string
	}

	cases := []testCase{
		{http.MethodPost, "/v1/jobs", regRequest("job-browser"), "Unauthorized"},
		{http.MethodPost, "/v1/jobs/job-browser/cancel", nil, "Unauthorized"},
		{http.MethodGet, "/v1/jobs", nil, "Unauthorized"},
	}

	browserHeaders := []map[string]string{
		{"Origin": "http://evil.com"},
		{"Origin": "null"},
		{"Sec-Fetch-Site": "cross-site"},
		{"Sec-Fetch-Site": "same-origin"},
		{"Origin": "http://evil.com", "Sec-Fetch-Site": "cross-site"},
	}

	for _, tc := range cases {
		for _, hdr := range browserHeaders {
			var bodyReader *bytes.Reader
			if tc.body != nil {
				data, err := json.Marshal(tc.body)
				if err != nil {
					t.Fatal(err)
				}
				bodyReader = bytes.NewReader(data)
			} else {
				bodyReader = bytes.NewReader(nil)
			}
			req := httptest.NewRequest(tc.method, tc.path, bodyReader)
			req.RemoteAddr = "127.0.0.1:12345" // local authority socket peer
			for k, v := range hdr {
				req.Header.Set(k, v)
			}
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, req)

			if rec.Code != http.StatusForbidden {
				t.Fatalf("%s %s with %v: got HTTP %d, want 403 Forbidden: %s", tc.method, tc.path, hdr, rec.Code, rec.Body.String())
			}
			var resp leadErrResponse
			if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
				t.Fatalf("failed decoding error response: %v", err)
			}
			if resp.Cause != tc.wantCause {
				t.Fatalf("%s %s with %v: got cause %q, want %q", tc.method, tc.path, hdr, resp.Cause, tc.wantCause)
			}
		}
	}

	// Verify headerless CLI/node requests remain allowed.
	cliReq := httptest.NewRequest(http.MethodGet, "/v1/jobs", nil)
	cliReq.RemoteAddr = "127.0.0.1:12345"
	cliRec := httptest.NewRecorder()
	h.ServeHTTP(cliRec, cliReq)
	if cliRec.Code != http.StatusOK {
		t.Fatalf("headerless GET /v1/jobs got HTTP %d, want 200 OK: %s", cliRec.Code, cliRec.Body.String())
	}
}
