package main

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"errors"
	"math/big"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestHubServeInvalidPort verifies that invalid port numbers produce HubServeInvalidConfig.
func TestHubServeInvalidPort(t *testing.T) {
	cfg := &HubConfig{
		Port:             0,
		Auth:             HubAuthModeTailscale,
		AllowedInstances: []string{"node1"},
	}
	err := serveHub(cfg)
	if err == nil || err.Cause != HubServeInvalidConfig {
		t.Fatalf("expected HubServeInvalidConfig for port 0, got %v", err)
	}

	cfg.Port = 70000
	err = serveHub(cfg)
	if err == nil || err.Cause != HubServeInvalidConfig {
		t.Fatalf("expected HubServeInvalidConfig for port 70000, got %v", err)
	}
}

// TestHubServeInvalidAuth verifies that unsupported auth modes produce HubServeInvalidConfig.
func TestHubServeInvalidAuth(t *testing.T) {
	cfg := &HubConfig{
		Port:             7777,
		Auth:             HubAuthMode("unsupported"),
		AllowedInstances: []string{"node1"},
	}
	err := serveHub(cfg)
	if err == nil || err.Cause != HubServeInvalidConfig {
		t.Fatalf("expected HubServeInvalidConfig for unsupported auth mode, got %v", err)
	}
}

// TestHubServeEmptyAllowed verifies that empty allowed instances produce HubServeInvalidConfig.
func TestHubServeEmptyAllowed(t *testing.T) {
	cfg := &HubConfig{
		Port:             7777,
		Auth:             HubAuthModeTailscale,
		AllowedInstances: nil,
	}
	err := serveHub(cfg)
	if err == nil || err.Cause != HubServeInvalidConfig {
		t.Fatalf("expected HubServeInvalidConfig for empty allowed instances, got %v", err)
	}
}

// TestHubServeTokenMissingTLS verifies that token mode without cert/key produces HubServeInvalidConfig.
func TestHubServeTokenMissingTLS(t *testing.T) {
	cfg := &HubConfig{
		Port:             7777,
		Auth:             HubAuthModeToken,
		AllowedInstances: []string{"mac=192.0.2.1"},
		Tokens:           map[string]string{"mac": "token123"},
	}
	err := serveHub(cfg)
	if err == nil || err.Cause != HubServeInvalidConfig {
		t.Fatalf("expected HubServeInvalidConfig when TLS cert/key are missing, got %v", err)
	}
}

// TestHubServeTokenMissingToken verifies that token mode without a token for an allowed instance produces HubServeInvalidConfig.
func TestHubServeTokenMissingToken(t *testing.T) {
	cfg := &HubConfig{
		Port:             7777,
		Auth:             HubAuthModeToken,
		AllowedInstances: []string{"mac=192.0.2.1"},
		TLSCertFile:      "/tmp/cert.pem",
		TLSKeyFile:       "/tmp/key.pem",
		Tokens:           map[string]string{}, // empty
	}
	err := serveHub(cfg)
	if err == nil || err.Cause != HubServeInvalidConfig {
		t.Fatalf("expected HubServeInvalidConfig when token is missing, got %v", err)
	}
}

// TestHubServeTokenInvalidFormat verifies that malformed allowed instance definitions produce HubServeInvalidConfig.
func TestHubServeTokenInvalidFormat(t *testing.T) {
	cfg := &HubConfig{
		Port:             7777,
		Auth:             HubAuthModeToken,
		AllowedInstances: []string{"mac-without-addr"},
		TLSCertFile:      "/tmp/cert.pem",
		TLSKeyFile:       "/tmp/key.pem",
		Tokens:           map[string]string{"mac-without-addr": "tok"},
	}
	err := serveHub(cfg)
	if err == nil || err.Cause != HubServeInvalidConfig {
		t.Fatalf("expected HubServeInvalidConfig for invalid name=addr format, got %v", err)
	}
}

// TestHubServeListenInvalidAddr verifies that invalid listen addresses produce HubServeListen.
func TestHubServeListenInvalidAddr(t *testing.T) {
	dir := t.TempDir()
	cfg := &HubConfig{
		Dir:              dir,
		Listen:           "999.999.999.999:7777",
		Port:             7777,
		Auth:             HubAuthModeTailscale,
		AllowedInstances: []string{"node1"},
	}
	err := serveHub(cfg)
	if err == nil || err.Cause != HubServeListen {
		t.Fatalf("expected HubServeListen for bad IP, got %v", err)
	}
}

