package main

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// TestNodeSyncDataRejectionDoesNotReconcile preserves valid assignments when a log batch is invalid.
func TestNodeSyncDataRejectionDoesNotReconcile(t *testing.T) {
	for _, tc := range []struct {
		name   string          // readable rejection scenario
		path   string          // operation whose transport contract is checked
		status int             // lead HTTP response status
		cause  NodeClientCause // exact node-side cause that selects retry versus reconciliation
	}{
		{"invalid sync logs", "/v1/sync", http.StatusBadRequest, NodeClientInvalidArgument},
		{"unauthorized sync", "/v1/sync", http.StatusForbidden, NodeClientRejected},
		{"expired sync job", "/v1/sync", http.StatusNotFound, NodeClientRejected},
		{"ordinary request unchanged", "/v1/heartbeat", http.StatusBadRequest, NodeClientRejected},
	} {
		t.Run(tc.name, func(t *testing.T) {
			srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				writeLeadJSON(w, tc.status, struct {
					Operation string // stable operation identifier
					Cause     string // representative invalid payload cause
				}{"lead/sync", "InvalidLogs"})
			}))
			defer srv.Close()
			_, err := nodeRequest(nodeRequestConfig{url: srv.URL, path: tc.path}, strings.NewReader(`{}`))
			failsWith(t, tc.cause)("rejection classification", err)
		})
	}
}
