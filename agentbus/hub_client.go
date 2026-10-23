// Hub clients forward operations without reading or writing local bus state.
package main

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

const (
	// hubResponseLimit bounds large notebook and transcript responses.
	hubResponseLimit = 64 << 20
	// hubPendingLimit includes the existing 600-second handler wait and retry allowance.
	hubPendingLimit = 630 * time.Second
	// hubRetryInterval avoids busy looping while a lead instance is unavailable.
	hubRetryInterval = 250 * time.Millisecond
)

// HubClient owns transport settings; configure it before making concurrent calls.
type HubClient struct {
	URL             string             // lead instance origin, without endpoint path
	HTTP            *http.Client       // authenticated transport; redirects are rejected regardless of its policy
	Instance        string             // local authenticated instance name
	Token           string             // bearer token in token mode; empty in Tailscale mode
	RetryDuration   time.Duration      // zero uses the approved 30-second outage budget
	PendingDuration time.Duration      // zero uses the 630-second overall operation cap
	Local           *Hub               // trusted in-process dispatcher on the lead instance, when present
	Observe         func(reached bool) // optional; called after every transport attempt, before any retry wait, with false when the hub did not answer, so an outage starts at its first failed attempt
}

// NewRequest fixes the ID and timestamp once; every reattachment uses the same envelope.
func (c *HubClient) NewRequest(caller HubCaller, kind HubCallKind, op string, args map[string]any) (HubRequest, *HubClientError) {
	var id [16]byte
	if _, err := rand.Read(id[:]); err != nil {
		return HubRequest{}, fromStorage(HubClientStorage, storageErr(err, "cannot generate hub request ID"))
	}
	req := HubRequest{Version: hubProtocolVersion, RequestID: hex.EncodeToString(id[:]), IssuedAt: Timestamp{time.Now().UTC()}, Caller: caller, Kind: kind, Op: op, Args: args}
	if kind == HubToolCall {
		req.Tool = op
	}
	return req, nil
}

// Send retries only transport failures and Unavailable admissions, retaining the request ID and payload.
// A pending response reattaches to the accepted operation and resets the contiguous outage budget.
func (c *HubClient) Send(ctx context.Context, req HubRequest) (HubResponse, *HubClientError) {
	if err := c.validate(); err != nil {
		return HubResponse{}, err
	}
	data, err := json.Marshal(req)
	if err != nil || len(data) > hubBodyLimit {
		return HubResponse{}, opErr(HubClientMalformed, "hub request is not encodable or exceeds %d bytes", hubBodyLimit)
	}
	// Freeze local input too, so later caller mutations cannot change retry content.
	var frozen HubRequest
	if err := json.Unmarshal(data, &frozen); err != nil {
		return HubResponse{}, opErr(HubClientMalformed, "cannot freeze hub request: %v", err)
	}
	req = frozen
	pendingLimit := c.PendingDuration
	if pendingLimit == 0 {
		pendingLimit = hubPendingLimit
	}
	ctx, cancel := context.WithTimeout(ctx, pendingLimit)
	defer cancel()
	retry := c.RetryDuration
	if retry == 0 {
		retry = 30 * time.Second
	}
	outageDeadline := time.Now().Add(retry)
	backoff := hubRetryInterval
	// Each cryptographically random request ID supplies a distinct retry phase without extra fallible I/O.
	jitter := sha256.Sum256([]byte(req.RequestID))
	for {
		attempt, stop := context.WithDeadline(ctx, outageDeadline)
		response, transportErr := c.once(attempt, req, data)
		stop()
		if c.Observe != nil {
			c.Observe(transportErr == nil || transportErr.Cause != HubClientUnavailable)
		}
		if transportErr != nil && transportErr.Cause != HubClientUnavailable {
			return HubResponse{}, transportErr
		}
		if transportErr == nil {
			if response.Pending != nil {
				outageDeadline = time.Now().Add(retry)
				backoff = hubRetryInterval
			} else if response.Error == nil || !hubWireUnavailable(response.Error) {
				return response, nil
			}
		}
		if ctx.Err() != nil || !time.Now().Before(outageDeadline) {
			return HubResponse{}, opErr(HubClientUnavailable, "hub did not return a result within the retry or operation deadline; retain request %s for reconciliation. Before resending a mutation, check its result with the operation's read tool: a new MCP call creates a new request ID and may repeat the write", req.RequestID)
		}
		// Jitter avoids synchronized retries after a shared outage; no security decision uses it.
		delay := backoff + time.Duration(jitter[0]%100)*time.Millisecond
		if backoff < time.Second {
			backoff *= 2
		}
		if response.Pending != nil && response.Pending.PollAfterMS > 0 {
			delay = time.Duration(response.Pending.PollAfterMS) * time.Millisecond
		}
		if remaining := time.Until(outageDeadline); remaining < delay {
			delay = remaining
		}
		timer := time.NewTimer(delay)
		select {
		case <-ctx.Done():
			timer.Stop()
			return HubResponse{}, opErr(HubClientUnavailable, "hub call interrupted; retain request %s for reconciliation. Before resending a mutation, check its result with the operation's read tool: a new MCP call creates a new request ID and may repeat the write", req.RequestID)
		case <-timer.C:
		}
	}
}

