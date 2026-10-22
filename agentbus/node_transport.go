// Node transport uses the hub's configured authentication without executing remote code locally.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"
	"time"
)

// nodeExchange sends one bounded control-plane request; the caller retains payloads across retry.
func nodeExchange(ctx context.Context, client *HubClient, path string, input, output any) *CmdNodeError {
	if client == nil {
		return opErr(CmdNodeInvalidConfig, "node requires a configured hub client")
	}
	if err := client.validate(); err != nil {
		return opErr(CmdNodeInvalidConfig, "%s", err.Message)
	}
	data, err := json.Marshal(input)
	if err != nil || len(data) > hubBodyLimit {
		return opErr(CmdNodeProtocol, "node request cannot be encoded within the request limit")
	}
	budget := client.RetryDuration
	if budget <= 0 {
		budget = 30 * time.Second
	}
	limit := budget
	if path == "/v1/commands/poll" {
		limit += hubPollDuration
	}
	bounded, cancel := context.WithTimeout(ctx, limit)
	defer cancel()
	request, err := http.NewRequestWithContext(bounded, http.MethodPost, strings.TrimRight(client.URL, "/")+path, bytes.NewReader(data))
	if err != nil {
		return opErr(CmdNodeInvalidConfig, "cannot construct node request: %v", err)
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("X-Instance-Name", client.Instance)
	if client.Token != "" {
		request.Header.Set("Authorization", "Bearer "+client.Token)
	}
	transport := http.Client{}
	if client.HTTP != nil {
		transport = *client.HTTP
	}
	transport.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	response, err := transport.Do(request)
	if err != nil {
		var invalid *HubCertificateError
		if errors.As(err, &invalid) {
			return opErr(CmdNodeInvalidConfig, "hub TLS verification failed: %s", invalid.Value.Message)
		}
		return opErr(CmdNodeHubUnavailable, "node cannot reach the hub")
	}
	defer func() {
		if err := response.Body.Close(); err != nil {
			warnSideEffect("close node response", storageErr(err, "cannot close node response"))
		}
	}()
	if response.StatusCode == http.StatusNoContent && path == "/v1/commands/poll" {
		return nil
	}
	if response.StatusCode >= 300 && response.StatusCode < 400 {
		return opErr(CmdNodeProtocol, "hub redirects are forbidden")
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, hubBodyLimit+1))
	if err != nil {
		return opErr(CmdNodeHubUnavailable, "node response was interrupted")
	}
	if len(body) > hubBodyLimit {
		return opErr(CmdNodeProtocol, "node response exceeds the size limit")
	}
	if response.StatusCode != http.StatusOK {
		var failure HubResponse
		if err := json.Unmarshal(body, &failure); err != nil || failure.Version != hubProtocolVersion || failure.Error == nil {
			if response.StatusCode == http.StatusTooManyRequests || response.StatusCode >= http.StatusInternalServerError {
				return opErr(CmdNodeHubUnavailable, "hub temporarily returned an unavailable control endpoint")
			}
			return opErr(CmdNodeProtocol, "hub rejection has no valid typed error")
		}
		if failure.Error.Op != nodeEndpointOperation(path) && failure.Error.Op != "hub_auth" {
			return opErr(CmdNodeProtocol, "hub error names an unexpected operation")
		}
		remote, ok := wireOperationError(failure.Error.Op, failure.Error.Cause, failure.Error.Message)
		if !ok {
			return opErr(CmdNodeProtocol, "hub error has an unknown operation or cause")
		}
		cause := CmdNodeRejected
		switch {
		case errors.Is(remote, ErrStorage), errors.Is(remote, ErrProcess), errors.Is(remote, ErrUnavailable):
			cause = CmdNodeHubUnavailable
		case nodeLeaseFailure(failure.Error):
			cause = CmdNodeLeaseLost
		}
		result := opErr(cause, "hub rejected %s: %s", path, failure.Error.Message)
		result.Err = errors.Join(result.Err, remote)
		return result
	}
	dec := json.NewDecoder(bytes.NewReader(body))
	dec.DisallowUnknownFields()
	if err := dec.Decode(output); err != nil {
		return opErr(CmdNodeProtocol, "cannot decode node response: %v", err)
	}
	var extra any
	if err := dec.Decode(&extra); !errors.Is(err, io.EOF) {
		return opErr(CmdNodeProtocol, "node response must contain one object")
	}
	return nil
}

// nodeEndpointOperation is a fixed allowlist, not a caller-selected RPC name.
func nodeEndpointOperation(path string) string {
	switch path {
	case "/v1/instances/register":
		return "instance_register"
	case "/v1/instances/heartbeat":
		return "instance_heartbeat"
	case "/v1/commands/poll":
		return "command_poll"
	case "/v1/commands/ack":
		return "command_ack"
	default:
		return ""
	}
}

// nodeLeaseFailure recognizes only explicit lease failures; a conflicting ack outcome is never retried as renewal.
func nodeLeaseFailure(wire *HubWireError) bool {
	switch wire.Op {
	case "instance_heartbeat":
		return wire.Cause == InstanceHeartbeatLeaseExpired.String() || wire.Cause == InstanceHeartbeatStaleLease.String() || wire.Cause == InstanceHeartbeatUnknownInstance.String()
	case "command_poll":
		return wire.Cause == CommandPollLeaseStale.String()
	case "command_ack":
		return wire.Cause == CommandAckLeaseStale.String()
	default:
		return false
	}
}
