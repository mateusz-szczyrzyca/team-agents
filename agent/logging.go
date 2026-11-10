package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
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
	// LogNoticeSent reports successful delivery of a bus notice.
	LogNoticeSent LogEvent = "notice_sent"
	// LogEventScanFailed reports failed activity filtering.
	LogEventScanFailed LogEvent = "event_scan_failed"
	// LogCommandFailed reports failure of a command.
	LogCommandFailed LogEvent = "command_failed"
	// LogInvListFailed reports a failed best-effort investigation list query.
	LogInvListFailed LogEvent = "inv_list_failed"
	// LogJobOriginLookupFailed reports failure querying git remote get-url origin.
	LogJobOriginLookupFailed LogEvent = "job_origin_lookup_failed"
)

// Logger owns destinations and immutable invocation metadata; Log serializes writes.
type Logger struct {
	mu          sync.Mutex // serializes writes and diagnostics across concurrent records
	LogDir      string     // directory holding agent.jsonl
	Subcommand  string     // command being run
	Task        string     // runner task identifier, empty for interactive commands
	PID         int        // process identifier, zero selects the current process
	Clock       Clock      // injected time source, nil selects wall time
	Runner      io.Writer  // runner fd 3 destination when Task is set
	Stderr      io.Writer  // diagnostics destination, nil selects os.Stderr
	Journal     bool       // invoke logger through App when enabled
	App         *App       // process boundary for journal delivery
	Interactive bool       // whether stderr is an interactive terminal
	Quiet       bool       // suppress interactive warnings
}

// Log emits one record to configured destinations, warning and returning a typed error on failure.
func (l *Logger) Log(ctx context.Context, level LogLevel, event LogEvent, fields ...string) *LogWriteError {
	l.mu.Lock()
	defer l.mu.Unlock()
	now := time.Now()
	if l.Clock != nil {
		now = l.Clock.Now()
	}
	pid := l.PID
	if pid == 0 {
		pid = os.Getpid()
	}
	record := map[string]any{"ts": now.UTC().Format(time.RFC3339), "level": string(level), "component": "agent", "event": string(event), "pid": pid, "subcommand": l.Subcommand}
	if l.Task != "" {
		record["task"] = l.Task
	}
	for _, field := range fields {
		key, value, ok := strings.Cut(field, "=")
		if !ok {
			value = key
		}
		record[key] = value
	}
	line, err := json.Marshal(record)
	if err != nil {
		return l.logFailure(storageErr(err, "log destination"), "agent: cannot encode the log record")
	}
	line = append(line, '\n')
	var failure *LogWriteError
	path := filepath.Join(l.LogDir, "agent.jsonl")
	if err := os.MkdirAll(l.LogDir, 0o700); err != nil {
		failure = l.logFailure(storageErr(err, "log destination"), "agent: cannot write "+path)
	} else if serr := appendLog(path, line); serr != nil {
		failure = l.logFailure(serr, "agent: cannot write "+path)
	}
	if l.Journal {
		app := l.App
		if app == nil {
			app = newApp(nil, nil, nil)
		}
		priority := "info"
		switch level {
		case LevelWarn:
			priority = "warning"
		case LevelError:
			priority = "err"
		}
		result, eerr := app.execute(ctx, Command{Name: "logger", Args: []string{"-t", "agent", "-p", "user." + priority, "--", strings.TrimSuffix(string(line), "\n")}})
		if eerr != nil {
			failure = joinLogFailures(failure, l.logFailure(storageErr(eerr, "journal command"), "agent: cannot write to the system journal"))
		} else if result.ExitCode != 0 {
			failure = joinLogFailures(failure, l.logFailure(storageErr(fmt.Errorf("logger exited %d", result.ExitCode), "journal command"), "agent: cannot write to the system journal"))
		}
	}
	if l.Task != "" && l.Runner != nil {
		n, err := l.Runner.Write(line)
		if err == nil && n != len(line) {
			err = io.ErrShortWrite
		}
		if err != nil {
			failure = joinLogFailures(failure, l.logFailure(storageErr(err, "log destination"), "agent: cannot write runner.log"))
		}
	} else if l.Task == "" && l.Interactive && !l.Quiet && (level == LevelWarn || level == LevelError) {
		message := "agent: " + string(level) + ": " + string(event)
		if len(fields) > 0 {
			message += " (" + strings.Join(fields, " ") + ")"
		}
		if _, err := fmt.Fprintln(l.diagnosticWriter(), message); err != nil {
			failure = joinLogFailures(failure, fromStorage(LogWriteStorage, storageErr(err, "write interactive log")))
		}
	}
	return failure
}

// diagnosticWriter selects the caller-owned diagnostic stream.
func (l *Logger) diagnosticWriter() io.Writer {
	if l.Stderr != nil {
		return l.Stderr
	}
	return os.Stderr
}

// logFailure reports a failed destination and retains both destination and diagnostic errors.
func (l *Logger) logFailure(failure *StorageError, message string) *LogWriteError {
	if derr := writeLog(l.diagnosticWriter(), []byte(message+"\n")); derr != nil {
		failure = storageErr(errors.Join(failure, derr), "write diagnostic")
	}
	return fromStorage(LogWriteStorage, failure)
}

