package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"sync"
	"syscall"
	"time"
)

// BusConn represents a connection to an agentbus subprocess via MCP JSON-RPC protocol.
type BusConn struct {
	cmd    *exec.Cmd
	stdin  io.WriteCloser
	stdout *bufio.Scanner
	mutex  sync.Mutex
	id     int64 // request ID counter
}

// Message represents a bus message.
type Message struct {
	From string `json:"from"`
	To   string `json:"to"`
	Kind string `json:"kind"`
	Text string `json:"text"`
	ID   string `json:"id,omitempty"`
}

// NewBus creates a new connection to agentbus subprocess.
// binPath is the path to the agentbus binary (e.g., "agentbus" or "/path/to/agentbus").
// args are additional arguments to pass to agentbus (e.g., "serve", "--dir", "/home/agents/bus").
func NewBus(ctx context.Context, binPath string, args ...string) (*BusConn, error) {
	// Check if context is already cancelled
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	default:
	}

	// Build command with subprocess group for proper cleanup
	cmd := exec.CommandContext(ctx, binPath, args...)

	// Set process group so we can clean up all descendants
	cmd.SysProcAttr = &syscall.SysProcAttr{
		Setpgid: true,
	}

	// Connect stdout for reading responses
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return nil, fmt.Errorf("failed to create stdout pipe: %w", err)
	}

	// Connect stdin for writing requests
	stdin, err := cmd.StdinPipe()
	if err != nil {
		stdout.Close()
		return nil, fmt.Errorf("failed to create stdin pipe: %w", err)
	}

	// Redirect stderr to os.Stderr for diagnostics
	cmd.Stderr = os.Stderr

	// Start the subprocess
	if err := cmd.Start(); err != nil {
		stdin.Close()
		stdout.Close()
		return nil, fmt.Errorf("failed to start agentbus: %w", err)
	}

	// Create scanner with larger buffer for JSON-RPC responses
	scanner := bufio.NewScanner(stdout)
	// Increase token limit from default 64KB to 1MB for large responses
	scanner.Buffer(make([]byte, 64*1024), 1024*1024)

	bus := &BusConn{
		cmd:    cmd,
		stdin:  stdin,
		stdout: scanner,
		id:     1,
	}

	// Start goroutine to monitor subprocess context cancellation
	go func() {
		<-ctx.Done()
		bus.cleanup()
	}()

	return bus, nil
}

// Call invokes an MCP tool on the bus and returns the result.
func (b *BusConn) Call(method string, args interface{}) (interface{}, error) {
	b.mutex.Lock()
	id := b.id
	b.id++
	b.mutex.Unlock()

	// Build MCP JSON-RPC request
	req := map[string]interface{}{
		"jsonrpc": "2.0",
		"method":  method,
		"params":  args,
		"id":      id,
	}

	// Marshal and send request
	reqBytes, err := json.Marshal(req)
	if err != nil {
		return nil, fmt.Errorf("failed to marshal request: %w", err)
	}

	// Write request with newline (JSON-RPC expects line-delimited)
	if _, err := b.stdin.Write(append(reqBytes, '\n')); err != nil {
		return nil, fmt.Errorf("failed to write request: %w", err)
	}

	// Read response until we get one with matching ID
	for b.stdout.Scan() {
		var resp map[string]interface{}
		if err := json.Unmarshal(b.stdout.Bytes(), &resp); err != nil {
			continue // Skip malformed responses
		}

		// Check if this is the response for our request
		respID, ok := resp["id"]
		if !ok {
			continue // Skip responses without ID
		}

		respIDNum, ok := respID.(float64)
		if !ok || respIDNum != float64(id) {
			continue // Not our response
		}

		// Check for error
		if errObj, ok := resp["error"]; ok {
			return nil, fmt.Errorf("tool error: %v", errObj)
		}

		// Return result
		if result, ok := resp["result"]; ok {
			return result, nil
		}

		// No result or error found
		return nil, fmt.Errorf("invalid response format")
	}

	// Scanner error or EOF
	if err := b.stdout.Err(); err != nil {
		return nil, fmt.Errorf("stdout read error: %w", err)
	}

	return nil, fmt.Errorf("subprocess connection closed")
}

// PostMessage posts a message to the bus.
func (b *BusConn) PostMessage(from, to, kind, text string) error {
	params := map[string]interface{}{
		"from": from,
		"to":   to,
		"kind": kind,
		"text": text,
	}

	_, err := b.Call("message_post", params)
	return err
}

// ReadMessages reads messages from the bus (not yet implemented).
func (b *BusConn) ReadMessages(ctx context.Context) ([]Message, error) {
	// Stub implementation - would implement message polling from bus
	return nil, fmt.Errorf("ReadMessages not implemented")
}

// Close closes the bus connection and cleans up the subprocess.
func (b *BusConn) Close() error {
	return b.cleanup()
}

// cleanup terminates the subprocess and closes pipes.
func (b *BusConn) cleanup() error {
	var err error

	// Close stdin to signal subprocess to exit
	if b.stdin != nil {
		if closeErr := b.stdin.Close(); closeErr != nil && err == nil {
			err = closeErr
		}
	}

	// Terminate the subprocess
	if b.cmd != nil && b.cmd.ProcessState == nil {
		// Send SIGTERM to process group
		if b.cmd.Process != nil {
			// Kill the entire process group to clean up descendants
			if pgid, _ := syscall.Getpgid(b.cmd.Process.Pid); pgid > 0 {
				_ = syscall.Kill(-pgid, syscall.SIGTERM)
			}

			// Wait for graceful termination with timeout
			done := make(chan error, 1)
			go func() {
				done <- b.cmd.Wait()
			}()

			select {
			case <-done:
				// Process exited gracefully
			case <-time.After(1 * time.Second):
				// Force kill after timeout
				if b.cmd.Process != nil {
					if pgid, _ := syscall.Getpgid(b.cmd.Process.Pid); pgid > 0 {
						_ = syscall.Kill(-pgid, syscall.SIGKILL)
					}
					b.cmd.Wait()
				}
			}
		}
	}

	return err
}
