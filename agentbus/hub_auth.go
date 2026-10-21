// hub_auth implements authentication and peer verification for the multi-instance hub.
//
// Two authentication modes are supported:
// 1. Tailscale mode (default): the hub queries the local tailscaled daemon via its LocalAPI
//    unix domain socket (/localapi/v0/whois) using the client's RemoteAddr, and validates
//    node names and tags against ALLOWED_INSTANCES.
// 2. Token mode: mutual authentication using pre-shared bearer tokens over TLS, verifying
//    both the token and the expected source IP address from ALLOWED_INSTANCES.

package main

import (
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"crypto/tls"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
)

// HubAuthMode is the authentication mode used by the hub.
type HubAuthMode string

const (
	// HubAuthModeTailscale authenticates clients via Tailscale LocalAPI whois over a unix socket.
	HubAuthModeTailscale HubAuthMode = "tailscale"

	// HubAuthModeToken authenticates clients using pre-shared bearer tokens over TLS.
	HubAuthModeToken HubAuthMode = "token"
)

// DefaultTailscaleLinuxSocket is the standard LocalAPI socket path on Linux.
const DefaultTailscaleLinuxSocket = "/var/run/tailscale/tailscaled.sock"

// DefaultTailscaleDarwinSocket is the standard LocalAPI socket path on macOS.
const DefaultTailscaleDarwinSocket = "/var/run/tailscaled.socket"

// TailscaleAuthConfig configures Tailscale LocalAPI authentication.
type TailscaleAuthConfig struct {
	// SocketPath is the path to the tailscaled LocalAPI unix socket.
	SocketPath string
	// AllowedInstances is the list of allowed node names, FQDNs, or "tag:..." tags.
	AllowedInstances []string
	// Client is an optional HTTP client used to query the LocalAPI (for testing).
	Client *http.Client
}

// TokenAuthConfig configures pre-shared bearer token authentication.
type TokenAuthConfig struct {
	// Tokens maps instance name -> expected bearer token.
	Tokens map[string]string
	// AllowedAddrs maps instance name -> expected source IP address.
	AllowedAddrs map[string]string
	// RequireTLS enforces that incoming requests must use TLS (r.TLS != nil).
	RequireTLS bool
}

// HubAuthenticator authenticates incoming HTTP requests to the hub.
type HubAuthenticator struct {
	// Mode is the active authentication mode ("tailscale" or "token").
	Mode HubAuthMode
	// Tailscale holds configuration for Tailscale authentication mode.
	Tailscale TailscaleAuthConfig
	// Token holds configuration for Token authentication mode.
	Token TokenAuthConfig
	// CheckDuplicate is an optional callback returning true if an instance is already active.
	CheckDuplicate func(instance string) bool

	// mu protects activeInstances.
	mu sync.Mutex
	// activeInstances tracks currently registered instance names when CheckDuplicate is nil.
	activeInstances map[string]bool
}

// NewHubAuthenticator creates a new HubAuthenticator for the given mode.
func NewHubAuthenticator(mode HubAuthMode) *HubAuthenticator {
	return &HubAuthenticator{
		Mode:            mode,
		Token:           TokenAuthConfig{Tokens: make(map[string]string), AllowedAddrs: make(map[string]string), RequireTLS: true},
		activeInstances: make(map[string]bool),
	}
}

// tailscaleWhoisNode represents node information in the /localapi/v0/whois response.
type tailscaleWhoisNode struct {
	// Name is the fully-qualified domain name of the node (e.g. "mac.example.ts.net.").
	Name string `json:"Name"`
	// Tags contains any Tailscale ACL tags assigned to the node (e.g. ["tag:agent-node"]).
	Tags []string `json:"Tags"`
}

// tailscaleWhoisUserProfile represents user profile information in the whois response.
type tailscaleWhoisUserProfile struct {
	// LoginName is the Tailscale login name of the user.
	LoginName string `json:"LoginName"`
}

// tailscaleWhoisResponse is the subset of /localapi/v0/whois response fields needed for auth.
type tailscaleWhoisResponse struct {
	// Node contains Tailscale node identity fields.
	Node tailscaleWhoisNode `json:"Node"`
	// UserProfile contains user profile information.
	UserProfile tailscaleWhoisUserProfile `json:"UserProfile"`
}

// RegisterInstance marks an instance name as actively connected. Returns HubAuthDuplicateInstanceName
// if the instance name is already active.
func (a *HubAuthenticator) RegisterInstance(name string) *HubAuthError {
	if a.isDuplicate(name) {
		return opErr(HubAuthDuplicateInstanceName, "instance %q is already connected with an active lease", name)
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.activeInstances == nil {
		a.activeInstances = make(map[string]bool)
	}
	a.activeInstances[name] = true
	return nil
}

// UnregisterInstance releases an instance name when disconnected.
func (a *HubAuthenticator) UnregisterInstance(name string) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.activeInstances != nil {
		delete(a.activeInstances, name)
	}
}

