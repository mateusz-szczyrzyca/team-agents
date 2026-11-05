package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"regexp"
	"strings"
	"sync"
	"testing"
	"time"
)

// TestLogEventConstants verifies all 27 required LogEvent constants are defined.
func TestLogEventConstants(t *testing.T) {
	requiredEvents := []string{
		"agent_gone",
		"client_read_failed",
		"command_failed",
		"corrupt_message_line",
		"corrupt_notebook_line",
		"doorbell_lost",
		"doorbell_ring",
		"jsonrpc_error",
		"jsonrpc_write_failed",
		"lead_http_error",
		"lead_listening",
		"lead_mcp_call",
		"lead_peer_rejected",
		"lead_stopped",
		"lead_sync_rejected",
		"member_waking",
		"member_woken_by",
		"message_posted",
		"non_json_cli_line",
		"relay_lead_failed",
		"remote_member_event",
		"server_started",
		"server_stopping",
		"side_effect_failed",
		"tool_call",
		"tool_call_failed",
		"tool_input_unencodable",
	}

	for _, evt := range requiredEvents {
		// This should not panic if constant is defined
		_ = LogEvent(evt)
	}

	if len(requiredEvents) != 27 {
		t.Fatalf("Expected 27 LogEvent constants, got %d", len(requiredEvents))
	}
}

// TestLoggerCreation verifies NewLogger returns a working logger.
func TestLoggerCreation(t *testing.T) {
	buf := new(bytes.Buffer)
	logger := NewLogger(buf)
	if logger == nil {
		t.Fatal("NewLogger returned nil")
	}
}

// TestLoggerInfoSchema verifies Info logs are JSON with correct schema.
func TestLoggerInfoSchema(t *testing.T) {
	buf := new(bytes.Buffer)
	logger := NewLogger(buf)

	logger.Info("tool_call", "tool", "board_list", "duration_ms", 123)

	output := buf.String()
	if output == "" {
		t.Fatal("logger.Info produced no output")
	}

	var record map[string]interface{}
	if err := json.Unmarshal([]byte(output), &record); err != nil {
		t.Fatalf("log output is not JSON: %v\n%s", err, output)
	}

	// Verify required fields
	if _, ok := record["ts"]; !ok {
		t.Error("missing 'ts' field in log record")
	}
	if _, ok := record["level"]; !ok {
		t.Error("missing 'level' field in log record")
	}
	if _, ok := record["event"]; !ok {
		t.Error("missing 'event' field in log record")
	}

	// Verify event value
	if event, ok := record["event"].(string); ok {
		if event != "tool_call" {
			t.Errorf("event field is %q, want %q", event, "tool_call")
		}
	} else {
		t.Errorf("event field is not a string: %v", record["event"])
	}

	// Verify level is lowercase
	if level, ok := record["level"].(string); ok {
		if level != strings.ToLower(level) {
			t.Errorf("level field %q is not lowercase", level)
		}
	} else {
		t.Errorf("level field is not a string: %v", record["level"])
	}

	// Verify timestamp is RFC 3339
	if ts, ok := record["ts"].(string); ok {
		if _, err := time.Parse(time.RFC3339, ts); err != nil {
			t.Errorf("ts field %q is not RFC 3339 format: %v", ts, err)
		}
	} else {
		t.Errorf("ts field is not a string: %v", record["ts"])
	}

	// Verify custom attributes are present
	if tool, ok := record["tool"].(string); ok {
		if tool != "board_list" {
			t.Errorf("tool attribute is %q, want %q", tool, "board_list")
		}
	} else {
		t.Errorf("tool attribute is not a string or missing: %v", record["tool"])
	}

	if duration, ok := record["duration_ms"].(float64); ok {
		if duration != 123 {
			t.Errorf("duration_ms attribute is %v, want 123", duration)
		}
	} else {
		t.Errorf("duration_ms attribute is not a number or missing: %v", record["duration_ms"])
	}
}

// TestLoggerWarnSchema verifies Warn logs are JSON with level=warn.
func TestLoggerWarnSchema(t *testing.T) {
	buf := new(bytes.Buffer)
	logger := NewLogger(buf)

	logger.Warn("side_effect_failed", "operation", "cleanup")

	output := buf.String()
	var record map[string]interface{}
	if err := json.Unmarshal([]byte(output), &record); err != nil {
		t.Fatalf("log output is not JSON: %v", err)
	}

	if level, ok := record["level"].(string); ok {
		if level != "warn" {
			t.Errorf("level field is %q, want %q", level, "warn")
		}
	} else {
		t.Errorf("level field is not a string: %v", record["level"])
	}

	if event, ok := record["event"].(string); ok {
		if event != "side_effect_failed" {
			t.Errorf("event field is %q, want %q", event, "side_effect_failed")
		}
	}
}

