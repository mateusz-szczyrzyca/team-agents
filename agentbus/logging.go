// Structured logging.
//
// Every agentbus process (MCP server or CLI subcommand) logs JSON records with log/slog to two destinations:
//
//   - the file <AGENTS_HOME>/logs/agentbus.jsonl (one JSON object per line, for Promtail/Vector/vmagent),
//   - the systemd journal, through its native socket, with SYSLOG_IDENTIFIER=agentbus and the JSON record as
//     MESSAGE (for pipelines that read journald).
//
// Every record has the same schema as the records of the agent script and agentlive:
//
//	{"ts":"2026-10-20T14:05:11Z","level":"warn","event":"side_effect_failed","component":"agentbus",...}
//
// "event" is a LogEvent constant (a stable snake_case name for dashboards and alerts).
// Records always contain "component":"agentbus", the process role ("serve" or the CLI subcommand) and, for
// MCP servers, the agent name and task. Logging never writes to stdout (the MCP protocol) and does not write
// JSON to stderr (agent CLIs and humans read it); a destination that cannot be opened is reported once on
// stderr and skipped.
package main

import (
	"bytes"
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
)

// LogEvent names a log record (its "event" field). The names are stable: dashboards and alerts use them.
type LogEvent string

const (
	LogServerStarted      LogEvent = "server_started"         // an MCP server started for an agent run
	LogServerStopping     LogEvent = "server_stopping"        // an MCP server is exiting (signal or stdin closed)
	LogToolCall           LogEvent = "tool_call"              // an MCP tool call succeeded
	LogToolCallFailed     LogEvent = "tool_call_failed"       // an MCP tool call returned an error
	LogMessagePosted      LogEvent = "message_posted"         // a message was appended to the bus
	LogAgentGone          LogEvent = "agent_gone"             // a presence of a dead agentbus process was removed
	LogMemberWaking       LogEvent = "member_waking"          // a sleeping team member is being woken
	LogSideEffectFailed   LogEvent = "side_effect_failed"     // an auxiliary step failed; the operation went on
	LogCommandFailed      LogEvent = "command_failed"         // a CLI subcommand failed
	LogJSONRPCError       LogEvent = "jsonrpc_error"          // a protocol-level error was sent to the MCP client
	LogJSONRPCWriteFailed LogEvent = "jsonrpc_write_failed"   // a response could not be written to the MCP client
	LogClientReadFailed   LogEvent = "client_read_failed"     // reading requests from the MCP client failed
	LogCorruptMessageLine LogEvent = "corrupt_message_line"   // a line of messages.jsonl could not be decoded
	LogCorruptNoteLine    LogEvent = "corrupt_notebook_line"  // a line of a notebook log could not be decoded
	LogNonJSONCLILine     LogEvent = "non_json_cli_line"      // the CLI event stream contained a non-JSON line
	LogUnencodableInput   LogEvent = "tool_input_unencodable" // a tool input of a CLI event could not be encoded
	LogLeadListening      LogEvent = "lead_listening"         // the lead started listening on an address
	LogLeadStopped        LogEvent = "lead_stopped"           // the lead shut down its listeners
	LogLeadHTTPError      LogEvent = "lead_http_error"        // net/http emitted a connection or handler diagnostic
)

// logInfo, logWarn and logError write a record with the given event name.
func logInfo(ev LogEvent, attrs ...any)  { logger.Info(string(ev), attrs...) }
func logWarn(ev LogEvent, attrs ...any)  { logger.Warn(string(ev), attrs...) }  // see logInfo
func logError(ev LogEvent, attrs ...any) { logger.Error(string(ev), attrs...) } // see logInfo

// newJSONHandler returns a JSON handler with the common record schema: "ts" (RFC 3339, UTC), lower-case
// "level" and "event" instead of slog's "time", "level" and "msg".
func newJSONHandler(w io.Writer) slog.Handler {
	return slog.NewJSONHandler(w, &slog.HandlerOptions{Level: slog.LevelInfo, ReplaceAttr: commonSchema})
}