// joinLogFailures preserves earlier failures when more than one destination fails.
func joinLogFailures(first, next *LogWriteError) *LogWriteError {
	if first == nil {
		return next
	}
	if next == nil {
		return first
	}
	return fromStorage(LogWriteStorage, storageErr(errors.Join(first, next), "write log destinations"))
}

// appendLog writes one complete record; a close error still fails the operation.
func appendLog(path string, line []byte) *StorageError {
	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600) //nolint:gosec // paths are caller-owned log destinations, not a security boundary
	if err != nil {
		return storageErr(err, "open log")
	}
	n, werr := f.Write(line)
	if werr == nil && n != len(line) {
		werr = io.ErrShortWrite
	}
	if cerr := f.Close(); cerr != nil {
		werr = errors.Join(werr, cerr)
	}
	if werr != nil {
		return storageErr(werr, "append log")
	}
	return nil
}

// logPrefix retains a bounded stderr prefix while draining the whole subprocess stream.
type logPrefix struct {
	data []byte /* first 2000 bytes of command diagnostics */
}

// Write implements io.Writer; discarding the suffix is deliberate, like head -c 2000.
func (p *logPrefix) Write(data []byte) (int, error) {
	if remaining := 2000 - len(p.data); remaining > 0 {
		p.data = append(p.data, data[:min(remaining, len(data))]...)
	}
	return len(data), nil
}

// Try runs a warning-only side effect and preserves the command's stdout destination.
func Try(ctx context.Context, app *App, logger *Logger, event LogEvent, cmd Command) *LogWriteError {
	stderr := &logPrefix{}
	cmd.Stderr = stderr
	result, eerr := app.execute(ctx, cmd)
	if eerr == nil && result.ExitCode == 0 {
		return nil
	}
	code := result.ExitCode
	if eerr != nil {
		code = eerr.Class().ExitCode()
		if len(stderr.data) == 0 {
			stderr.data = []byte(eerr.Error())
			stderr.data = stderr.data[:min(2000, len(stderr.data))]
		}
	}
	return logger.Log(ctx, LevelWarn, event, "exit_code="+strconv.Itoa(code), "command="+strings.Join(append([]string{cmd.Name}, cmd.Args...), " "), "stderr="+strings.TrimRight(string(stderr.data), "\n"))
}

// JQSoft runs jq and returns command-substitution output, warning on malformed input.
func JQSoft(ctx context.Context, app *App, logger *Logger, event LogEvent, input io.Reader, args ...string) (string, *LogWriteError) {
	var stdout bytes.Buffer
	stderr := &logPrefix{}
	result, eerr := app.execute(ctx, Command{Name: "jq", Args: args, Stdin: input, Stdout: &stdout, Stderr: stderr})
	var failure *LogWriteError
	if eerr != nil || result.ExitCode != 0 {
		code := result.ExitCode
		if eerr != nil {
			code = eerr.Class().ExitCode()
			if len(stderr.data) == 0 {
				stderr.data = []byte(eerr.Error())
				stderr.data = stderr.data[:min(2000, len(stderr.data))]
			}
		}
		failure = logger.Log(ctx, LevelWarn, event, "exit_code="+strconv.Itoa(code), "stderr="+strings.TrimRight(string(stderr.data), "\n"))
	}
	return strings.TrimRight(stdout.String(), "\n"), failure
}

// RenderLog formats structured records in their original field order, passing through other lines.
func RenderLog(r io.Reader, w io.Writer) *RenderLogError {
	reader := bufio.NewReader(r)
	for {
		line, rerr := reader.ReadString('\n')
		if len(line) > 0 {
			rendered := renderLogLine(strings.TrimSuffix(line, "\n"))
			n, werr := io.WriteString(w, rendered+"\n")
			if werr == nil && n != len(rendered)+1 {
				werr = io.ErrShortWrite
			}
			if werr != nil {
				return fromStorage(RenderLogStorage, storageErr(werr, "write rendered log"))
			}
		}
		if rerr != nil {
			if rerr == io.EOF {
				return nil
			}
			return fromStorage(RenderLogStorage, storageErr(rerr, "read log"))
		}
	}
}

// logField preserves JSON object entry order for readable log rendering.
type logField struct {
	key   string // original object key
	value any    // decoded JSON field value
}

