package main

import (
	"io"
	"log/slog"
	"time"
)

// LogEvent is a typed constant for structured logging events.
type LogEvent string

// LogEvent constants - all 27 logging events from agentbus.
const (
	LogServerStarted      LogEvent = "server_started"         // an MCP server started for an agent run
	LogServerStopping     LogEvent = "server_stopping"        // an MCP server is exiting (signal or stdin closed)
	LogToolCall           LogEvent = "tool_call"              // an MCP tool call succeeded
	LogToolCallFailed     LogEvent = "tool_call_failed"       // an MCP tool call returned an error
	LogMessagePosted      LogEvent = "message_posted"         // a message was appended to the bus
	LogAgentGone          LogEvent = "agent_gone"             // a presence of a dead agentbus process was removed
	LogMemberWaking       LogEvent = "member_waking"          // a sleeping team member is being woken
	LogMemberWokenBy      LogEvent = "member_woken_by"        // what woke a sleeping member: a message (its id, kind, sender), an unread one, new work on the board
	LogSideEffectFailed   LogEvent = "side_effect_failed"     // an auxiliary step failed; the operation went on
	LogCommandFailed      LogEvent = "command_failed"         // a CLI subcommand failed
	LogJSONRPCError       LogEvent = "jsonrpc_error"          // a protocol-level error was sent to the MCP client
	LogJSONRPCWriteFailed LogEvent = "jsonrpc_write_failed"   // a response could not be written to the MCP client
	LogClientReadFailed   LogEvent = "client_read_failed"     // reading requests from the MCP client failed
	LogCorruptMessageLine LogEvent = "corrupt_message_line"   // a line of messages.jsonl could not be decoded
	LogCorruptNoteLine    LogEvent = "corrupt_notebook_line"  // a line of a notebook log could not be decoded
	LogNonJSONCLILine     LogEvent = "non_json_cli_line"      // the CLI event stream contained a non-JSON line
	LogUnencodableInput   LogEvent = "tool_input_unencodable" // a tool input of a CLI event could not be encoded
	LogLeadSyncRejected   LogEvent = "lead_sync_rejected"     // a task mirror sync was refused or could not be stored
	LogLeadListening      LogEvent = "lead_listening"         // the lead started listening on an address
	LogLeadMCPCall        LogEvent = "lead_mcp_call"          // a remote member's MCP message ran on the lead
	LogRemoteMemberEvent  LogEvent = "remote_member_event"    // a remote member's change handed to the agent script
	LogDoorbellRing       LogEvent = "doorbell_ring"          // the lead rang a node: something waits for it
	LogDoorbellLost       LogEvent = "doorbell_lost"          // a node's doorbell stream broke or was refused (it reconnects)
	LogRelayLeadFailed    LogEvent = "relay_lead_failed"      // a remote member's relay could not reach the lead, or was refused
	LogLeadStopped        LogEvent = "lead_stopped"           // the lead shut down its listeners
	LogLeadHTTPError      LogEvent = "lead_http_error"        // net/http emitted a connection or handler diagnostic
	LogLeadPeerRejected   LogEvent = "lead_peer_rejected"     // the lead refused a socket peer; the error says why
)

// Logger provides structured logging with JSON output.
type Logger struct {
	handler slog.Handler
	logger  *slog.Logger
}

// NewLogger creates a new logger that writes JSON records to w.
func NewLogger(w io.Writer) *Logger {
	// Create JSON handler with custom schema
	handler := slog.NewJSONHandler(w, &slog.HandlerOptions{
		Level:       slog.LevelInfo,  // Only info and above
		ReplaceAttr: commonSchema,
	})

	logger := slog.New(handler)

	return &Logger{
		handler: handler,
		logger:  logger,
	}
}

// Info logs an info-level message.
func (l *Logger) Info(event LogEvent, attrs ...interface{}) {
	l.logger.Info(string(event), attrs...)
}

// Warn logs a warn-level message.
func (l *Logger) Warn(event LogEvent, attrs ...interface{}) {
	l.logger.Warn(string(event), attrs...)
}

// Error logs an error-level message.
func (l *Logger) Error(event LogEvent, attrs ...interface{}) {
	l.logger.Error(string(event), attrs...)
}

// commonSchema renames slog's built-in keys to match the common schema.
// Changes "time" → "ts", "level" → "level" (lowercase), "msg" → "event".
func commonSchema(groups []string, a slog.Attr) slog.Attr {
	if len(groups) > 0 {
		return a
	}
	switch a.Key {
	case slog.TimeKey:
		return slog.String("ts", a.Value.Time().UTC().Format(time.RFC3339))
	case slog.LevelKey:
		// Convert level to lowercase (slog.Level.String() returns uppercase)
		levelStr := a.Value.String()
		if levelStr == "INFO" {
			levelStr = "info"
		} else if levelStr == "WARN" {
			levelStr = "warn"
		} else if levelStr == "ERROR" {
			levelStr = "error"
		}
		return slog.String("level", levelStr)
	case slog.MessageKey:
		return slog.String("event", a.Value.String())
	}
	return a
}