// commonSchema renames slog's built-in keys to the common schema of all team-agents components.
func commonSchema(groups []string, a slog.Attr) slog.Attr {
	if len(groups) > 0 {
		return a
	}
	switch a.Key {
	case slog.TimeKey:
		return slog.String("ts", a.Value.Time().UTC().Format(time.RFC3339))
	case slog.LevelKey:
		return slog.String("level", strings.ToLower(a.Value.String()))
	case slog.MessageKey:
		return slog.String("event", a.Value.String())
	}
	return a
}

// journalSocket is the native protocol socket of systemd-journald.
const journalSocket = "/run/systemd/journal/socket"

// logger is the process-wide structured logger. Until setupLogging runs it discards everything; tests replace it.
var logger = slog.New(slog.NewJSONHandler(io.Discard, nil))

// setupLogging configures logger for this process. home is AGENTS_HOME, role is "serve" or the CLI subcommand,
// and attrs are added to every record (e.g. agent and task of an MCP server).
func setupLogging(home, role string, attrs ...any) {
	var handlers []slog.Handler

	logDir := filepath.Join(home, "logs")
	if err := os.MkdirAll(logDir, 0o700); err != nil {
		fmt.Fprintf(os.Stderr, "agentbus: logging to files disabled: cannot create %s: %v\n", logDir, err)
	} else if f, err := os.OpenFile(filepath.Join(logDir, "agentbus.jsonl"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600); err != nil { //nolint:gosec // G304: the log directory is under the configured local AGENTS_HOME.
		fmt.Fprintf(os.Stderr, "agentbus: logging to files disabled: %v\n", err)
	} else {
		// one Write per record: O_APPEND keeps concurrent writers' lines intact
		handlers = append(handlers, newJSONHandler(f))
	}
	if j, serr := newJournalHandler(); serr != nil {
		fmt.Fprintf(os.Stderr, "agentbus: logging to the journal disabled: %v\n", serr)
	} else if j != nil {
		handlers = append(handlers, j)
	}

	base := []any{"component", "agentbus", "role", role, "pid", os.Getpid()}
	logger = slog.New(fanout(handlers)).With(append(base, attrs...)...)
}

// ---------------------------------------------------------------- fan-out

// fanoutHandler sends every record to several handlers.
type fanoutHandler struct {
	handlers []slog.Handler // destinations
}

// fanout returns a handler for all destinations (a discarding one if there are none).
func fanout(hs []slog.Handler) slog.Handler {
	if len(hs) == 0 {
		return slog.NewJSONHandler(io.Discard, nil)
	}
	return fanoutHandler{handlers: hs}
}

// Enabled implements slog.Handler: enabled if any destination is.
func (f fanoutHandler) Enabled(ctx context.Context, l slog.Level) bool {
	for _, h := range f.handlers {
		if h.Enabled(ctx, l) {
			return true
		}
	}
	return false
}

// Handle implements slog.Handler (hence the plain error type): the errors of the destinations that failed are
// joined (*StorageError from the journal; errors of the file handler come from os.File.Write).
func (f fanoutHandler) Handle(ctx context.Context, r slog.Record) error {
	var errs []error
	for _, h := range f.handlers {
		if h.Enabled(ctx, r.Level) {
			if err := h.Handle(ctx, r.Clone()); err != nil {
				errs = append(errs, err)
			}
		}
	}
	return errors.Join(errs...)
}

// WithAttrs implements slog.Handler.
func (f fanoutHandler) WithAttrs(attrs []slog.Attr) slog.Handler {
	out := make([]slog.Handler, len(f.handlers))
	for i, h := range f.handlers {
		out[i] = h.WithAttrs(attrs)
	}
	return fanoutHandler{handlers: out}
}

// WithGroup implements slog.Handler.
func (f fanoutHandler) WithGroup(name string) slog.Handler {
	out := make([]slog.Handler, len(f.handlers))
	for i, h := range f.handlers {
		out[i] = h.WithGroup(name)
	}
	return fanoutHandler{handlers: out}
}

// ---------------------------------------------------------------- journald

// journalHandler formats records as JSON (MESSAGE) and sends them to journald's native socket.
type journalHandler struct {
	mu   *sync.Mutex   // guards buf and json across goroutines
	buf  *bytes.Buffer // the JSON-encoded record
	json slog.Handler  // JSON handler writing into buf
	conn *net.UnixConn // datagram connection to journald
}

// newJournalHandler connects to journald. A host without a journal is not an error: the result is then
// (nil, nil) and the journal destination is simply skipped. A journal that exists but cannot be reached is a
// *StorageError.
func newJournalHandler() (*journalHandler, *StorageError) {
	_, err := os.Stat(journalSocket)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, storageErr(err, "cannot check %s", journalSocket)
	}
	conn, err := net.DialUnix("unixgram", nil, &net.UnixAddr{Name: journalSocket, Net: "unixgram"})
	if err != nil {
		return nil, storageErr(err, "cannot connect to %s", journalSocket)
	}
	buf := &bytes.Buffer{}
	return &journalHandler{mu: &sync.Mutex{}, buf: buf, json: newJSONHandler(buf), conn: conn}, nil
}

