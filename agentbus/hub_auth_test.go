package main

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"encoding/json"
	"math/big"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

// TestHubAuthInvalidMode checks that an unknown HUB_AUTH mode returns HubAuthInvalidMode.
func TestHubAuthInvalidMode(t *testing.T) {
	auth := NewHubAuthenticator(HubAuthMode("unsupported"))
	req := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	_, err := auth.Authenticate(req)
	if err == nil || err.Cause != HubAuthInvalidMode {
		t.Fatalf("expected HubAuthInvalidMode, got %v", err)
	}
}

// TestHubAuthTokenMode checks token authentication scenarios: success, missing headers,
// wrong token, unknown node, address mismatch, and TLS enforcement.
func TestHubAuthTokenMode(t *testing.T) {
	auth := NewHubAuthenticator(HubAuthModeToken)
	auth.Token.Tokens = map[string]string{
		"mac": "secret-token-mac",
		"pi5": "secret-token-pi5",
	}
	auth.Token.AllowedAddrs = map[string]string{
		"mac": "192.0.2.20",
		"pi5": "192.0.2.10",
	}
	auth.Token.RequireTLS = true

	// 1. Missing TLS when RequireTLS is true
	req := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	req.RemoteAddr = "192.0.2.20:1234"
	req.Header.Set("Authorization", "Bearer secret-token-mac")
	req.Header.Set("X-Instance-Name", "mac")
	_, err := auth.Authenticate(req)
	if err == nil || err.Cause != HubAuthMissingCredentials {
		t.Fatalf("expected HubAuthMissingCredentials on non-TLS request, got %v", err)
	}

	// Enable TLS for subsequent requests
	req.TLS = &tls.ConnectionState{}

	// 2. Success case
	inst, err := auth.Authenticate(req)
	if err != nil {
		t.Fatalf("expected success, got error %v", err)
	}
	if inst != "mac" {
		t.Fatalf("expected instance 'mac', got %q", inst)
	}

	// 3. Missing Authorization header
	reqNoAuth := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	reqNoAuth.TLS = &tls.ConnectionState{}
	reqNoAuth.RemoteAddr = "192.0.2.20:1234"
	reqNoAuth.Header.Set("X-Instance-Name", "mac")
	_, err = auth.Authenticate(reqNoAuth)
	if err == nil || err.Cause != HubAuthMissingCredentials {
		t.Fatalf("expected HubAuthMissingCredentials, got %v", err)
	}

	// 4. Missing X-Instance-Name header
	reqNoName := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	reqNoName.TLS = &tls.ConnectionState{}
	reqNoName.RemoteAddr = "192.0.2.20:1234"
	reqNoName.Header.Set("Authorization", "Bearer secret-token-mac")
	_, err = auth.Authenticate(reqNoName)
	if err == nil || err.Cause != HubAuthMissingCredentials {
		t.Fatalf("expected HubAuthMissingCredentials, got %v", err)
	}

	// 5. Wrong token
	reqWrongToken := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	reqWrongToken.TLS = &tls.ConnectionState{}
	reqWrongToken.RemoteAddr = "192.0.2.20:1234"
	reqWrongToken.Header.Set("Authorization", "Bearer bad-token")
	reqWrongToken.Header.Set("X-Instance-Name", "mac")
	_, err = auth.Authenticate(reqWrongToken)
	if err == nil || err.Cause != HubAuthWrongToken {
		t.Fatalf("expected HubAuthWrongToken, got %v", err)
	}

	// 6. Unknown node
	reqUnknown := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	reqUnknown.TLS = &tls.ConnectionState{}
	reqUnknown.RemoteAddr = "192.0.2.20:1234"
	reqUnknown.Header.Set("Authorization", "Bearer secret-token-mac")
	reqUnknown.Header.Set("X-Instance-Name", "unknown-instance")
	_, err = auth.Authenticate(reqUnknown)
	if err == nil || err.Cause != HubAuthUnknownNode {
		t.Fatalf("expected HubAuthUnknownNode, got %v", err)
	}

	// 7. Address mismatch
	reqAddrMismatch := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	reqAddrMismatch.TLS = &tls.ConnectionState{}
	reqAddrMismatch.RemoteAddr = "192.0.2.99:1234" // expected 192.0.2.20
	reqAddrMismatch.Header.Set("Authorization", "Bearer secret-token-mac")
	reqAddrMismatch.Header.Set("X-Instance-Name", "mac")
	_, err = auth.Authenticate(reqAddrMismatch)
	if err == nil || err.Cause != HubAuthAddressMismatch {
		t.Fatalf("expected HubAuthAddressMismatch, got %v", err)
	}
}