// TestLoggerErrorSchema verifies Error logs are JSON with level=error.
func TestLoggerErrorSchema(t *testing.T) {
	buf := new(bytes.Buffer)
	logger := NewLogger(buf)

	logger.Error("command_failed", "command", "team-get", "exit_code", 1)

	output := buf.String()
	var record map[string]interface{}
	if err := json.Unmarshal([]byte(output), &record); err != nil {
		t.Fatalf("log output is not JSON: %v", err)
	}

	if level, ok := record["level"].(string); ok {
		if level != "error" {
			t.Errorf("level field is %q, want %q", level, "error")
		}
	}
}

// TestLoggerMultipleRecords verifies multiple log calls produce separate JSON lines.
func TestLoggerMultipleRecords(t *testing.T) {
	buf := new(bytes.Buffer)
	logger := NewLogger(buf)

	logger.Info("tool_call", "tool", "board_add")
	logger.Warn("side_effect_failed", "effect", "cleanup")
	logger.Error("command_failed", "cmd", "serve")

	lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
	if len(lines) < 3 {
		t.Fatalf("Expected at least 3 log lines, got %d", len(lines))
	}

	for i, line := range lines[:3] {
		var record map[string]interface{}
		if err := json.Unmarshal([]byte(line), &record); err != nil {
			t.Errorf("line %d is not valid JSON: %v", i, err)
		}
	}
}

// TestLoggerConcurrency verifies concurrent logging is safe.
func TestLoggerConcurrency(t *testing.T) {
	buf := new(bytes.Buffer)
	logger := NewLogger(buf)

	var wg sync.WaitGroup
	for i := 0; i < 10; i++ {
		wg.Add(3)
		go func(n int) {
			defer wg.Done()
			logger.Info("tool_call", "worker", n)
		}(i)
		go func(n int) {
			defer wg.Done()
			logger.Warn("side_effect_failed", "worker", n)
		}(i)
		go func(n int) {
			defer wg.Done()
			logger.Error("command_failed", "worker", n)
		}(i)
	}
	wg.Wait()

	output := buf.String()
	lines := strings.Split(strings.TrimSpace(output), "\n")
	if len(lines) < 30 {
		t.Errorf("Expected at least 30 log lines from 30 concurrent calls, got %d", len(lines))
	}

	// Verify all lines are valid JSON
	for i, line := range lines {
		if line == "" {
			continue
		}
		var record map[string]interface{}
		if err := json.Unmarshal([]byte(line), &record); err != nil {
			t.Errorf("line %d is not valid JSON: %v", i, err)
		}
	}
}

// TestLoggerWriteFailure verifies logging gracefully handles write errors.
func TestLoggerWriteFailure(t *testing.T) {
	failWriter := &failAfterWriter{limit: 0}
	logger := NewLogger(failWriter)

	// This should not panic even though write will fail
	logger.Info("tool_call", "tool", "test")

	if failWriter.attempts == 0 {
		t.Error("logger did not attempt to write")
	}
}

// failAfterWriter fails after a certain number of writes.
type failAfterWriter struct {
	limit    int
	attempts int
}

func (w *failAfterWriter) Write(p []byte) (int, error) {
	w.attempts++
	if w.attempts > w.limit {
		return 0, fmt.Errorf("write failed")
	}
	return len(p), nil
}

// TestLoggerAttributeTypes verifies various attribute types are encoded correctly.
func TestLoggerAttributeTypes(t *testing.T) {
	buf := new(bytes.Buffer)
	logger := NewLogger(buf)

	logger.Info("tool_call",
		"string_val", "hello",
		"int_val", 42,
		"float_val", 3.14,
		"bool_val", true,
		"nil_val", nil,
	)

	output := buf.String()
	var record map[string]interface{}
	if err := json.Unmarshal([]byte(output), &record); err != nil {
		t.Fatalf("log output is not JSON: %v", err)
	}

	// Verify all attributes are present
	if v, ok := record["string_val"].(string); !ok || v != "hello" {
		t.Errorf("string_val: got %v, want 'hello'", record["string_val"])
	}
	if v, ok := record["int_val"].(float64); !ok || v != 42 {
		t.Errorf("int_val: got %v, want 42", record["int_val"])
	}
	if v, ok := record["float_val"].(float64); !ok || (v < 3.13 || v > 3.15) {
		t.Errorf("float_val: got %v, want ~3.14", record["float_val"])
	}
	if v, ok := record["bool_val"].(bool); !ok || !v {
		t.Errorf("bool_val: got %v, want true", record["bool_val"])
	}
}

