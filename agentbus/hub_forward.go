// Forwarded MCP dispatch keeps local-file mode and the existing stdio protocol intact.
package main

import (
	"context"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// hubClientFromEnv configures forwarding using the default bus directory.
// Bash loads agents.conf and secrets.env; the bus never executes configuration as shell code.
func hubClientFromEnv() (*HubClient, *HubClientError) {
	return hubClientForDir(defaultDir())
}

// hubClientForDir respects serve's --dir when locating the default lead-local socket.
func hubClientForDir(dir string) (*HubClient, *HubClientError) {
	lead := os.Getenv("LEAD_INSTANCE")
	socket := os.Getenv("HUB_SOCKET")
	local := os.Getenv("HUB_LISTEN") != "" || socket != ""
	if lead == "" && !local {
		return nil, nil
	}
	mode := os.Getenv("HUB_AUTH")
	if mode == "" {
		mode = string(HubAuthModeTailscale)
	}
	if mode != string(HubAuthModeTailscale) && mode != string(HubAuthModeToken) {
		return nil, opErr(HubClientInvalidConfig, "HUB_AUTH must be tailscale or token")
	}
	seconds := 30
	if raw := os.Getenv("HUB_RETRY_SECONDS"); raw != "" {
		value, err := strconv.Atoi(raw)
		if err != nil || value <= 0 || value > 600 {
			return nil, opErr(HubClientInvalidConfig, "HUB_RETRY_SECONDS must be between 1 and 600")
		}
		seconds = value
	}
	if local {
		if socket == "" {
			path, err := filepath.Abs(filepath.Join(dir, "hub.sock"))
			if err != nil {
				return nil, opErr(HubClientInvalidConfig, "cannot resolve default hub socket: %v", err)
			}
			socket = path
		}
		client, err := newHubSocketClient(socket, os.Getenv("INSTANCE_NAME"))
		if err != nil {
			return nil, err
		}
		client.RetryDuration = time.Duration(seconds) * time.Second
		return client, nil
	}
	transport := &http.Transport{DialContext: (&net.Dialer{Timeout: 5 * time.Second}).DialContext,
		TLSHandshakeTimeout: 5 * time.Second, ResponseHeaderTimeout: 30 * time.Second, IdleConnTimeout: 60 * time.Second}
	client := &HubClient{URL: lead, Instance: os.Getenv("INSTANCE_NAME"), RetryDuration: time.Duration(seconds) * time.Second,
		HTTP: &http.Client{Transport: transport}}
	scheme := "http://"
	if mode == string(HubAuthModeToken) {
		scheme = "https://"
		client.Token = os.Getenv("HUB_TOKEN")
		if client.Token == "" {
			return nil, opErr(HubClientInvalidConfig, "token mode requires HUB_TOKEN from secrets.env")
		}
		fingerprint := os.Getenv("HUB_CERT_SHA256")
		if fingerprint == "" {
			fingerprint = os.Getenv("HUB_CERT_FINGERPRINT")
		}
		tlsConfig, err := NewPinnedTLSConfig(fingerprint)
		if err != nil {
			return nil, opErr(HubClientInvalidConfig, "invalid HUB_CERT_SHA256/HUB_CERT_FINGERPRINT: %v", err)
		}
		transport.TLSClientConfig = tlsConfig
	}
	if !strings.Contains(client.URL, "://") {
		client.URL = scheme + client.URL
	}
	if err := client.validate(); err != nil {
		return nil, err
	}
	return client, nil
}

// callForMCP is the MCP dispatcher boundary; its error is a concrete generated operation error.
// The hub's unread count is returned separately so failures retain their exact typed errors.
func (s *Server) callForMCP(name string, args map[string]any) (string, int, error) {
	if s.hubClient == nil {
		text, err := s.call(name, args)
		return text, 0, err
	}
	req, requestErr := s.hubClient.NewRequest(s.hubCaller, HubToolCall, name, args)
	if requestErr != nil {
		return "", 0, requestErr
	}
	response, err := s.hubClient.Call(context.Background(), req)
	if serr := recordHubReachability(s.home, hubReached(err), time.Now()); serr != nil {
		warnSideEffect("record hub reachability", serr, "instance", s.instance)
	}
	text := ""
	if response.Result != nil {
		text = *response.Result
	}
	return text, response.UnreadCount, err
}

// forwardCLI forwards an allowlisted CLI operation to the hub if configured.
// It writes the hub's stdout response directly to os.Stdout.
// Returns (true, err) if forwarded, or (false, nil) if single-instance local mode applies.
func forwardCLI(dir, op string, args map[string]any) (bool, error) {
	client, err := hubClientForDir(dir)
	if err != nil {
		return true, err
	}
	if client == nil {
		return false, nil
	}
	caller := HubCaller{
		Agent:    "cli",
		Instance: client.Instance,
		Task:     "cli",
		Role:     RoleAgent,
		Tier:     TierNormal,
	}
	req, reqErr := client.NewRequest(caller, HubCLICall, op, args)
	if reqErr != nil {
		return true, reqErr
	}
	resp, callErr := client.Call(context.Background(), req)
	if callErr != nil {
		return true, callErr
	}
	if resp.Stdout != nil {
		if _, writeErr := os.Stdout.Write(*resp.Stdout); writeErr != nil {
			if ok, outputErr := decodeHubWireError(HubCLICall, &HubWireError{Op: op, Cause: "Storage", Message: "cannot write forwarded stdout: " + writeErr.Error()}); ok {
				return true, outputErr
			}
			return true, fromStorage(HubClientStorage, storageErr(writeErr, "cannot write forwarded stdout"))
		}
	}
	return true, nil
}