// TestHubServeListenBadTLSKey verifies that invalid TLS key files produce HubServeListen.
func TestHubServeListenBadTLSKey(t *testing.T) {
	dir := t.TempDir()
	badCert := filepath.Join(dir, "cert.pem")
	badKey := filepath.Join(dir, "key.pem")
	_ = os.WriteFile(badCert, []byte("bad cert data"), 0o600)
	_ = os.WriteFile(badKey, []byte("bad key data"), 0o600)

	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen: %v", err)
	}
	defer func() {
		if err := ln.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
			t.Error(err)
		}
	}()

	cfg := &HubConfig{
		Dir:              dir,
		Listener:         ln,
		Port:             7777,
		Auth:             HubAuthModeToken,
		AllowedInstances: []string{"mac=127.0.0.1"},
		Tokens:           map[string]string{"mac": "token123"},
		TLSCertFile:      badCert,
		TLSKeyFile:       badKey,
	}
	serveErr := serveHub(cfg)
	if serveErr == nil || serveErr.Cause != HubServeListen {
		t.Fatalf("expected HubServeListen for corrupt TLS files, got %v", serveErr)
	}
}

// TestHubServeSuccessGracefulShutdown verifies hub startup and graceful stop.
func TestHubServeSuccessGracefulShutdown(t *testing.T) {
	dir := t.TempDir()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}

	stopCh := make(chan struct{})
	cfg := &HubConfig{
		Dir:              dir,
		Listener:         ln,
		Port:             7777,
		Auth:             HubAuthModeTailscale,
		AllowedInstances: []string{"mac"},
		Stop:             stopCh,
	}

	done := make(chan *HubServeError, 1)
	go func() {
		done <- serveHub(cfg)
	}()

	// Verify listener is serving
	resp, httpErr := http.Get("http://" + ln.Addr().String() + "/v1/tools/send_message")
	if httpErr == nil {
		_ = resp.Body.Close()
	}

	// Trigger graceful stop
	close(stopCh)

	select {
	case serveErr := <-done:
		if serveErr != nil {
			t.Fatalf("expected clean shutdown, got error: %v", serveErr)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("timed out waiting for hub server graceful stop")
	}
}

// TestHubServeTokenModeSuccess verifies that token mode starts with valid TLS files and shuts down cleanly.
func TestHubServeTokenModeSuccess(t *testing.T) {
	dir := t.TempDir()
	certPath, keyPath := generateTestCertificate(t, dir)

	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}

	stopCh := make(chan struct{})
	cfg := &HubConfig{
		Dir:              dir,
		Listener:         ln,
		Port:             7777,
		Auth:             HubAuthModeToken,
		AllowedInstances: []string{"mac=127.0.0.1"},
		Tokens:           map[string]string{"mac": "token123"},
		TLSCertFile:      certPath,
		TLSKeyFile:       keyPath,
		Stop:             stopCh,
	}

	done := make(chan *HubServeError, 1)
	go func() {
		done <- serveHub(cfg)
	}()

	// Trigger graceful stop
	close(stopCh)

	select {
	case serveErr := <-done:
		if serveErr != nil {
			t.Fatalf("expected clean shutdown, got error: %v", serveErr)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("timed out waiting for token hub server graceful stop")
	}
}

// generateTestCertificate creates a temporary self-signed cert and key for tests.
func generateTestCertificate(t *testing.T, dir string) (string, string) {
	t.Helper()
	priv, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatalf("generate key: %v", err)
	}
	template := x509.Certificate{
		SerialNumber: big.NewInt(1),
		Subject:      pkix.Name{CommonName: "127.0.0.1"},
		IPAddresses:  []net.IP{net.ParseIP("127.0.0.1")},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(time.Hour),
	}
	der, err := x509.CreateCertificate(rand.Reader, &template, &template, &priv.PublicKey, priv)
	if err != nil {
		t.Fatalf("create cert: %v", err)
	}

	certPath := filepath.Join(dir, "cert.pem")
	if err := os.WriteFile(certPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), 0o600); err != nil { //nolint:gosec // G304: test certificate fixture written into temporary test directory
		t.Fatalf("write cert file: %v", err)
	}

	keyBytes, err := x509.MarshalECPrivateKey(priv)
	if err != nil {
		t.Fatalf("marshal key: %v", err)
	}
	keyPath := filepath.Join(dir, "key.pem")
	if err := os.WriteFile(keyPath, pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: keyBytes}), 0o600); err != nil { //nolint:gosec // G304: test private key fixture written into temporary test directory
		t.Fatalf("write key file: %v", err)
	}

	return certPath, keyPath
}