// validate fails closed before transmitting secrets or accepting impossible timeout settings.
func (c *HubClient) validate() *HubClientError {
	if c.Instance == "" || c.RetryDuration < 0 || c.PendingDuration < 0 {
		return opErr(HubClientInvalidConfig, "instance name and nonnegative hub timeouts are required")
	}
	if c.Local != nil {
		return nil
	}
	u, err := url.Parse(c.URL)
	if err != nil || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || (u.Path != "" && u.Path != "/") ||
		(u.Scheme != "http" && u.Scheme != "https") || (c.Token != "" && u.Scheme != "https") {
		return opErr(HubClientInvalidConfig, "lead instance must be an HTTP origin; token authentication requires HTTPS")
	}
	return nil
}

// once performs one bounded exchange; it never follows redirects or changes the request envelope.
func (c *HubClient) once(ctx context.Context, req HubRequest, data []byte) (HubResponse, *HubClientError) {
	if c.Local != nil {
		response, status := c.Local.Execute(ctx, c.Instance, req)
		return validateHubResponse(req, response, status)
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, strings.TrimRight(c.URL, "/")+"/v1/tools/"+url.PathEscape(req.Op), bytes.NewReader(data))
	if err != nil {
		return HubResponse{}, opErr(HubClientInvalidConfig, "cannot construct hub request: %v", err)
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("X-Instance-Name", c.Instance)
	if c.Token != "" {
		request.Header.Set("Authorization", "Bearer "+c.Token)
	}
	client := http.Client{}
	if c.HTTP != nil {
		client = *c.HTTP
	}
	client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	response, err := client.Do(request)
	if err != nil {
		var invalid *HubCertificateError
		if errors.As(err, &invalid) {
			return HubResponse{}, opErr(HubClientInvalidConfig, "hub TLS verification failed: %s", invalid.Value.Message)
		}
		return HubResponse{}, opErr(HubClientUnavailable, "hub transport failed")
	}
	defer func() {
		if err := response.Body.Close(); err != nil {
			warnSideEffect("close hub response", storageErr(err, "cannot close hub response body"))
		}
	}()
	if response.StatusCode >= 300 && response.StatusCode < 400 {
		return HubResponse{}, opErr(HubClientMalformed, "hub redirects are forbidden")
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, hubResponseLimit+1))
	if err != nil {
		return HubResponse{}, opErr(HubClientUnavailable, "hub response was interrupted")
	}
	if len(body) > hubResponseLimit {
		return HubResponse{}, opErr(HubClientMalformed, "hub response exceeds %d bytes", hubResponseLimit)
	}
	var out HubResponse
	dec := json.NewDecoder(bytes.NewReader(body))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&out); err != nil {
		return HubResponse{}, opErr(HubClientMalformed, "invalid hub response JSON")
	}
	var extra any
	if err := dec.Decode(&extra); !errors.Is(err, io.EOF) {
		return HubResponse{}, opErr(HubClientMalformed, "hub response must contain one JSON object")
	}
	return validateHubResponse(req, out, response.StatusCode)
}