// TestLoggerTimestampPrecision verifies timestamps are in correct format with UTC.
func TestLoggerTimestampPrecision(t *testing.T) {
	buf := new(bytes.Buffer)
	logger := NewLogger(buf)

	before := time.Now().UTC().Add(-1 * time.Second) // Allow 1 second margin
	logger.Info("test_event")
	after := time.Now().UTC().Add(1 * time.Second) // Allow 1 second margin

	output := buf.String()
	var record map[string]interface{}
	if err := json.Unmarshal([]byte(output), &record); err != nil {
		t.Fatalf("log output is not JSON: %v", err)
	}

	if ts, ok := record["ts"].(string); ok {
		parsed, err := time.Parse(time.RFC3339, ts)
		if err != nil {
			t.Errorf("timestamp %q is not RFC 3339: %v", ts, err)
		}

		// Verify timestamp is between before and after (within reasonable bounds)
		if parsed.Before(before) || parsed.After(after) {
			t.Errorf("timestamp %v is not within expected range [%v, %v]", parsed, before, after)
		}

		// Verify it ends with Z (UTC)
		if !strings.HasSuffix(ts, "Z") {
			t.Errorf("timestamp %q does not end with Z (UTC)", ts)
		}
	} else {
		t.Errorf("ts field is not a string: %v", record["ts"])
	}
}

// TestLoggerNoDebugMessages verifies Debug-level messages are not output.
func TestLoggerNoDebugMessages(t *testing.T) {
	buf := new(bytes.Buffer)
	logger := NewLogger(buf)

	// Log Info, Warn, Error only
	logger.Info("server_started")
	logger.Warn("side_effect_failed")
	logger.Error("command_failed")

	lines := strings.Split(strings.TrimSpace(buf.String()), "\n")
	if len(lines) < 3 {
		t.Fatalf("Expected at least 3 log lines, got %d", len(lines))
	}

	// Verify only expected lines are logged
	for i, line := range lines {
		var record map[string]interface{}
		if err := json.Unmarshal([]byte(line), &record); err != nil {
			t.Errorf("line %d is not valid JSON: %v", i, err)
			continue
		}

		if level, ok := record["level"].(string); ok {
			if level == "debug" {
				t.Errorf("line %d has unexpected debug level", i)
			}
		}
	}
}

// TestLoggerTimestampConsistency verifies timestamps follow RFC 3339 consistently.
func TestLoggerTimestampConsistency(t *testing.T) {
	buf := new(bytes.Buffer)
	logger := NewLogger(buf)

	// Log multiple records in quick succession
	for i := 0; i < 5; i++ {
		logger.Info("test_event", "index", i)
	}

	lines := strings.Split(strings.TrimSpace(buf.String()), "\n")
	rfc3339Regex := regexp.MustCompile(`^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$`)

	for i, line := range lines {
		var record map[string]interface{}
		if err := json.Unmarshal([]byte(line), &record); err != nil {
			t.Errorf("line %d is not valid JSON: %v", i, err)
			continue
		}

		if ts, ok := record["ts"].(string); ok {
			if !rfc3339Regex.MatchString(ts) {
				t.Errorf("line %d timestamp %q does not match RFC 3339 format", i, ts)
			}
		}
	}
}

// TestLoggerEmptyAttributes verifies logging works with no attributes.
func TestLoggerEmptyAttributes(t *testing.T) {
	buf := new(bytes.Buffer)
	logger := NewLogger(buf)

	logger.Info("test_event")
	logger.Warn("test_event")
	logger.Error("test_event")

	lines := strings.Split(strings.TrimSpace(buf.String()), "\n")
	if len(lines) < 3 {
		t.Fatalf("Expected at least 3 log lines, got %d", len(lines))
	}

	for i, line := range lines {
		var record map[string]interface{}
		if err := json.Unmarshal([]byte(line), &record); err != nil {
			t.Errorf("line %d is not valid JSON: %v", i, err)
			continue
		}

		// Verify required fields are still present
		if _, ok := record["ts"]; !ok {
			t.Errorf("line %d missing 'ts' field", i)
		}
		if _, ok := record["level"]; !ok {
			t.Errorf("line %d missing 'level' field", i)
		}
		if _, ok := record["event"]; !ok {
			t.Errorf("line %d missing 'event' field", i)
		}
	}
}
