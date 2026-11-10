package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestLogJobOriginLookupFailedValidation verifies LogJobOriginLookupFailed is properly validated.
func TestLogJobOriginLookupFailedValidation(t *testing.T) {
	t.Run("event is valid", func(t *testing.T) {
		if !LogJobOriginLookupFailed.Valid() {
			t.Fatalf("LogJobOriginLookupFailed.Valid() = false, want true")
		}
	})

	t.Run("JSON round-trip preserves validity", func(t *testing.T) {
		tempDir := t.TempDir()
		logger := &Logger{LogDir: tempDir}
		ctx := context.Background()

		// Log the event
		if err := logger.Log(ctx, LevelWarn, LogJobOriginLookupFailed, "repository=/test", "exit=128", "reason=test error"); err != nil {
			t.Fatalf("logger.Log failed: %v", err)
		}

		// Read and verify the logged event
		data, err := os.ReadFile(filepath.Clean(filepath.Join(tempDir, "agent.jsonl"))) //nolint:gosec // G304: tempDir is test-controlled
		if err != nil {
			t.Fatalf("failed to read agent.jsonl: %v", err)
		}

		// Parse JSON and verify event field
		line := strings.TrimSpace(string(data))
		var record map[string]interface{}
		if err := json.Unmarshal([]byte(line), &record); err != nil {
			t.Fatalf("failed to parse JSON: %v", err)
		}

		event, ok := record["event"].(string)
		if !ok {
			t.Fatalf("event field missing or not string: %v", record)
		}

		logEvent := LogEvent(event)
		if !logEvent.Valid() {
			t.Fatalf("parsed event %q should be valid", event)
		}

		if logEvent != LogJobOriginLookupFailed {
			t.Fatalf("event = %q, want %q", logEvent, LogJobOriginLookupFailed)
		}
	})
}

// TestLogJobOriginLookupFailedJSONRoundtrip verifies LogJobOriginLookupFailed round-trips through JSON marshaling.
func TestLogJobOriginLookupFailedJSONRoundtrip(t *testing.T) {
	event := LogJobOriginLookupFailed

	// Marshal to JSON
	data, err := json.Marshal(event)
	if err != nil {
		t.Fatalf("json.Marshal failed: %v", err)
	}

	// Unmarshal back into typed LogEvent using UnmarshalJSON
	var decoded LogEvent
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatalf("json.Unmarshal into LogEvent failed: %v", err)
	}

	// Verify identity and validity
	if decoded != LogJobOriginLookupFailed {
		t.Fatalf("decoded = %q, want %q", decoded, LogJobOriginLookupFailed)
	}

	if !decoded.Valid() {
		t.Fatalf("decoded.Valid() = false, want true")
	}
}
