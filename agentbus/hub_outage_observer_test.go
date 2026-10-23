package main

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"
)

// flakyHubTransport stands in for the network: while down is set every attempt fails at the transport level, and
// otherwise the request is served in process by the hub handler.
type flakyHubTransport struct {
	down *atomic.Bool // true makes every attempt fail with a transport error
	next http.Handler // hub handler that serves the attempts once the hub is up
}

// RoundTrip implements http.RoundTripper for the stub network.
func (f *flakyHubTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	if f.down.Load() {
		return nil, errors.New("connection refused")
	}
	rec := httptest.NewRecorder()
	f.next.ServeHTTP(rec, r)
	return rec.Result(), nil
}

// TestHubOutageIsRecordedDuringTheCallAndClosedOnRecovery reproduces the hub-down/back case with a retry budget much
// longer than the hang limit: the outage must be on disk while the call is still retrying (not only after the budget
// is spent), and the first successful answer must close it.
func TestHubOutageIsRecordedDuringTheCallAndClosedOnRecovery(t *testing.T) {
	h, req := testHub(t)
	var down atomic.Bool
	down.Store(true)
	home := t.TempDir()
	observe := reachabilityObserver(home, "mac")
	attempts := make(chan bool, 100)
	client := &HubClient{URL: "http://hub.invalid", Instance: "mac", RetryDuration: 30 * time.Second,
		HTTP: &http.Client{Transport: &flakyHubTransport{down: &down, next: h}},
		Observe: func(reached bool) {
			observe(reached)
			select {
			case attempts <- reached:
			default: // only the first observation is awaited; never block the client
			}
		}}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan *HubClientError, 1)
	go func() {
		_, err := client.Send(ctx, req)
		done <- err
	}()
	if reached := <-attempts; reached {
		t.Fatal("first attempt reported the hub as reachable")
	}
	// The record is written before the observation is delivered, and Send cannot return before its 30 s budget or a
	// cancelled context, so an open outage here proves it was recorded during the call.
	if got := outageAt(t, home); got.OpenSince == 0 {
		t.Fatalf("outage not recorded at its first failed attempt: %+v", got)
	}
	down.Store(false)
	if err := <-done; err != nil {
		t.Fatalf("call did not succeed after recovery: %v", err)
	}
	if got := outageAt(t, home); got.OpenSince != 0 {
		t.Fatalf("recovery did not close the outage: %+v", got)
	}
}
