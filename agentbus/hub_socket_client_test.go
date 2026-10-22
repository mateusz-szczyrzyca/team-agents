package main

import (
	"context"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// socketTestHub serves a hub through an owner-only Unix socket in an isolated directory.
func socketTestHub(t *testing.T, h *Hub) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "hub.sock")
	if err := os.Chmod(filepath.Dir(path), 0o700); err != nil { //nolint:gosec // private test directories need owner execute permission for traversal
		t.Fatal(err)
	}
	endpoint := httptest.NewUnstartedServer(h)
	if err := endpoint.Listener.Close(); err != nil {
		t.Fatal(err)
	}
	listener, err := net.Listen("unix", path)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(path, 0o600); err != nil {
		if closeErr := listener.Close(); closeErr != nil {
			t.Error(closeErr)
		}
		t.Fatal(err)
	}
	endpoint.Listener = listener
	endpoint.Start()
	t.Cleanup(endpoint.Close)
	if err := checkHubSocket(path); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestHubSocketClientUsesSameTypedDispatch(t *testing.T) {
	h, req := testHub(t)
	h.Authenticate = func(r *http.Request) (string, *HubAuthError) {
		if r.Header.Get("Authorization") != "" || r.Header.Get("X-Instance-Name") != "mac" {
			t.Error("local socket sent a token or lost instance identity")
		}
		return "mac", nil
	}
	client, err := newHubSocketClient(socketTestHub(t, h), "mac")
	if err != nil {
		t.Fatal(err)
	}
	response, callErr := client.Call(context.Background(), req)
	if callErr != nil || response.Result == nil {
		t.Fatalf("socket dispatch: %#v %v", response, callErr)
	}
	req.RequestID = "socket-error"
	req.Args["to"] = "codex"
	_, callErr = client.Call(context.Background(), req)
	var typed *SendMessageError
	if !errors.As(callErr, &typed) || typed.Cause != SendMessageToSelf {
		t.Fatalf("socket changed error type: %T %v", callErr, callErr)
	}
}

func TestHubSocketClientRejectsUnsafeEndpoints(t *testing.T) {
	h, req := testHub(t)
	path := socketTestHub(t, h)
	if err := os.Chmod(path, 0o666); err != nil { //nolint:gosec // deliberately unsafe test socket verifies that clients reject public access
		t.Fatal(err)
	}
	if err := checkHubSocket(path); err == nil || !errors.Is(err, ErrInvalidArgument) {
		t.Fatalf("public socket accepted: %v", err)
	}
	client, err := newHubSocketClient(path, "mac")
	if err != nil {
		t.Fatal(err)
	}
	client.RetryDuration = 10 * time.Millisecond
	_, sendErr := client.Send(context.Background(), req)
	failsWith(t, HubClientUnavailable)("", sendErr)
	if err := os.Chmod(path, 0o600); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(filepath.Dir(path), "link.sock")
	if err := os.Symlink(path, link); err != nil {
		t.Fatal(err)
	}
	if err := checkHubSocket(link); err == nil {
		t.Fatal("symlink socket accepted")
	}
	if err := os.Chmod(filepath.Dir(path), 0o777); err != nil { //nolint:gosec // deliberately unsafe test directory verifies that clients reject replacement by other users
		t.Fatal(err)
	}
	if err := checkHubSocket(path); err == nil {
		t.Fatal("socket in another user's writable directory accepted")
	}
}

func TestHubSocketClientMissingSocketIsUnavailable(t *testing.T) {
	_, req := testHub(t)
	client, err := newHubSocketClient(filepath.Join(t.TempDir(), "missing.sock"), "mac")
	if err != nil {
		t.Fatal(err)
	}
	client.RetryDuration = 10 * time.Millisecond
	_, callErr := client.Call(context.Background(), req)
	var typed *SendMessageError
	if !errors.As(callErr, &typed) || typed.Cause != SendMessageUnavailable {
		t.Fatalf("missing socket lost typed outage: %v", callErr)
	}
	_, err = newHubSocketClient("relative.sock", "mac")
	failsWith(t, HubClientInvalidConfig)("", err)
	_, err = newHubSocketClient(filepath.Join(t.TempDir(), "hub.sock"), "")
	failsWith(t, HubClientInvalidConfig)("", err)
}