// TestHubServeUnixSocketServing verifies that the hub serves lead-local agents over its Unix domain socket.
func TestHubServeUnixSocketServing(t *testing.T) {
	dir := t.TempDir()
	sockPath := filepath.Join(dir, "test-hub.sock")
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}

	stopCh := make(chan struct{})
	cfg := &HubConfig{
		Dir:              dir,
		Listener:         ln,
		SocketPath:       sockPath,
		InstanceName:     "lead-node",
		Port:             7777,
		Auth:             HubAuthModeTailscale,
		AllowedInstances: []string{"lead-node"},
		Stop:             stopCh,
	}

	done := make(chan *HubServeError, 1)
	go func() {
		done <- serveHub(cfg)
	}()

	// Wait briefly for socket creation and verify file exists with 0600 mode
	time.Sleep(50 * time.Millisecond)
	info, err := os.Stat(sockPath)
	if err != nil {
		t.Fatalf("socket file not created: %v", err)
	}
	if info.Mode().Perm() != 0o600 {
		t.Fatalf("expected 0600 socket permissions, got %o", info.Mode().Perm())
	}

	// Make an HTTP request to the Unix domain socket
	client := &http.Client{
		Transport: &http.Transport{
			DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
				var d net.Dialer
				return d.DialContext(ctx, "unix", sockPath)
			},
		},
	}
	resp, httpErr := client.Get("http://unix-hub/v1/tools/send_message")
	if httpErr != nil {
		t.Fatalf("request over unix socket failed: %v", httpErr)
	}
	_ = resp.Body.Close()

	// Trigger graceful stop
	close(stopCh)

	select {
	case serveErr := <-done:
		if serveErr != nil {
			t.Fatalf("expected clean shutdown, got error: %v", serveErr)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("timed out waiting for graceful stop")
	}

	// Verify socket file was unlinked on exit
	if _, err := os.Stat(sockPath); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("expected socket file to be unlinked after shutdown, got: %v", err)
	}
}

// TestHubServeSocketCause verifies that failing to listen on the unix socket produces HubServeSocket.
func TestHubServeSocketCause(t *testing.T) {
	dir := t.TempDir()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	defer func() { _ = ln.Close() }()

	stopCh := make(chan struct{})
	defer close(stopCh)

	// A socket path exceeding 108 characters fails net.Listen("unix", ...) with bind: file name too long
	badSockPath := filepath.Join(dir, strings.Repeat("long_socket_path_component_", 10)+".sock")
	cfg := &HubConfig{
		Dir:              dir,
		Listener:         ln,
		SocketPath:       badSockPath,
		Port:             7777,
		Auth:             HubAuthModeTailscale,
		AllowedInstances: []string{"node1"},
		Stop:             stopCh,
	}

	serveErr := serveHub(cfg)
	if serveErr == nil || serveErr.Cause != HubServeSocket {
		t.Fatalf("expected HubServeSocket, got %v", serveErr)
	}
}

// TestHubServeDefaultResolverFailsClosed checks that defaultRunResolver rejects callers with HubCallIdentity.
func TestHubServeDefaultResolverFailsClosed(t *testing.T) {
	b := &Bus{dir: t.TempDir()}
	resolver := defaultRunResolver(b)
	caller := HubCaller{
		Agent:    "codex",
		Instance: "lead",
		Task:     "task-123",
		Team:     "team-abc",
		Tier:     TierNormal,
		Role:     RoleAgent,
	}
	server, err := resolver(caller)
	if server != nil {
		t.Fatalf("expected nil server from defaultRunResolver, got %v", server)
	}
	if err == nil || err.Cause != HubCallIdentity {
		t.Fatalf("expected HubCallIdentity error, got %v", err)
	}
}

// TestHubServeActiveSocketConflict checks that starting a hub on an active socket returns HubServeSocket without unlinking.
func TestHubServeActiveSocketConflict(t *testing.T) {
	dir := t.TempDir()
	sockPath := filepath.Join(dir, "active-hub.sock")
	activeLn, err := net.Listen("unix", sockPath)
	if err != nil {
		t.Fatalf("listen on test unix socket: %v", err)
	}
	defer func() {
		if err := activeLn.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
			t.Error(err)
		}
	}()

	tcpLn, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen on test tcp: %v", err)
	}
	defer func() {
		if err := tcpLn.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
			t.Error(err)
		}
	}()

	cfg := &HubConfig{
		Dir:              dir,
		Listener:         tcpLn,
		SocketPath:       sockPath,
		Port:             7777,
		Auth:             HubAuthModeTailscale,
		AllowedInstances: []string{"node1"},
	}

	serveErr := serveHub(cfg)
	if serveErr == nil || serveErr.Cause != HubServeSocket {
		t.Fatalf("expected HubServeSocket on active socket collision, got %v", serveErr)
	}

	// Verify the original active socket file is preserved (not unlinked)
	if _, err := os.Stat(sockPath); err != nil {
		t.Fatalf("active socket file was improperly deleted: %v", err)
	}
}
