package main

import (
	"context"
	"io"
)

// LogLevel is the stable severity written to agent.jsonl.
type LogLevel string

const (
	// LevelInfo records ordinary progress.
	LevelInfo LogLevel = "info"
	// LevelWarn records recoverable failures.
	LevelWarn LogLevel = "warn"
	// LevelError records command failures.
	LevelError LogLevel = "error"
)

// LogEvent names a stable event emitted by the script candidate.
type LogEvent string

const (
	// LogNoticeFailed reports a failed best-effort bus notice.
	LogNoticeFailed LogEvent = "notice_failed"
	// LogEventScanFailed reports failed activity filtering.
	LogEventScanFailed LogEvent = "event_scan_failed"
	// LogCommandFailed reports failure of a command.
	LogCommandFailed LogEvent = "command_failed"
)

// Logger owns destinations and immutable invocation metadata; Log serializes writes.
type Logger struct {
	LogDir      string    // directory holding agent.jsonl
	Subcommand  string    // command being run
	Task        string    // runner task identifier, empty for interactive commands
	PID         int       // process identifier, zero selects the current process
	Clock       Clock     // injected time source, nil selects wall time
	Runner      io.Writer // runner fd 3 destination when Task is set
	Stderr      io.Writer // diagnostics destination, nil selects os.Stderr
	Journal     bool      // invoke logger through App when enabled
	App         *App      // process boundary for journal delivery
	Interactive bool      // whether stderr is an interactive terminal
	Quiet       bool      // suppress interactive warnings
}

// Log emits one record to the configured destinations without aborting on failure.
func (l *Logger) Log(_ context.Context, _ LogLevel, _ LogEvent, _ ...string) *LogWriteError {
	return nil
}

// Try runs a warning-only side effect, preserving its stdout destination.
func Try(_ context.Context, _ *App, _ *Logger, _ LogEvent, _ Command) *LogWriteError {
	return nil
}

// JQSoft runs jq through the process boundary and returns command-substitution output.
func JQSoft(_ context.Context, _ *App, _ *Logger, _ LogEvent, _ io.Reader, _ ...string) (string, *LogWriteError) {
	return "", nil
}

// RenderLog formats structured records and passes through other lines.
func RenderLog(_ io.Reader, _ io.Writer) *RenderLogError { return nil }

// Act appends a timestamped activity entry to both legacy activity streams.
func Act(_, _, _ string, _ Clock) *ActError { return nil }

// LastEvent returns jq's last matching activity object, or empty output for a missing file.
func LastEvent(_ context.Context, _ *App, _ *Logger, _, _ string) (string, *LogWriteError) {
	return "", nil
}

// Valid reports whether this severity is part of the log schema.
func (LogLevel) Valid() bool { return false }

// UnmarshalJSON validates the severity; an error contains *ValueError.
func (*LogLevel) UnmarshalJSON([]byte) error { return nil }

// Valid reports whether this event is defined by this candidate's logging module.
func (LogEvent) Valid() bool { return false }

// UnmarshalJSON validates the event; an error contains *ValueError.
func (*LogEvent) UnmarshalJSON([]byte) error { return nil }
