package main

import (
	"bytes"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

// TestLeadHTTPSocketPeerCannotBecomeLocal exercises authenticated nonlocal loopback over actual sockets.
// This supplements the individual identity, routing and operation-cause regressions with their composition.
func TestLeadHTTPSocketPeerCannotBecomeLocal(t *testing.T) {
	stubWhois(t, func(_, ip string) (*leadWhoisResponse, *LeadWhoisError) {
		if ip != "127.0.0.2" {
			return nil, opErr(LeadWhoisUntrustedNode, "unexpected peer")
		}
		return &leadWhoisResponse{Node: leadWhoisNode{ComputedName: "node2"}}, nil
	})
	srv := httptest.NewServer(newLeadHandler(newBus(t), "lead", []string{"node2"}, time.Minute))
	defer srv.Close()
	dialer := &net.Dialer{LocalAddr: &net.TCPAddr{IP: net.ParseIP("127.0.0.2")}, Timeout: time.Second}
	transport := &http.Transport{DialContext: dialer.DialContext}
	defer transport.CloseIdleConnections()
	remote := &http.Client{Transport: transport, Timeout: 5 * time.Second}
	local := &http.Client{Timeout: 5 * time.Second}
	request := func(client *http.Client, method, path string, body any, status int) []byte {
		t.Helper()
		data, err := json.Marshal(body)
		if err != nil {
			t.Fatal(err)
		}
		req, err := http.NewRequest(method, srv.URL+path, bytes.NewReader(data))
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("X-Forwarded-For", "127.0.0.1")
		resp, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		out, readErr := io.ReadAll(resp.Body)
		closeErr := resp.Body.Close()
		if readErr != nil {
			t.Fatal(readErr)
		}
		if closeErr != nil {
			t.Fatal(closeErr)
		}
		if resp.StatusCode != status {
			t.Fatalf("%s %s: %d want %d: %.300s", method, path, resp.StatusCode, status, out)
		}
		return out
	}
	jobReq := regRequest("socket-job")
	jobReq.Requires = []string{"hw:rp2350"}
	request(local, http.MethodPost, "/v1/jobs", jobReq, http.StatusOK)
	// A device authenticated as node2 cannot claim local human submission or cancellation privileges.
	request(remote, http.MethodPost, "/v1/jobs", regRequest("spoof"), http.StatusForbidden)
	request(remote, http.MethodPost, "/v1/jobs/socket-job/cancel", nil, http.StatusForbidden)
	beat := leadHeartbeatBody{NodeHeartbeat: regBeat("", []string{"arch:arm64"}, FreeAgent{Name: "alpha", Tier: TierNormal})}
	out := request(remote, http.MethodPost, "/v1/heartbeat", beat, http.StatusOK)
	var reply struct{ Job *Job } // one assignment or JSON null
	if err := json.Unmarshal(out, &reply); err != nil {
		t.Fatal(err)
	}
	if reply.Job != nil {
		t.Fatal("node without required hardware received a job")
	}
	beat.Capabilities = append(beat.Capabilities, "hw:rp2350")
	out = request(remote, http.MethodPost, "/v1/heartbeat", beat, http.StatusOK)
	if err := json.Unmarshal(out, &reply); err != nil {
		t.Fatal(err)
	}
	if reply.Job == nil || reply.Job.Node != "node2" {
		t.Fatalf("assignment: %.300s", out)
	}
	rep := JobReport{Assignment: reply.Job.Assignment, State: JobRunning, TaskID: "task-node2", Branch: "agents/node2/socket-job"}
	request(local, http.MethodPost, "/v1/jobs/socket-job/state", rep, http.StatusForbidden)
	request(remote, http.MethodPost, "/v1/jobs/socket-job/state", rep, http.StatusOK)
	delegated := regRequest("delegated")
	delegated.Requester = "alpha@node2"
	delegated.BaseSHA = strings.Repeat("a", 40)
	request(remote, http.MethodPost, "/v1/jobs", delegated, http.StatusOK)
}
