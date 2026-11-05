# R08 Implementation Reference — bus.go and logging.go

## LogEvent Constants (27 events)

From `agentbus/logging.go`, all log events that agent must define:

```
agent_gone
client_read_failed
command_failed
corrupt_message_line
corrupt_notebook_line
doorbell_lost
doorbell_ring
jsonrpc_error
jsonrpc_write_failed
lead_http_error
lead_listening
lead_mcp_call
lead_peer_rejected
lead_stopped
lead_sync_rejected
member_waking
member_woken_by
message_posted
non_json_cli_line
relay_lead_failed
remote_member_event
server_started
server_stopping
side_effect_failed
tool_call
tool_call_failed
tool_input_unencodable
```

## Log Record Schema

From `agentbus/logging.go` `commonSchema` function:
- **ts**: RFC 3339 UTC timestamp (standard Go `time.RFC3339` constant)
- **level**: lowercase ("info", "warn", "error") 
- **event**: the LogEvent constant name (replaces slog's "msg")
- **additional attributes**: arbitrary key=value pairs passed to log functions

Example JSON record:
```json
{"ts":"2026-11-05T16:47:00Z","level":"info","event":"tool_call","tool":"board_list","time_ms":125}
```

## Implementation Approach

### logging.go

1. Define LogEvent as typed constants
2. Implement Logger using `log/slog` with custom JSON handler
3. Key functions:
   - `NewLogger(w io.Writer) *Logger` - creates logger to given writer
   - `Info/Warn/Error(event LogEvent, attrs ...any)` - log at respective levels
4. Custom handler must:
   - Set log level to Info (omit Debug messages)
   - Rename "time" → "ts", "level" → "level", "msg" → "event"
   - Format timestamps as RFC 3339 UTC

### bus.go

1. Spawn agentbus subprocess as MCP server
2. Implement JSON RPC 2.0 protocol for tool calls
3. Key interfaces:
   - `NewBus(ctx Context, binPath string, args ...string) (*BusConn, error)`
   - `Call(method string, args any) (any, error)` - MCP tool call
   - `PostMessage(from, to, kind, text string) error`
   - `ReadMessages(ctx) ([]Message, error)`
4. Handle subprocess lifecycle, stdout/stderr, exit codes
5. Proper process cleanup on context cancellation

## MCP JSON RPC 2.0 Format

Tool call request:
```json
{"jsonrpc":"2.0","method":"tools/call","params":{"name":"<tool>","arguments":{...}},"id":1}
```

Response (success):
```json
{"jsonrpc":"2.0","result":"...","id":1}
```

Response (error):
```json
{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid params"},"id":1}
```

## Key Test Cases

### logging_test.go (RED/GREEN)
- All 27 LogEvent constants defined
- JSON schema validation (ts, level, event fields)
- Concurrent logging safety
- Write error handling

### bus_test.go (RED/GREEN)  
- Subprocess spawning and cleanup
- MCP tool call protocol
- Error response handling
- Context cancellation
- Message posting and reading

## Reference Files

- `agentbus/logging.go` - log handler implementation pattern
- `agentbus/server.go` - MCP server implementation
- `agentbus/tool.go` - tool definitions
- `agentbus/errors.json` - operation and cause definitions