// validateHubResponse enforces the envelope before any caller treats it as a completed mutation.
func validateHubResponse(req HubRequest, response HubResponse, status int) (HubResponse, *HubClientError) {
	count := 0
	for _, present := range []bool{response.Result != nil, response.Stdout != nil, response.Error != nil, response.Pending != nil} {
		if present {
			count++
		}
	}
	if response.Version != hubProtocolVersion || count != 1 || response.UnreadCount < 0 ||
		(response.RequestID != req.RequestID && (response.RequestID != "" || response.Error == nil || (response.Error.Op != "hub_auth" && response.Error.Op != "hub"))) ||
		(response.Pending != nil && (status != http.StatusOK || response.Pending.PollAfterMS < 0 || response.Pending.PollAfterMS > 25000)) ||
		(response.Result != nil && req.Kind != HubToolCall) || (response.Stdout != nil && req.Kind != HubCLICall) ||
		(status != http.StatusOK && response.Error == nil) {
		return HubResponse{}, opErr(HubClientMalformed, "inconsistent hub response envelope")
	}
	if response.Error != nil {
		if response.Error.Op != req.Op && response.Error.Op != "hub" && response.Error.Op != "hub_auth" && response.Error.Op != "tools/call" {
			return HubResponse{}, opErr(HubClientMalformed, "hub error names a different operation")
		}
		if ok, _ := decodeHubWireError(req.Kind, response.Error); !ok {
			return HubResponse{}, opErr(HubClientMalformed, "unknown hub operation or error cause")
		}
	}
	return response, nil
}

// Call is the forwarding MCP dispatcher boundary; its error contains a generated operation error.
// Transport exhaustion is rebuilt as the original operation's Unavailable cause.
func (c *HubClient) Call(ctx context.Context, req HubRequest) (HubResponse, error) {
	response, err := c.Send(ctx, req)
	if err != nil {
		if err.Cause == HubClientUnavailable {
			if ok, operationErr := decodeHubWireError(req.Kind, &HubWireError{Op: req.Op, Cause: "Unavailable", Message: err.Message}); ok {
				return HubResponse{}, operationErr
			}
		}
		return HubResponse{}, err
	}
	if response.Error != nil {
		ok, operationErr := decodeHubWireError(req.Kind, response.Error)
		if !ok {
			return HubResponse{}, opErr(HubClientMalformed, "unknown hub error")
		}
		return response, operationErr
	}
	return response, nil
}

// decodeHubWireError is the forwarding dispatcher boundary; its error contains the exact operation type.
func decodeHubWireError(kind HubCallKind, wire *HubWireError) (bool, error) {
	if wire.Op == "release" {
		switch kind {
		case HubToolCall:
			if cause, ok := ReleaseCauseByName(wire.Cause); ok {
				return true, opErr(cause, "%s", wire.Message)
			}
		case HubCLICall:
			if cause, ok := CmdReleaseCauseByName(wire.Cause); ok {
				return true, opErr(cause, "%s", wire.Message)
			}
		}
		return false, nil
	}
	err, ok := wireOperationError(wire.Op, wire.Cause, wire.Message)
	return ok, err
}

// hubWireUnavailable recognizes only locally generated error classes, never peer-supplied class names.
func hubWireUnavailable(wire *HubWireError) bool {
	err, ok := wireOperationError(wire.Op, wire.Cause, wire.Message)
	return ok && errors.Is(err, ErrUnavailable)
}
