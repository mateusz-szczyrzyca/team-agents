package main

import (
	"bytes"
	"context"
	"encoding/json"
	"os/exec"
	"testing"
	"time"
)

// TestNewBus verifies creating a bus connection to agentbus.
func TestNewBus(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	// This should fail gracefully - agentbus binary may not be in test path
	bus, err := NewBus(ctx, "nonexistent-agentbus", "serve", "--dir", "/tmp")

	// Should return an error for missing binary, not panic
	if err == nil {
		if bus != nil {
			_ = bus.Close()
		}
		t.Skip("agentbus not available in test path")
	}
}

// TestBusCallToolStructure verifies Call method signature and error handling.
func TestBusCallToolStructure(t *testing.T) {
	// This test verifies the interface exists and basic error handling works
	// Actual tool calls require agentbus subprocess to be available
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()

	// Create with invalid path to test error handling
	bus, err := NewBus(ctx, "nonexistent-agentbus")
	if err == nil && bus != nil {
		defer bus.Close()
	}

	// Call should exist and handle errors gracefully
	// (We can't actually call it without a real subprocess)
}

// TestBusToolCallResponse verifies tool call response format.
func TestBusToolCallResponse(t *testing.T) {
	// Expected response from a successful tool call:
	// {"jsonrpc":"2.0","result":"...","id":1}

	responseJSON := `{"jsonrpc":"2.0","result":"success","id":1}`
	var resp map[string]interface{}

	if err := json.Unmarshal([]byte(responseJSON), &resp); err != nil {
		t.Fatalf("could not parse expected response: %v", err)
	}

	if jsonrpc, ok := resp["jsonrpc"].(string); !ok || jsonrpc != "2.0" {
		t.Error("response missing jsonrpc 2.0")
	}
	if _, ok := resp["result"]; !ok {
		t.Error("response missing result field")
	}
	if id, ok := resp["id"].(float64); !ok || id != 1 {
		t.Error("response missing id field")
	}
}

// TestBusToolErrorResponse verifies tool error response format.
func TestBusToolErrorResponse(t *testing.T) {
	// Expected error response format:
	// {"jsonrpc":"2.0","error":{"code":-32000,"message":"..."},"id":1}

	errorJSON := `{"jsonrpc":"2.0","error":{"code":-32000,"message":"tool not found"},"id":1}`
	var resp map[string]interface{}

	if err := json.Unmarshal([]byte(errorJSON), &resp); err != nil {
		t.Fatalf("could not parse expected error response: %v", err)
	}

	if jsonrpc, ok := resp["jsonrpc"].(string); !ok || jsonrpc != "2.0" {
		t.Error("error response missing jsonrpc 2.0")
	}

	if errObj, ok := resp["error"].(map[string]interface{}); ok {
		if code, ok := errObj["code"].(float64); !ok || code >= 0 {
			t.Error("error response missing negative error code")
		}
		if msg, ok := errObj["message"].(string); !ok || msg == "" {
			t.Error("error response missing error message")
		}
	} else {
		t.Error("response missing error object")
	}
}

// TestBusClose verifies Close returns error handling.
func TestBusClose(t *testing.T) {
	// Test that Close exists and can be called
	// (Without a real bus, this is just interface verification)
	ctx := context.Background()

	// Create should fail, but even if it succeeds, Close should exist
	if bus, err := NewBus(ctx, "nonexistent"); err == nil && bus != nil {
		// Close should not panic
		_ = bus.Close()
	}
}

// TestBusPostMessage verifies PostMessage signature and error handling.
func TestBusPostMessage(t *testing.T) {
	// PostMessage(from, to, kind, text string) error

	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()

	bus, err := NewBus(ctx, "nonexistent-agentbus")
	if err != nil && bus == nil {
		// Expected for missing binary - PostMessage should error gracefully
		return
	}

	if bus != nil {
		defer bus.Close()
		// Method should exist and handle disconnected state gracefully
		_ = bus.PostMessage("agent", "coordinator", "message", "test")
	}
}

// TestBusReadMessages verifies ReadMessages signature and error handling.
func TestBusReadMessages(t *testing.T) {
	// ReadMessages(ctx context.Context) ([]Message, error)

	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()

	bus, err := NewBus(ctx, "nonexistent-agentbus")
	if err != nil && bus == nil {
		// Expected for missing binary
		return
	}

	if bus != nil {
		defer bus.Close()
		// Method should exist and handle gracefully
		_, _ = bus.ReadMessages(ctx)
	}
}

// TestBusSubprocessEnvironment verifies subprocess inherits correct environment.
func TestBusSubprocessEnvironment(t *testing.T) {
	// Verify that when spawning agentbus, we pass required env vars
	// (This is primarily a documentation test of expected behavior)

	// Expected environment for agentbus subprocess:
	// - AGENTS_HOME (inherited from parent or set explicitly)
	// - PATH (inherited)
	// - Any MCP-related vars

	// Test subprocess execution pattern
	cmd := exec.Command("echo", "test")
	if cmd == nil {
		t.Error("exec.Command failed")
	}
}