// renderLogLine mirrors the jq formatter for well-formed structured records.
func renderLogLine(line string) string {
	var record map[string]any
	if err := json.Unmarshal([]byte(line), &record); err != nil || record == nil {
		return line
	}
	event, exists := record["event"]
	if !exists {
		return line
	}
	decoder := json.NewDecoder(strings.NewReader(line))
	if _, err := decoder.Token(); err != nil {
		return line
	}
	var fields []logField
	for decoder.More() {
		token, err := decoder.Token()
		if err != nil {
			return line
		}
		var value any
		if err := decoder.Decode(&value); err != nil {
			return line
		}
		key, ok := token.(string)
		if !ok {
			return line
		}
		fields = append(fields, logField{key: key, value: value})
	}
	ts, _ := record["ts"].(string)
	if len(ts) > 11 {
		ts = ts[11:min(19, len(ts))]
	} else {
		ts = ""
	}
	level, _ := record["level"].(string)
	if level == "" {
		level = "info"
	}
	level = strings.ToUpper(level)
	if len(level) > 5 {
		level = level[:5]
	}
	rendered := ts + " " + level + " " + logValue(event)
	for _, field := range fields {
		switch field.key {
		case "ts", "level", "event", "component", "pid", "subcommand", "task":
			continue
		}
		if value, ok := field.value.(string); ok && value == "" {
			continue
		}
		value := logValue(field.value)
		if strings.ContainsAny(value, " \t\r\n\v\f") {
			encoded, e := json.Marshal(value)
			if e != nil {
				return line
			}
			value = string(encoded)
		}
		rendered += " " + field.key + "=" + value
	}
	return rendered
}

// logValue matches jq tostring for the JSON values emitted in log records.
func logValue(value any) string {
	if text, ok := value.(string); ok {
		return text
	}
	encoded, err := json.Marshal(value)
	if err != nil {
		return "null"
	}
	return string(encoded)
}

// Act appends the same local-time event to both legacy activity streams.
func Act(taskDir, kind, text string, clock Clock) *ActError {
	now := time.Now()
	if clock != nil {
		now = clock.Now()
	}
	record := struct {
		TS   string `json:"ts"` // Bash date's local timestamp
		Kind string `json:"k"`  // activity category expected by the live panel
		Text string `json:"t"`  // human-readable activity description
	}{TS: now.Format("2006-01-02 15:04:05"), Kind: kind, Text: text}
	line, err := json.Marshal(record)
	if err != nil {
		return fromStorage(ActStorage, storageErr(err, "encode activity"))
	}
	line = append(line, '\n')
	for _, name := range []string{"activity.jsonl", "activity-full.jsonl"} {
		if serr := appendLog(filepath.Join(taskDir, name), line); serr != nil {
			return fromStorage(ActStorage, serr)
		}
	}
	return nil
}

// LastEvent preserves arbitrary jq filters and ignores malformed JSON activity lines.
func LastEvent(ctx context.Context, app *App, logger *Logger, path, filter string) (string, *LogWriteError) {
	input, err := os.Open(path) //nolint:gosec // caller selects a local activity log, as with the Bash helper
	if errors.Is(err, os.ErrNotExist) {
		return "", nil
	}
	if err != nil {
		return "", logger.logFailure(storageErr(err, "activity stream"), "agent: cannot read "+path)
	}
	output, failure := JQSoft(ctx, app, logger, LogEventScanFailed, input, "-ncR", "last(inputs | fromjson? | select("+filter+"))")
	if err := input.Close(); err != nil {
		failure = joinLogFailures(failure, logger.logFailure(storageErr(err, "activity stream"), "agent: cannot close "+path))
	}
	return output, failure
}

// Valid reports whether this severity is part of the log schema.
func (level LogLevel) Valid() bool {
	return level == LevelInfo || level == LevelWarn || level == LevelError
}

// UnmarshalJSON validates the severity; errors contain *ValueError.
func (level *LogLevel) UnmarshalJSON(data []byte) error {
	var text string
	if err := json.Unmarshal(data, &text); err != nil {
		return valueWrap(err, "invalid log level").asError()
	}
	candidate := LogLevel(text)
	if !candidate.Valid() {
		return valueErr("unknown log level %q", text).asError()
	}
	*level = candidate
	return nil
}

// Valid reports whether this event is defined by this candidate's logging module.
func (event LogEvent) Valid() bool {
	return event == LogNoticeFailed || event == LogNoticeSent || event == LogEventScanFailed || event == LogCommandFailed ||
		event == LogGitIdentity || event == LogSigningKeyUnreadable || event == LogAgentsGHMissing || event == LogGitHubColorsFailed ||
		event == EventGitHubCommentIgnored || event == EventGitHubCommentMarkedIgnored || event == EventGitHubPRIgnored ||
		event == EventGitHubIssueIgnored || event == EventPRUntrustedIgnored || event == EventGitHubGraphQLRefused ||
		event == EventGitHubMemberPRRefused || event == EventGitHubSecondPRRefused || event == EventGitHubCoordinatorUnknown
}

// UnmarshalJSON validates the event; errors contain *ValueError.
func (event *LogEvent) UnmarshalJSON(data []byte) error {
	var text string
	if err := json.Unmarshal(data, &text); err != nil {
		return valueWrap(err, "invalid log event").asError()
	}
	candidate := LogEvent(text)
	if !candidate.Valid() {
		return valueErr("unknown log event %q", text).asError()
	}
	*event = candidate
	return nil
}

// writeLog detects broken writers that return a short count without an error.
func writeLog(w io.Writer, data []byte) *StorageError {
	n, err := w.Write(data)
	if err == nil && n != len(data) {
		err = io.ErrShortWrite
	}
	if err != nil {
		return storageErr(err, "write log stream")
	}
	return nil
}
