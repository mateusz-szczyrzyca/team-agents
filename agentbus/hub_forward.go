// Forwarded MCP dispatch keeps local-file mode and the existing stdio protocol intact.
package main

import (
	"context"
	"net"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"
)

// hubClientFromEnv configures remote forwarding only when LEAD_INSTANCE is set.
// Bash loads agents.conf and secrets.env; the bus never executes configuration as shell code.
func hubClientFromEnv() (*HubClient, *HubClientError) {
	lead := os.Getenv("LEAD_INSTANCE")
	if os.Getenv("HUB_LISTEN") != "" {
		logWarn(LogLeadLocalFallback, "reason", "lead-local Unix socket transport is not yet available; using local bus files")
		return nil, nil
	}
	if lead == "" {
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
		tlsConfig, err := NewPinnedTLSConfig(os.Getenv("HUB_CERT_FINGERPRINT"))
		if err != nil {
			return nil, opErr(HubClientInvalidConfig, "invalid HUB_CERT_FINGERPRINT: %v", err)
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
	text := ""
	if response.Result != nil {
		text = *response.Result
	}
	return text, response.UnreadCount, err
}