// journalPriority maps slog levels to syslog priorities (journald PRIORITY field).
func journalPriority(l slog.Level) int {
	switch {
	case l >= slog.LevelError:
		return 3 // err
	case l >= slog.LevelWarn:
		return 4 // warning
	case l >= slog.LevelInfo:
		return 6 // info
	default:
		return 7 // debug
	}
}

// Enabled implements slog.Handler.
func (j *journalHandler) Enabled(ctx context.Context, l slog.Level) bool {
	return j.json.Enabled(ctx, l)
}

// Handle implements slog.Handler (hence the plain error type): it encodes the record and sends one journal
// entry. A failure is always a *StorageError: the record could not be encoded, or the journal socket did not
// accept the entry.
func (j *journalHandler) Handle(ctx context.Context, r slog.Record) error {
	j.mu.Lock()
	defer j.mu.Unlock()
	j.buf.Reset()
	if err := j.json.Handle(ctx, r); err != nil {
		return storageErr(err, "cannot encode the log record for the journal").asError()
	}
	msg := bytes.TrimRight(j.buf.Bytes(), "\n")
	var entry bytes.Buffer
	entry.WriteString("SYSLOG_IDENTIFIER=agentbus\n")
	entry.WriteString("PRIORITY=" + strconv.Itoa(journalPriority(r.Level)) + "\n")
	// MESSAGE in the binary-safe form: name, newline, 64-bit little-endian length, data, newline
	entry.WriteString("MESSAGE\n")
	var size [8]byte
	binary.LittleEndian.PutUint64(size[:], uint64(len(msg)))
	entry.Write(size[:])
	entry.Write(msg)
	entry.WriteByte('\n')
	if _, err := j.conn.Write(entry.Bytes()); err != nil {
		return storageErr(err, "cannot send the log record to the journal").asError()
	}
	return nil
}

// WithAttrs implements slog.Handler.
func (j *journalHandler) WithAttrs(attrs []slog.Attr) slog.Handler {
	c := *j
	c.json = j.json.WithAttrs(attrs)
	return &c
}

// WithGroup implements slog.Handler.
func (j *journalHandler) WithGroup(name string) slog.Handler {
	c := *j
	c.json = j.json.WithGroup(name)
	return &c
}

// ---------------------------------------------------------------- helpers

// warnSideEffect logs a failed side effect (an operation that succeeded in the shared state but could not
// update an auxiliary file or start a helper process). The operation itself is not failed. Only typed errors
// are accepted, so the record always carries a class and a cause.
func warnSideEffect(effect string, err classified, attrs ...any) {
	logWarn(LogSideEffectFailed, append([]any{"effect", effect, "error", err.Error(), "class", err.Class().String(),
		"cause", err.CauseName()}, attrs...)...)
}

// warnedOnce remembers the keys already reported by warnOnce.
var warnedOnce sync.Map

// warnOnce logs a warning only the first time a key is seen in this process (for problems found while
// polling, e.g. a corrupt line read every few seconds).
func warnOnce(key string, ev LogEvent, err classified) {
	if _, seen := warnedOnce.LoadOrStore(key, true); seen {
		return
	}
	logWarn(ev, "key", key, "error", err.Error(), "class", err.Class().String(), "cause", err.CauseName())
}