// IsInstanceActive reports whether the instance name is currently recorded as active.
func (a *HubAuthenticator) IsInstanceActive(name string) bool {
	return a.isDuplicate(name)
}

func (a *HubAuthenticator) isDuplicate(name string) bool {
	if a.CheckDuplicate != nil {
		return a.CheckDuplicate(name)
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.activeInstances != nil && a.activeInstances[name]
}

// Authenticate verifies the incoming HTTP request against the configured authentication mode
// and returns the authenticated instance name, or a *HubAuthError on failure.
func (a *HubAuthenticator) Authenticate(r *http.Request) (string, *HubAuthError) {
	switch a.Mode {
	case HubAuthModeTailscale:
		return a.authenticateTailscale(r)
	case HubAuthModeToken:
		return a.authenticateToken(r)
	default:
		return "", opErr(HubAuthInvalidMode, "unsupported HUB_AUTH mode %q", a.Mode)
	}
}

// authenticateTailscale verifies the request using the local tailscaled LocalAPI whois service.
func (a *HubAuthenticator) authenticateTailscale(r *http.Request) (string, *HubAuthError) {
	remoteAddr := r.RemoteAddr
	if remoteAddr == "" {
		return "", opErr(HubAuthMissingCredentials, "client remote address is missing")
	}

	host, _, err := net.SplitHostPort(remoteAddr)
	if err != nil {
		// Bare IP without port fallback
		host = remoteAddr
	}
	if host == "" {
		return "", opErr(HubAuthMissingCredentials, "client remote address is invalid: %s", remoteAddr)
	}

	client := a.Tailscale.Client
	if client == nil {
		sock := a.Tailscale.SocketPath
		if sock == "" {
			sock = DefaultTailscaleLinuxSocket
		}
		client = &http.Client{
			Transport: &http.Transport{
				DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
					var d net.Dialer
					return d.DialContext(ctx, "unix", sock)
				},
			},
		}
	}

	whoisURL := fmt.Sprintf("http://local-tailscaled/localapi/v0/whois?addr=%s", url.QueryEscape(remoteAddr))
	req, err := http.NewRequestWithContext(r.Context(), http.MethodGet, whoisURL, nil) //nolint:gosec // G704: query target is fixed local-tailscaled unix domain socket
	if err != nil {
		return "", opErr(HubAuthLocalAPIFailed, "failed to create whois request: %v", err)
	}
	req.Host = "local-tailscaled"

	resp, err := client.Do(req) //nolint:gosec // G704: dialed exclusively via unix domain socket
	if err != nil {
		return "", opErr(HubAuthLocalAPIFailed, "tailscaled LocalAPI query failed: %v", err)
	}
	defer func() {
		if err := resp.Body.Close(); err != nil {
			warnSideEffect("close tailscale whois body", storageErr(err, "cannot close tailscaled response body"))
		}
	}()

	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return "", opErr(HubAuthLocalAPIFailed, "tailscaled LocalAPI returned status %d: %s", resp.StatusCode, strings.TrimSpace(string(body)))
	}

	var whois tailscaleWhoisResponse
	if err := json.NewDecoder(resp.Body).Decode(&whois); err != nil {
		return "", opErr(HubAuthLocalAPIFailed, "failed to decode tailscaled whois response: %v", err)
	}

	rawNodeName := strings.TrimSuffix(whois.Node.Name, ".")
	if rawNodeName == "" {
		return "", opErr(HubAuthUnknownNode, "tailscaled whois returned empty node name for address %s", remoteAddr)
	}

	// Base node name is the first label of the FQDN (e.g. "mac" from "mac.example.ts.net")
	baseNodeName := strings.SplitN(rawNodeName, ".", 2)[0]
	declaredName := r.Header.Get("X-Instance-Name")

	// Match node identity against ALLOWED_INSTANCES
	allowed := false
	for _, entry := range a.Tailscale.AllowedInstances {
		entry = strings.TrimSpace(entry)
		if entry == "" {
			continue
		}
		if strings.HasPrefix(entry, "tag:") {
			for _, tag := range whois.Node.Tags {
				if tag == entry {
					allowed = true
					break
				}
			}
		} else if strings.EqualFold(entry, baseNodeName) || strings.EqualFold(entry, rawNodeName) {
			allowed = true
		}
		if allowed {
			break
		}
	}

	if !allowed {
		return "", opErr(HubAuthUnknownNode, "node %q (FQDN: %q, tags: %v) is not in ALLOWED_INSTANCES", baseNodeName, rawNodeName, whois.Node.Tags)
	}

	resolvedInstance := baseNodeName
	if declaredName != "" {
		// If declared name is provided, verify it matches the node name or is authorized by tag
		isTagAuthorized := false
		for _, entry := range a.Tailscale.AllowedInstances {
			if strings.HasPrefix(entry, "tag:") {
				for _, tag := range whois.Node.Tags {
					if tag == entry {
						isTagAuthorized = true
						break
					}
				}
			}
		}
		if !isTagAuthorized && !strings.EqualFold(declaredName, baseNodeName) && !strings.EqualFold(declaredName, rawNodeName) {
			return "", opErr(HubAuthUnknownNode, "declared instance name %q does not match authenticated node %q", declaredName, baseNodeName)
		}
		resolvedInstance = declaredName
	}

	return resolvedInstance, nil
}