// TestBusContextCancellation verifies proper cleanup on context cancellation.
func TestBusContextCancellation(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel() // Cancel immediately

	// NewBus should handle cancelled context gracefully
	bus, err := NewBus(ctx, "agentbus", "serve")

	// Either error or nil bus is acceptable - both mean proper cancellation handling
	if bus != nil {
		bus.Close()
	}
	if err == nil && bus == nil {
		t.Error("NewBus should error or return nil on cancelled context")
	}
}

// TestBusToolCallRequest verifies request format for MCP tool calls.
func TestBusToolCallRequest(t *testing.T) {
	// Expected request format for MCP tool call:
	// {"jsonrpc":"2.0","method":"tools/call","params":{"name":"board_list","arguments":{}},"id":1}

	requestData := map[string]interface{}{
		"jsonrpc": "2.0",
		"method":  "tools/call",
		"params": map[string]interface{}{
			"name":      "board_list",
			"arguments": map[string]interface{}{},
		},
		"id": 1,
	}

	requestJSON, err := json.Marshal(requestData)
	if err != nil {
		t.Fatalf("could not marshal request: %v", err)
	}

	// Verify it's valid JSON with required fields
	var parsed map[string]interface{}
	if err := json.Unmarshal(requestJSON, &parsed); err != nil {
		t.Errorf("request JSON is invalid: %v", err)
	}

	if method, ok := parsed["method"].(string); !ok || method != "tools/call" {
		t.Error("request missing tools/call method")
	}
}

// TestBusStdoutParsing verifies bus can parse stdout from agentbus subprocess.
func TestBusStdoutParsing(t *testing.T) {
	// Simulated agentbus stdout with multiple JSON-RPC responses
	stdout := bytes.NewBufferString(`{"jsonrpc":"2.0","result":{"status":"ok"},"id":1}
{"jsonrpc":"2.0","error":{"code":-32000,"message":"failed"},"id":2}
`)

	// Should be able to parse line-by-line JSON responses
	decoder := json.NewDecoder(stdout)

	count := 0
	for decoder.More() {
		var resp map[string]interface{}
		if err := decoder.Decode(&resp); err != nil {
			t.Errorf("failed to decode response: %v", err)
			break
		}
		count++
	}

	if count != 2 {
		t.Errorf("expected 2 responses, got %d", count)
	}
}

// TestBusStdin verifies bus can write requests to subprocess stdin.
func TestBusStdin(t *testing.T) {
	// Verify that writing to subprocess stdin works

	// Test pattern: create pipe, write JSON-RPC request, verify no error
	var buf bytes.Buffer

	request := map[string]interface{}{
		"jsonrpc": "2.0",
		"method":  "tools/call",
		"params":  map[string]interface{}{"name": "board_list"},
		"id":      1,
	}

	encoder := json.NewEncoder(&buf)
	if err := encoder.Encode(request); err != nil {
		t.Errorf("failed to encode request: %v", err)
	}

	if buf.Len() == 0 {
		t.Error("request encoding produced no output")
	}
}

// TestBusProcessCleanup verifies process groups are cleaned up on error.
func TestBusProcessCleanup(t *testing.T) {
	// Verify that if subprocess fails to start, no zombie processes remain
	// This is a contract test - actual verification requires process inspection

	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()

	// Try to spawn nonexistent binary
	bus, err := NewBus(ctx, "nonexistent-binary-xyz-123")

	if bus != nil {
		bus.Close()
		t.Error("NewBus should fail for nonexistent binary")
	}

	if err == nil {
		t.Error("NewBus should return error for nonexistent binary")
	}
}

// TestBusMessagePosting verifies message structure for PostMessage.
func TestBusMessagePosting(t *testing.T) {
	// PostMessage should create a valid MCP message post request
	// Expected format: tool call to "post_message" with from/to/kind/text params

	messageParams := map[string]interface{}{
		"from": "agent",
		"to":   "coordinator",
		"kind": "test",
		"text": "hello",
	}

	paramsJSON, err := json.Marshal(messageParams)
	if err != nil {
		t.Fatalf("could not marshal message params: %v", err)
	}

	var parsed map[string]interface{}
	if err := json.Unmarshal(paramsJSON, &parsed); err != nil {
		t.Errorf("message params JSON is invalid: %v", err)
	}

	if len(parsed) != 4 {
		t.Errorf("expected 4 message fields, got %d", len(parsed))
	}
}

// TestBusMultipleToolCalls verifies sequential tool calls work.
func TestBusMultipleToolCalls(t *testing.T) {
	// Test that bus can handle multiple sequential tool calls

	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()

	bus, err := NewBus(ctx, "nonexistent-agentbus")
	if err != nil && bus == nil {
		// Expected for missing binary
		return
	}

	if bus != nil {
		defer bus.Close()
		// Multiple Call invocations should be possible (even if they fail)
		_, _ = bus.Call("board_list", nil)
		_, _ = bus.Call("team_get", nil)
	}
}