// TestHubAuthDuplicateInstance tests duplicate instance name rejection.
func TestHubAuthDuplicateInstance(t *testing.T) {
	auth := NewHubAuthenticator(HubAuthModeToken)
	auth.Token.Tokens = map[string]string{"mac": "token"}
	auth.Token.RequireTLS = false

	if err := auth.RegisterInstance("mac"); err != nil {
		t.Fatalf("first registration failed: %v", err)
	}

	// Second registration must fail with HubAuthDuplicateInstanceName
	err := auth.RegisterInstance("mac")
	if err == nil || err.Cause != HubAuthDuplicateInstanceName {
		t.Fatalf("expected HubAuthDuplicateInstanceName on duplicate register, got %v", err)
	}

	// Unregister allows re-registration
	auth.UnregisterInstance("mac")
	if err := auth.RegisterInstance("mac"); err != nil {
		t.Fatalf("registration after unregister failed: %v", err)
	}
}

// TestHubAuthTailscaleMode tests Tailscale LocalAPI authentication with a stub server.
func TestHubAuthTailscaleMode(t *testing.T) {
	// Stub LocalAPI server
	whoisHandler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/localapi/v0/whois" {
			http.NotFound(w, r)
			return
		}
		addr := r.URL.Query().Get("addr")
		switch addr {
		case "100.64.0.1:1234":
			_ = json.NewEncoder(w).Encode(map[string]any{
				"Node": map[string]any{
					"Name": "mac.example.ts.net.",
					"Tags": []string{"tag:agent-node"},
				},
				"UserProfile": map[string]any{
					"LoginName": "dev@example.com",
				},
			})
		case "100.64.0.2:1234":
			_ = json.NewEncoder(w).Encode(map[string]any{
				"Node": map[string]any{
					"Name": "rogue.example.ts.net.",
					"Tags": []string{},
				},
				"UserProfile": map[string]any{
					"LoginName": "unknown@example.com",
				},
			})
		case "100.64.0.3:1234":
			_ = json.NewEncoder(w).Encode(map[string]any{
				"Node": map[string]any{
					"Name": "tagged.example.ts.net.",
					"Tags": []string{"tag:team-agents"},
				},
			})
		case "100.64.0.4:1234":
			http.Error(w, "internal daemon error", http.StatusInternalServerError)
		default:
			http.Error(w, "not found", http.StatusNotFound)
		}
	})

	server := httptest.NewServer(whoisHandler)
	defer server.Close()

	auth := NewHubAuthenticator(HubAuthModeTailscale)
	auth.Tailscale.AllowedInstances = []string{"mac", "pi5", "tag:team-agents"}
	auth.Tailscale.Client = server.Client()
	// Override transport to route local-tailscaled requests to the httptest server
	serverURL := server.URL
	auth.Tailscale.Client.Transport = &rewriteTransport{
		target: serverURL,
		base:   server.Client().Transport,
	}

	// 1. Success matching bare node name "mac"
	req := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	req.RemoteAddr = "100.64.0.1:1234"
	inst, err := auth.Authenticate(req)
	if err != nil {
		t.Fatalf("expected success, got %v", err)
	}
	if inst != "mac" {
		t.Fatalf("expected 'mac', got %q", inst)
	}

	// 2. Success matching tag:team-agents with custom declared instance name
	reqTag := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	reqTag.RemoteAddr = "100.64.0.3:1234"
	reqTag.Header.Set("X-Instance-Name", "custom-node")
	inst, err = auth.Authenticate(reqTag)
	if err != nil {
		t.Fatalf("expected success with tag auth, got %v", err)
	}
	if inst != "custom-node" {
		t.Fatalf("expected 'custom-node', got %q", inst)
	}

	// 3. Unknown node (not in ALLOWED_INSTANCES)
	reqUnknown := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	reqUnknown.RemoteAddr = "100.64.0.2:1234"
	_, err = auth.Authenticate(reqUnknown)
	if err == nil || err.Cause != HubAuthUnknownNode {
		t.Fatalf("expected HubAuthUnknownNode, got %v", err)
	}

	// 4. Declared name mismatch on direct node name
	reqMismatch := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	reqMismatch.RemoteAddr = "100.64.0.1:1234"
	reqMismatch.Header.Set("X-Instance-Name", "impersonator")
	_, err = auth.Authenticate(reqMismatch)
	if err == nil || err.Cause != HubAuthUnknownNode {
		t.Fatalf("expected HubAuthUnknownNode on name mismatch, got %v", err)
	}

	// 5. LocalAPI daemon error (500)
	reqDaemonErr := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	reqDaemonErr.RemoteAddr = "100.64.0.4:1234"
	_, err = auth.Authenticate(reqDaemonErr)
	if err == nil || err.Cause != HubAuthLocalAPIFailed {
		t.Fatalf("expected HubAuthLocalAPIFailed, got %v", err)
	}

	// 6. Missing remote address
	reqNoAddr := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	reqNoAddr.RemoteAddr = ""
	_, err = auth.Authenticate(reqNoAddr)
	if err == nil || err.Cause != HubAuthMissingCredentials {
		t.Fatalf("expected HubAuthMissingCredentials, got %v", err)
	}
}