// authenticateToken verifies the request using pre-shared bearer tokens and source address matching over TLS.
func (a *HubAuthenticator) authenticateToken(r *http.Request) (string, *HubAuthError) {
	if a.Token.RequireTLS && r.TLS == nil {
		return "", opErr(HubAuthMissingCredentials, "token authentication strictly requires TLS")
	}

	authHeader := r.Header.Get("Authorization")
	instanceName := strings.TrimSpace(r.Header.Get("X-Instance-Name"))
	if authHeader == "" || instanceName == "" {
		return "", opErr(HubAuthMissingCredentials, "Authorization header and X-Instance-Name header are required")
	}

	const bearerPrefix = "Bearer "
	if !strings.HasPrefix(authHeader, bearerPrefix) {
		return "", opErr(HubAuthMissingCredentials, "Authorization header must use Bearer scheme")
	}
	token := strings.TrimSpace(strings.TrimPrefix(authHeader, bearerPrefix))
	if token == "" {
		return "", opErr(HubAuthMissingCredentials, "bearer token cannot be empty")
	}

	expectedToken, ok := a.Token.Tokens[instanceName]
	if !ok {
		return "", opErr(HubAuthUnknownNode, "instance %q is not in allowed token instances", instanceName)
	}

	if subtle.ConstantTimeCompare([]byte(token), []byte(expectedToken)) != 1 {
		return "", opErr(HubAuthWrongToken, "invalid bearer token for instance %q", instanceName)
	}

	// Verify client source address if configured in AllowedAddrs (name=addr)
	if expectedAddr, hasAddr := a.Token.AllowedAddrs[instanceName]; hasAddr && expectedAddr != "" {
		remoteAddr := r.RemoteAddr
		if remoteAddr == "" {
			return "", opErr(HubAuthAddressMismatch, "client remote address is missing for address check")
		}
		clientIP, _, err := net.SplitHostPort(remoteAddr)
		if err != nil {
			clientIP = remoteAddr
		}
		if clientIP != expectedAddr {
			return "", opErr(HubAuthAddressMismatch, "source IP %q does not match configured address %q for instance %q", clientIP, expectedAddr, instanceName)
		}
	}

	return instanceName, nil
}

// NewPinnedTLSConfig creates a *tls.Config that verifies the server presents a certificate
// whose SHA-256 fingerprint matches expectedFingerprintHex.
// The fingerprint is case-insensitive and may optionally include colon separators.
func NewPinnedTLSConfig(expectedFingerprintHex string) (*tls.Config, error) {
	cleaned := strings.ReplaceAll(expectedFingerprintHex, ":", "")
	cleaned = strings.TrimSpace(cleaned)
	if len(cleaned) != 64 {
		return nil, fmt.Errorf("invalid SHA-256 certificate fingerprint: expected 64 hex characters, got %d", len(cleaned))
	}
	expectedBytes, err := hex.DecodeString(cleaned)
	if err != nil {
		return nil, fmt.Errorf("invalid hex in certificate fingerprint: %w", err)
	}

	return &tls.Config{
		MinVersion:         tls.VersionTLS12,
		InsecureSkipVerify: true, //nolint:gosec // G402: pinned leaf certificate SHA-256 fingerprint verified explicitly in VerifyConnection
		VerifyConnection: func(cs tls.ConnectionState) error {
			if len(cs.PeerCertificates) == 0 {
				return errors.New("server presented no certificates")
			}
			leafHash := sha256.Sum256(cs.PeerCertificates[0].Raw)
			if subtle.ConstantTimeCompare(leafHash[:], expectedBytes) != 1 {
				return fmt.Errorf("certificate fingerprint mismatch: server presented SHA-256 %x, expected %s", leafHash, cleaned)
			}
			return nil
		},
	}, nil
}