// TestPinnedTLSConfig verifies leaf certificate fingerprint verification.
func TestPinnedTLSConfig(t *testing.T) {
	// Generate self-signed certificate
	priv, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatalf("failed to generate ecdsa key: %v", err)
	}
	template := x509.Certificate{
		SerialNumber: big.NewInt(1),
		Subject:      pkix.Name{CommonName: "hub.example.com"},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(time.Hour),
	}
	derBytes, err := x509.CreateCertificate(rand.Reader, &template, &template, &priv.PublicKey, priv)
	if err != nil {
		t.Fatalf("failed to create certificate: %v", err)
	}

	leafHash := sha256.Sum256(derBytes)
	validHex := hex.EncodeToString(leafHash[:])

	cert, err := x509.ParseCertificate(derBytes)
	if err != nil {
		t.Fatalf("failed to parse certificate: %v", err)
	}
	connState := tls.ConnectionState{PeerCertificates: []*x509.Certificate{cert}}

	// 1. Success with exact hex
	tlsCfg, err := NewPinnedTLSConfig(validHex)
	if err != nil {
		t.Fatalf("failed to create pinned TLS config: %v", err)
	}
	if err := tlsCfg.VerifyConnection(connState); err != nil {
		t.Fatalf("expected valid fingerprint to pass verification, got %v", err)
	}

	// 2. Success with colon-separated uppercase hex
	var colonHex string
	for i := 0; i < len(validHex); i += 2 {
		if i > 0 {
			colonHex += ":"
		}
		colonHex += validHex[i : i+2]
	}
	tlsCfgColon, err := NewPinnedTLSConfig(colonHex)
	if err != nil {
		t.Fatalf("failed to create colon-separated pinned TLS config: %v", err)
	}
	if err := tlsCfgColon.VerifyConnection(connState); err != nil {
		t.Fatalf("expected colon-separated fingerprint to pass, got %v", err)
	}

	// 3. Mismatch detection
	tamperedHex := validHex
	if tamperedHex[0] == 'a' {
		tamperedHex = "b" + tamperedHex[1:]
	} else {
		tamperedHex = "a" + tamperedHex[1:]
	}
	tlsCfgBad, err := NewPinnedTLSConfig(tamperedHex)
	if err != nil {
		t.Fatalf("failed to create bad TLS config: %v", err)
	}
	if err := tlsCfgBad.VerifyConnection(connState); err == nil {
		t.Fatal("expected fingerprint mismatch error, got nil")
	}

	// 4. Invalid hex length
	if _, err := NewPinnedTLSConfig("abcd"); err == nil {
		t.Fatal("expected error on short hex string, got nil")
	}
}

// rewriteTransport redirects requests intended for "local-tailscaled" to the test server.
type rewriteTransport struct {
	target string
	base   http.RoundTripper
}

func (t *rewriteTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	reqCopy := req.Clone(req.Context())
	targetURL, _ := http.NewRequest(http.MethodGet, t.target, nil)
	reqCopy.URL.Scheme = targetURL.URL.Scheme
	reqCopy.URL.Host = targetURL.URL.Host
	transport := t.base
	if transport == nil {
		transport = http.DefaultTransport
	}
	return transport.RoundTrip(reqCopy)
}
