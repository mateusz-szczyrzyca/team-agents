# Multi-instance team-agents (design)

Status: **DRAFT for human approval (milestone M0).** No implementation code is written before the human approves
this document. Open decisions are listed in section 12 and are asked through the questions file.

Terminology (decision 5): the **lead instance** is the instance that runs the **hub** (the code name). "Coordinator"
is only the role of an agent in a team (`claim_lead`). The lead instance is never called a coordinator.

Sections written by team members are marked with their author. Initial synthesis by openrouter-haiku; maintained by coordinator gemini-1 and team 20261022-154419-team (gemini-1, codex-1, openrouter-haiku).

## 1. Architecture

```
            ┌────────────────────── lead instance (e.g. pi5) ──────────────────────┐
            │  agentbus hub  (HTTP, listens on HUB_LISTEN)                         │
            │   ├─ executes bus logic on its own state files (transact / flock)    │
            │   ├─ instance registry, manifests, command queues                    │
            │   └─ backups of state (section 8)                                    │
            │  local agents: claude-1@pi5 … (their agentbus serve calls the hub    │
            │                 over private Unix domain socket)                    │
            └──────▲───────────────────────────────▲──────────────────────────────┘
                   │ HTTP (outbound from node)     │ HTTP (outbound from node)
          ┌────────┴────────┐              ┌───────┴─────────┐
          │ instance "i1"   │              │ instance "mac"  │
          │ agent node      │              │ agent node      │   (launchd on macOS)
          │ agentbus serve  │              │ agentbus serve  │
          │  → forwards to  │              │  → forwards to  │
          │    hub          │              │    hub          │
          │ agents: codex-1 │              │ agents: claude-1│
          │  worktrees, git │              │  worktrees, git │
          └─────────────────┘              └─────────────────┘
```

- **One logical bus.** The hub is the only process that reads or writes `bus/state.json` and
  `bus/messages.jsonl` of the team. Non-lead instances never touch the logical bus files; they forward.
- **Forwarding.** On a non-lead instance, every MCP call of every agent goes through the local
  `agentbus serve` (one per run, as today). `serve` sends the tool name, the arguments and the caller identity
  (agent, instance, task, team, tier, role) to the hub and returns the hub's answer unchanged. The hub runs the
  same `callTool` code path as a local `serve`; there is no second implementation of the bus logic and no
  remote locking.
- **Local instance without a lead.** With no `LEAD_INSTANCE`, `HUB_LISTEN` or `HUB_SOCKET`, `agentbus serve` uses the local
  files exactly as today (decision 2). The hub mode is a switch in configuration, not a different code base.
- **The lead instance runs agents too** (decision 7). Its local `agentbus serve` sends the same HTTP envelope through
  the hub daemon's private Unix socket (resumed round 1 approval), with one shared hub executor.
- **`agent node`** runs on every non-lead instance (section 6). It is the only process that connects to the hub
  for control: registration, manifest, commands and task state. Only the hub listens (decision 8).

## 2. HTTP protocol

Author: codex-1, reviewed by gemini-1 and the coordinator; answers from the human applied (rounds 1–3).
The protocol text below replaces the draft that was in `/tmp/opencode`; the same content is in section 2 of this document.

Author: codex-1. Proposed M0 contract, subject to human approval. This is not an implemented API.

### 2.1 Transport and endpoints

Only `agentbus hub` listens. Node daemons, per-run MCP servers, interactive commands, and views initiate
requests to that listener. The lead instance's own per-run MCP servers send the same HTTP requests over a private Unix socket served by
the hub daemon. This supersedes the original in-process proposal (resumed round 1 approval): per-run servers
are separate processes. The socket has owner-only permissions, uses the configured local instance identity,
and retains the same registered-run validation as network requests. `HUB_SOCKET` overrides the default
`<bus directory>/hub.sock`; `serve --dir` selects the directory. Local callers must not execute independent
copies of the bus logic against files while hub mode is enabled. A missing or refused socket returns
`Unavailable`; the interim file fallback was removed with explicit resumed round 2 approval.
Without hub configuration, the existing local-file mode and all existing outputs remain unchanged.

Proposed versioned endpoints:

| Method and path | Operation and purpose |
|---|---|
| `POST /v1/tools/{op}` | one route per operation (round 2 answer 3, round 3 answer a): the MCP tool or shared-state CLI operation named `{op}`, from either client kind |
| `POST /v1/instances/register` | `instance_register`: register an instance and initial manifest; return a lease |
| `POST /v1/instances/heartbeat` | `instance_heartbeat`: renew the lease, update manifest and task summaries |
| `POST /v1/commands/poll` | `command_poll`: long poll for commands addressed to the authenticated instance |
| `POST /v1/commands/ack` | `command_ack`: record a command's durable execution result |

Every endpoint operation gets its own errors.json entry, exact causes, generated error type, and tests.
No endpoint exposes arbitrary file access, shell command execution, or a client-selected executable.
Task views, questions retrieval/submission, and command enqueueing use explicit allowlisted operations
through `/v1/tools/{op}`; they do not gain an unrestricted RPC or filesystem interface.

Requests and responses use JSON and a protocol version of 1. All requests authenticate independently
according to section 3, including reused connections and long polls. Authentication precedes body dispatch.
Token mode uses HTTPS pinned by the hub certificate's SHA-256 fingerprint (`HUB_CERT_SHA256`) and the expected peer-address check (section 3.2).
Tailscale mode uses the actual TCP peer address for LocalAPI whois; forwarded headers are never trusted.
Credentials are not forwarded on redirects; clients reject redirects. The hub bounds request sizes,
concurrent polls, header-read time, and idle connections. Response deadlines allow the negotiated poll time.
Exact operational limits are implementation parameters to document and test before M1 completion.

### 2.2 Call envelope and identity

Example request:

```json
{
  "version": 1,
  "request_id": "fc7644e0-efc8-471e-a33b-f735e246dfcc",
  "issued_at": "2026-10-21T20:00:00Z",
  "caller": {
    "agent": "codex-1",
    "instance": "mac",
    "task": "20261021-200000-codex-1",
    "team": "20261021-200000-team",
    "tier": "normal",
    "role": "worker"
  },
  "kind": "tool",
  "op": "send_message",
  "tool": "send_message",
  "args": {"to": "claude-1@linux", "text": "The build is ready."}
}
```

The route and the envelope `op` must match, or the request is rejected as a malformed request (400). `kind` is a closed enum (`tool` or `cli`); `op` is the existing operation name. `tool` is required only
for tool calls and must equal `op`. CLI calls use their existing subcommand names with typed arguments,
never command strings. Request IDs come from `crypto/rand` and remain fixed across retries of one call.
The envelope is internal: MCP clients continue using the existing stdio protocol and tool schemas.
`initialize`, `ping`, and `tools/list` remain local MCP protocol handling. Existing role `worker` covers
normal agents too; `investigator` remains distinct from the normal/worker tier enum.

The hub binds `caller.instance` to the authenticated peer and active instance lease. It qualifies the
agent as `agent@instance`; an unqualified recipient is resolved against the caller's instance, never
the lead instance. Existing `all`, `team`, and `team:<id>` recipient semantics remain distinct.
The hub verifies agent, task, tier, role, and team against registered runs and membership rather than
granting privileges from the envelope alone. Team coordinator authority still comes from `claim_lead`.
Node/admin operations require the relevant instance control identity and explicit authorization.
This protects instance boundaries; processes running as the same instance user remain trusted together.

Repository, branch, and worktree metadata belong to the registered run. A remote worktree path is display
metadata, not a path the hub may open. Code transfer uses origin, not remote paths or RPC file reads.

### 2.3 Results and typed errors

Successful call, preserving the exact existing handler result string:

```json
{"version":1,"request_id":"fc7644e0-efc8-471e-a33b-f735e246dfcc","result":"Sent: ..."}
```

The final response also carries an `unread_count` computed at the hub so the proxy can retain the existing
MCP unread footer without opening local bus files. CLI results use a typed `stdout` byte field encoded
losslessly as base64 JSON, rather than assuming arbitrary output bytes are valid UTF-8 text.

Failed operation:

```json
{
  "version": 1,
  "request_id": "a4a9e453-84c4-4e0f-a20a-afb09219c19e",
  "error": {
    "op": "quiz_start",
    "cause": "NotCoordinator",
    "message": "only the coordinator can do this"
  }
}
```

Exactly one of `result` or `error` is present. Cause strings are generated cause names such as
`NotCoordinator`, not full Go identifiers or numeric enum values. The message excludes the `op: ` prefix;
rebuilding `*QuizStartError` adds that prefix exactly once. Class and exit status come from the client's
generated operation/cause table, never a peer-supplied class or exit code. Existing class exit codes stay
2–10; append `Unavailable` as 11 with its own sentinel. Add an `Unavailable` cause to each forwardable
operation. A remote `QuizStartNotCoordinator` must still support the same `errors.Is` and `errors.As`.

The retry-stable request `kind` also disambiguates error types where an MCP tool and CLI command share
an operation name. In particular, `release` rebuilds `*ReleaseError` for `kind=tool` and `*CmdReleaseError`
for `kind=cli`. The generic operation lookup continues to reject this ambiguous name without a kind.

M1 client limits: request body 1 MiB, response body 64 MiB, total pending lifetime 630 seconds; the default
contiguous outage budget is 30 seconds. Requests never follow redirects. Retry backoff starts at 250 ms,
doubles to one second, and adds up to 99 ms jitter. A successful pending poll resets the outage budget,
while retaining the original request identity, timestamp and total operation deadline.
Underlying operating-system error chains are local and are not serialized.

Extend errgen with per-operation cause-by-name lookups and a dispatcher that rebuilds the concrete
operation error, plus an operation-name accessor for encoding without parsing human messages.
Unknown operation/cause names, wrong result shapes, mismatched request IDs, or an
unsupported protocol version produce a typed transport/protocol failure, never a fabricated known cause
or a successful empty answer. Valid dispatcher errors may name `tool_call` rather than the requested
tool; the client validates that it is an allowed dispatch-stage failure. The protocol error needs its
own errors.json operation/cause and coverage.

Completed application calls return HTTP 200 even when the result contains a typed error. HTTP status codes
describe transport/admission failures: malformed request 400, rejected credentials 401/403, lease or request
conflict 409, oversized request 413, overload 429, hub unavailable 503. Authenticated structured failures
carry endpoint operation/cause/message. A non-JSON proxy error is not decoded as an application error.
Authentication, validation, permission, and version failures are not retried as transient outages.

### 2.4 Retries and the mutation replay ledger

HTTP retries alone can duplicate `send_message`, queue insertion, or board changes when an answer is lost.
Accepted (round 1 answer 9): maintain a durable replay ledger on the hub, keyed by authenticated instance,
run identity, and request ID. Store a hash of the validated operation, caller, and arguments. Reusing an
ID with a different payload is a typed conflict. Never re-execute a completed request: return its stored
result or exact typed error. Concurrent duplicates wait outside the bus lock or receive an in-progress
response; they cannot both enter the handler.

Before executing a mutating operation, durably write its accepted record. After the handler returns,
durably write its response, then send the answer. Durability requires checked writes, file sync, atomic
replacement where used, and directory sync where required. A failure recording acceptance prevents
execution. A failure recording completion leaves the accepted record unresolved and is reported.

**Crash window:** existing `transact` is not a transaction across state.json, messages.jsonl, notebook files,
process side effects, and the replay ledger. A crash after acceptance and before durable completion can
mean either that nothing ran or that effects already happened. On restart such a request returns the
operation's typed `OutcomeUnknown` cause, which has the `InvalidState` class (round 2 answer 2). It is never automatically
re-executed. The caller inspects state/messages or asks its coordinator before intentionally making a
new call with a new ID. This is at-most-one automatic execution, not an exactly-once transaction.
The human accepted this semantics (round 1 answer 9).

An apparently read-only operation can mutate: `read_messages` and `wait_for_message` advance read cursors,
and existing handlers refresh presence and run garbage collection. Replay classification must use actual
effects. In particular, cursor-advancing responses must be cached so a lost answer does not lose delivery.
Pure observational polling can be retried without replaying a mutation; no retry resets its deadline.

Retention must not turn an old request ID into a new operation. Proposed ledger reclamation keeps
unresolved records until explicitly reconciled and rejects requests older than the supported replay
window using `issued_at`, a documented clock-skew allowance, and durable expiry/tombstone information.
The supported window must exceed the maximum retry and long-poll durations. Exact retention limits
remain part of the approval/design completion, not an unbounded claim of permanent deduplication.

Clients retry only transient connection failures, transport timeouts, and retryable HTTP responses, using
bounded backoff with jitter. The elapsed retry budget is `HUB_RETRY_SECONDS`, default 30 seconds (round 2 answer 5).
Dial, TLS, request, and backoff time all count; a blackholed TCP connection cannot exceed the budget.
On exhaustion, return the attempted operation's `Unavailable` cause. A lost mutation response retains
its original ID throughout retry; no fallback ever executes it against local files.
If the server reports `OutcomeUnknown`, do not transform it into `Unavailable` or retry execution.

M1 limitation: retry identity is stable within one MCP call. A subsequent MCP call creates a new ID;
there is no durable client outbox yet. After `Unavailable` or `OutcomeUnknown`, the caller must check the
effect with the operation's read tool before resending a mutation, because a new call may repeat a write.
The unavailable reply includes this instruction and the original ID. Some operations have no sufficient
read-side reconciliation, so such calls need human investigation rather than automatic replacement work.

### 2.5 Long polling and command delivery

`wait_for_message` travels through `/v1/tools/wait_for_message` and runs on the hub. Poll chunks are 25 seconds (round 2 answer 5);
the original tool's total wait remains capped at its existing 600-second limit. Continuation carries the
original deadline and call identity; an empty chunk is transport state and must not become an early MCP
timeout answer. A third mutually exclusive response field, `pending`, signals this continuation (instead
of `result` or `error`); final responses retain the disjoint result/error contract above. Empty chunk
handling releases the HTTP connection without busy polling. Cancellation
stops further waiting but cannot promise rollback of a cursor update or other completed mutation.
No bus lock is held across sleeps, network writes, or client disconnection.
Apply the same bounded continuation to other blocking tools such as reserve, quiz_results, and queue waits.

The node long polls `/v1/commands/poll` with its lease and last acknowledged command cursor. An empty
25-second poll returns 204; otherwise the hub returns a command batch. Commands carry stable IDs,
instance/lease generation, typed kind, task/run identity, and validated parameters. They are persisted
before delivery and remain pending until acknowledgment. Delivery is at least once.

The node keeps a durable command journal and acknowledges with an execution result. Repeated `start_run`
must identify the same task/run and reconcile its existing runner rather than launch a second process.
`wake`, `stop_task`, and answer delivery similarly require idempotent handlers and local result records.
The command journal must address the crash between spawning a process and recording its result; it
cannot claim exactly-once process launch from an acknowledgment alone. Unknown command kinds fail
explicitly. Enqueueing work for an expired instance fails with `InstanceUnreachable`.

A live lease rejects a second daemon with the same instance name. The instance name in registration must match the authenticated peer identity (for Tailscale: the verified node name or tag authorization; for token mode: the instance name associated with the configured token/address). A peer cannot register under or claim another instance's name. Lease ownership is strictly bound to the authenticated node identity, and reconnecting with a mismatched identity is rejected with `InstanceNotAllowed`. Heartbeats and polls require the lease
generation; reconnecting the same daemon renews its lease, while replacing an expired lease fences stale
heartbeats, command acknowledgments, and polls. A member whose board claim was released during an outage
must reacquire it before resuming that item; reconnection cannot revive the old exclusive claim.
Heartbeats run independently of polls and tools. Reports carry monotonic sequences so an older heartbeat
cannot overwrite newer task state, and hub time determines expiry. Command order is preserved per task:
a delayed start command must not undo a later stop.

### 2.6 Helper commands, questions, views, and watchdog

Forward every shared-bus CLI path through the same authenticated envelope: `send`, `log`, `status`,
`release`, `team-get`, `inv-next`, helper queue operations, notices, and state-changing runner callbacks.
Keep their stdout bytes and existing exit statuses. Local-only event normalization stays local. Merely
forwarding MCP calls while runners read local team/queue state would split one team into several buses.
Remote inv/helper dispatch returns a node command, never an instruction to open the requester's worktree
path on the lead host. Aggregated views read task summaries from the hub.

`agent answer` retrieves a versioned questions snapshot through the hub and submits answers with the task,
question round, and snapshot version; stale submissions fail explicitly. The hub durably queues delivery
to the coordinator's instance. The destination validates identifiers and constructs the local path itself.
Member questions go only to the coordinator; the coordinator broadcasts the full answer set through the
bus. A solo task retains its own questions flow. Notifications include the originating instance.

Local structured outage state records when hub communication becomes unavailable and recovers. The
watchdog excludes only the affected outage interval from hang accounting and resumes its prior inactivity
budget on recovery. It must not reset inactivity indefinitely from repeated failures or treat an unrelated
hung build as evidence of an outage. Outage tracking works for bounded failures of both `serve` and node
and survives the return of an `Unavailable` tool error. No change to local-only watchdog behavior.

### 2.7 Verification and compatibility

Use in-process httptest TLS servers, a stub LocalAPI Unix socket, fake clocks where appropriate, and
isolated temporary homes. Cover every endpoint/cause, exact typed-error round trips, deadline bounds,
lost-response replay, crash-window outcome unknown, cursor delivery, stale lease fencing, duplicate names,
wrong token/address, unknown node/tag, long-poll cancellation, and command redelivery. No real agents or
Tailscale are needed. Check helper CLI stdout/exit-code compatibility and unchanged local-only behavior.
Add legacy fixtures for every persisted type change and include ledger/command state in coherent backups;
restoring only state.json can otherwise re-enable already executed operations.


## 3. Authentication modes

Instances connect to the lead instance (hub). The hub never initiates connections to non-lead instances. Connections are authenticated using one of two modes configured via `HUB_AUTH`: **`tailscale`** (default) or **`token`**.

### 3.1 Tailscale mode (`HUB_AUTH=tailscale`, default)

Tailscale mode is the default and recommended authentication mechanism. It eliminates shared secrets on disk, provides cryptographic node identity via WireGuard, and requires zero extra credentials.

#### Mechanism
1. The hub listens on its Tailscale address (`HUB_LISTEN=tailscale` or explicit IP/port).
2. When an instance or tool call connects over TCP, the hub retrieves the client's `RemoteAddr` (`<ip>:<port>`).
3. The hub queries the local `tailscaled` daemon via its LocalAPI Unix Domain Socket (`/var/run/tailscale/tailscaled.sock` on Linux, `/var/run/tailscaled.socket` on macOS):
   ```http
   GET /localapi/v0/whois?addr=<ip>:<port> HTTP/1.1
   Host: local-tailscaled
   ```
   Implemented in Go using the standard library `net.Dial("unix", ...)` and `http.Client`. No external Tailscale Go modules or CLI commands are used.
4. The Tailscale daemon returns the peer's node identity:
   - Node FQDN (e.g. `mac.example.ts.net`) and base node name (`mac`).
   - Node tags (e.g. `["tag:agent-node", "tag:ci"]`).
   - User profile identity.
5. The hub validates the peer against `ALLOWED_INSTANCES`:
   - An entry in `ALLOWED_INSTANCES` may be a bare node name (e.g. `mac`, `pi5`), an exact FQDN, or a tag (e.g. `tag:agent-node`).
   - The instance name declared during registration (`instance_name`) must match the authenticated Tailscale node name (or be authorized by the matching node's tag).
6. The connection and its subsequent requests are bound to this verified instance identity.

#### Security Properties
- **Zero Secrets on Disk:** No API keys or tokens are stored on the worker instances.
- **Agent Isolation:** Even if an unprivileged agent explores the filesystem or environment, it cannot extract credentials to impersonate another instance or connect from outside the tailnet. WireGuard private keys are managed by root/`tailscaled`.
- **ACL Enforcement:** Tailscale ACLs should restrict instances so they can reach only the hub's port (`HUB_PORT`, default 7777) and no other ports on the lead host.

---

### 3.2 Token mode (`HUB_AUTH=token`)

For deployments where Tailscale is not available, Token mode provides mutual authentication using pre-shared tokens over TLS.

#### Mechanism
1. **Transport Layer Security (TLS):** Token mode **strictly requires TLS**. The hub serves HTTPS on its configured listen address. Unencrypted plaintext HTTP in token mode is rejected on startup.
2. **Hub Configuration:** The hub maintains a mapping of allowed instances and their pre-shared tokens in `secrets.env`:
   ```bash
   HUB_TOKEN_mac="secret-token-value-for-mac"
   HUB_TOKEN_pi5="secret-token-value-for-pi5"
   ```
   And `agents.conf` defines allowed instances with the address each must connect from (a token from any other
   source address is rejected):
   ```bash
   ALLOWED_INSTANCES=(mac=192.0.2.20 pi5=192.0.2.10)   # name=expected source address; token of each name in secrets.env
   ```
3. **Instance Configuration:** Non-lead instances store the token in `secrets.env`:
   ```bash
   HUB_TOKEN="secret-token-value-for-mac"
   ```
   And in `agents.conf`:
   ```bash
   LEAD_INSTANCE="hub.example.com:7777"   # host:port, the same form in both modes; token mode always uses TLS
   HUB_AUTH=token
   ```
4. **Header Authentication:** Each HTTP request or registration stream passes the token in the `Authorization` header:
   ```http
   Authorization: Bearer <HUB_TOKEN>
   X-Instance-Name: mac
   ```
5. **TLS Certificate Setup (round 1 answer 5: self-signed, pinned by fingerprint):**
   - The hub uses a self-signed certificate and key (`HUB_TLS_CERT`, `HUB_TLS_KEY`), created once on the hub
     with a documented command (for example `openssl req -x509 -newkey ec -nodes -days 3650 ...`).
   - Each non-lead instance pins the SHA-256 fingerprint of that certificate in its `agents.conf`:
     ```bash
     HUB_CERT_SHA256="<64 hex digits>"
     ```
     The client accepts only a connection whose certificate has this fingerprint. No CA is operated; the
     pin replaces the system trust store and the hostname check.
   - Rotating the certificate: update `HUB_CERT_SHA256` on every instance first, then replace the hub's
     certificate. A mismatch is a configuration error, never a silent fallback.

#### Security Caveat (Token Mode)
> [!WARNING]
> **Token Readability by Agents:** In Token mode, the token is stored in `secrets.env` (or environment variables) on the instance. Because agents running on that instance execute under the same user account (or have worktree access), an agent CLI can inspect `secrets.env` and read `HUB_TOKEN`. Token mode must only be used in trusted internal networks where instance identity isolation between agents is not a primary security boundary.

---

## 4. Configuration (lead and other instances)

New variables in `config/agents.conf` (all have defaults; a file without them is a single-instance installation):

| Variable | Lead | Other | Meaning |
|---|---|---|---|
| `INSTANCE_NAME` | required | required | unique name of the instance (`[a-z][a-z0-9-]*`, must not contain `@`) |
| `HUB_LISTEN` | `tailscale` or `addr:port` | — | where the hub listens (`tailscale`: the tailnet address and `HUB_PORT`) |
| `HUB_PORT` | `7777` | — | port of the hub |
| `HUB_AUTH` | `tailscale` (default) or `token` | `tailscale` or `token` | authentication mode (section 3) |
| `ALLOWED_INSTANCES` | array: node names or `tag:…` (tailscale); `name=addr` (token) | — | instances the hub accepts |
| `LEAD_INSTANCE` | — | `host:port` of the hub | where `agent node` connects |
| `HUB_TOKEN` | — (per-instance tokens live in `secrets.env`) | `secrets.env` | token of this instance (token mode only) |
| `HUB_BACKUP_DIR` | `~/agents/bus/backup` | — | where the hub writes periodic state backups (section 8) |
| `HUB_RETRY_SECONDS` | `30` | `30` | how long `serve` retries an unreachable hub before returning `Unavailable` |
| `HUB_TLS_CERT`, `HUB_TLS_KEY` | file paths (token mode, required) | — | TLS certificate and key of the hub (section 3.2) |
| `HUB_CERT_SHA256` | — | 64 hex digits (token mode) | SHA-256 fingerprint of the hub's self-signed certificate; the only trust anchor in token mode (round 1 answer 5) |
| `HUB_BACKUP_MINUTES` | `30` | — | interval of hub state backups |
| `MANIFEST_TTL` | `180` (seconds) | — | how long the hub keeps a manifest without heartbeat (section 5.1) |
| `INSTANCE_CAPABILITIES` | array of typed capabilities | same | declared capabilities (section 5.3); the name follows decision 11 wording |
| `INSTANCE_ACCEPT_REPOS` | array (optional) | same | local policy: repositories this instance accepts work for |
| `PAUSE_ON_BATTERY` | `false` | `false` | laptops: pause new runs on battery (section 13.5) |

Token values never go into `agents.conf`; they are read from `secrets.env`: on the lead one `HUB_TOKEN_<name>`
per allowed instance (for example `HUB_TOKEN_mac`, the instance name with `-` replaced by `_`), on every other
instance `HUB_TOKEN`.

Example (shape only; the final form is the table above):

```bash
# lead instance (pi5)
INSTANCE_NAME=pi5
HUB_LISTEN=tailscale
HUB_AUTH=tailscale
ALLOWED_INSTANCES=(i1 mac tag:team-agents)

# other instance (mac)
INSTANCE_NAME=mac
LEAD_INSTANCE=pi5.example.ts.net:7777
HUB_AUTH=tailscale
```

Rules:
- An unknown capability in `INSTANCE_CAPABILITIES` or a duplicate `INSTANCE_NAME` in the allowed list is a
  configuration error: `agent` refuses to start and names the value.
- A non-lead instance without `LEAD_INSTANCE` is a single-instance installation (no `agent node`).
- Changing `INSTANCE_NAME` on an instance that has tasks is refused; the name is part of every qualified name.

## 5. Manifest and capability vocabulary

Each non-lead instance runs the `agent node` daemon. Upon starting, and continuously during operation, `agent node` reports its capabilities, hardware status, and agent capacity to the hub.

### 5.1 Heartbeat and TTL Lifecycle

1. **Registration:** On startup, `agent node` connects to the hub (`POST /v1/instances/register`) and sends the initial manifest. The hub assigns/confirms a registration lease and records the instance as active.
2. **Periodic Heartbeat:** Every 60 seconds, `agent node` sends an updated manifest (`POST /v1/instances/heartbeat`).
3. **Lease TTL:** Every manifest carries a TTL (default: 180 seconds, or 3 missed heartbeats).
4. **Expiration (`unreachable`):**
   - If no heartbeat is received within the TTL window, the hub marks the instance state as `unreachable`.
   - The coordinator of any active team with members on the unreachable instance is immediately notified via bus event.
   - Any board items claimed by members on the unreachable instance are automatically released back to the board (`board_release`).
   - If the instance re-establishes its heartbeat before its tasks are terminated, its status returns to `active` and members resume execution.

---

### 5.2 Manifest JSON Schema

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "InstanceManifest",
  "type": "object",
  "required": [
    "instance_name",
    "timestamp",
    "ttl_seconds",
    "detected",
    "declared",
    "current_state"
  ],
  "properties": {
    "instance_name": {
      "type": "string",
      "description": "Unique identifier of the instance (e.g. 'mac', 'pi5')"
    },
    "timestamp": {
      "type": "string",
      "format": "date-time",
      "description": "RFC3339 timestamp when the manifest was generated"
    },
    "ttl_seconds": {
      "type": "integer",
      "default": 180,
      "description": "Time-to-live in seconds before the hub marks instance as unreachable"
    },
    "detected": {
      "type": "object",
      "required": ["os", "arch", "toolchains"],
      "properties": {
        "os": { "type": "string", "enum": ["linux", "darwin"] },
        "arch": { "type": "string", "enum": ["amd64", "arm64"] },
        "toolchains": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Discovered toolchains (e.g. ['go1.23', 'rust1.82', 'node20'])"
        },
        "xcode_version": {
          "type": "string",
          "description": "Xcode version string if present (e.g. '16.1')"
        },
        "ios_simulators": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Installed iOS simulator runtimes (e.g. ['iPhone 16 iOS 18.1'])"
        },
        "hardware_probes": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Attached hardware debug probes (e.g. ['rp2350-probe', 'jlink'])"
        }
      }
    },
    "declared": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Explicit capabilities declared in config/agents.conf"
    },
    "current_state": {
      "type": "object",
      "required": ["agents", "slots", "subscriptions"],
      "properties": {
        "slots": {
          "type": "object",
          "required": ["total", "free"],
          "properties": {
            "total": { "type": "integer" },
            "free": { "type": "integer" }
          }
        },
        "agents": {
          "type": "array",
          "items": {
            "type": "object",
            "required": ["name", "tier", "role", "status"],
            "properties": {
              "name": { "type": "string" },
              "tier": { "type": "string", "enum": ["normal", "worker"] },   // an empty legacy tier is read as "normal", as in agentbus/types.go; no legacy value is newly rejected
              "role": { "type": "string" },
              "status": { "type": "string", "enum": ["idle", "busy", "sleeping"] },
              "current_task": { "type": "string" }
            }
          }
        },
        "subscriptions": {
          "type": "object",
          "additionalProperties": {
            "type": "object",
            "required": ["exhausted"],
            "properties": {
              "exhausted": { "type": "boolean" },
              "exhausted_until": { "type": "string", "format": "date-time" }
            }
          },
          "description": "Per-model or provider quota exhaustion state"
        }
      }
    }
  }
}
```

---

### 5.3 Typed Capability Vocabulary and Routing

Capabilities allow teams to route work to the instance with the appropriate hardware, SDKs, or tools.

#### Namespace Structure
Capabilities are typed strings formatted as `<namespace>:<identifier>`. Namespaces are strictly enforced:

| Namespace | Meaning | Examples |
|---|---|---|
| `os:` | Operating system family | `os:linux`, `os:darwin` |
| `build:` | Compilation and toolchain abilities | `build:ios`, `build:android`, `build:rust`, `build:go`, `build:c` |
| `test:` | Test runtime environments | `test:ios-simulator`, `test:android-emulator`, `test:docker` |
| `hw:` | Physical devices and CPU architectures | `hw:rp2350-probe`, `hw:jlink`, `hw:apple-silicon`, `hw:x86-64` |
| `model:` | Specific model provider access | `model:claude-3-7-sonnet`, `model:codex`, `model:gemini-flash` |
| `sign:` | Code signing keys and entitlements | `sign:apple-development`, `sign:apple-distribution`, `sign:android-keystore` |
| `publish:` | Publishing packages and releases (approval-gated like `sign:*`, round 1 answer 7) | `publish:npm`, `publish:pypi`, `publish:github-release` |

#### Configuration Validation Rule
Capabilities must belong to the closed set of recognized capability identifiers defined in `agentbus/types.go`.
- If an unknown or malformed capability appears in `INSTANCE_CAPABILITIES` in `agents.conf`, `agent node` and `agentbus` **fail fast on startup** with a configuration error.
- Typos (e.g. `build:iso` instead of `build:ios`) are immediately caught at configuration time.

#### Security Approval for Sensitive Capabilities
Certain capabilities represent sensitive actions or security boundaries:
- `sign:*` (cryptographic signing of binaries).
- `publish:*` (publishing packages to npm, PyPI, or releases).
- **Rule:** Tasks requiring these sensitive capabilities cannot execute automatically without human approval. If an agent claims a task or board item requiring `sign:*`, the instance prompts the human in the loop for approval before granting execution.

#### Work Routing
1. **Board Items (`requires`):** Items on the board can specify an optional `requires` array:
   ```json
   {
     "id": "B12",
     "level": "normal",
     "description": "Compile and run iOS smoke test in simulator",
     "requires": ["build:ios", "test:ios-simulator"]
   }
   ```
   Only members running on instances that possess all listed capabilities may claim the item (`board_claim`).
2. **Worker Delegation & Investigations:** `delegate` and `request_investigation` route to instances that meet the required capability and have free capacity, prioritizing instances where model subscriptions are not exhausted.
3. **Capability Discovery Tool (`find_capability`):**
   Agents can query the bus via the `find_capability` MCP tool to locate instances and agents with desired capabilities:
   ```json
   {
     "capability": "build:ios"
   }
   ```
   Returns matching instances, active agents, and whether their capacity is free.

## 6. Command channel

- `agent node` opens a long-poll connection to the hub after registration (section 2). The hub answers the poll
  with the next queued command for that instance or with `204` after the poll timeout (25 s).
- Commands (typed; each carries an id so that a retry is idempotent):
  - `start_run` — task id, agent, run number, prompt file reference (the prompt is rendered by the node from the
    task's own files; the hub sends the parameters, not the text);
  - `wake` — qualified member name and the reason (message or board item);
  - `stop_task` — task id and reason;
  - `answer_deliver` — task id and the answer text (section 7).
- The node acknowledges every command with its result (`started`, `already_running`, `stopped`, `unknown_task`,
  `failed:<reason>`). A command for an instance that is gone (manifest expired) is not queued: the hub returns
  the typed cause `InstanceUnreachable` to the caller and does not keep the command (section 8).
- The node reports task state (status, run number, last activity time) with each heartbeat, so the hub can answer
  `agent status` for all instances without asking the node.

## 7. Questions flow

Decisions 16–20, made concrete:

1. A member's `STATUS: QUESTIONS` or a `send_message` to the coordinator goes through the hub. The hub stores the
   message with the sender's qualified name (`codex-1@mac`).
2. The coordinator's team task collects the questions in its questions file. Each question is annotated with the
   qualified name of its author: `## 3. codex-1@mac: Should the iOS app support iOS 17?`. The file format is the
   existing one (section 8 of the team protocol), only the header text changes.
3. `agent answer <task>` runs on any instance. The answer is sent to the hub, which routes it to the coordinator's
   task (command `answer_deliver` to the instance that runs the coordinator). The answer is written into the
   coordinator's questions file, as today.
4. When the coordinator forwards the answers to the members, it sends the full set of answers (all questions,
   with their authors) to every member. Members receive the set through the bus (`send_message` to each member or
   `team`).
5. A solo task on a remote instance asks from its own instance, as today (decision 20). The notification names
   the instance (`[mac] task 20261021-…: questions`).

## 8. Failure modes

| Situation | Behaviour |
|---|---|
| Hub unreachable from an instance | `serve` retries for `HUB_RETRY_SECONDS` (bounded), then returns `Unavailable` to the agent. Local work on code (edits, commits, tests) continues. The agent may retry later. |
| Hub unreachable for `agent node` | node keeps running, retries with backoff, keeps its running agents going; after reconnect it re-registers and sends the current task state. |
| Hub restarts | state files are the source of truth (as today); the hub reloads them on start; nodes re-register; no command is lost, because commands are persisted in `bus/commands.json` before they are sent. |
| Instance manifest expires (no heartbeat for `MANIFEST_TTL`, default 180 s) | the instance is **unreachable**: its members become `unreachable`, the claimed board items return to the board, the coordinator receives a message naming the instance. |
| Instance returns | it re-registers; its members go back to their previous state (`active`/`sleeping`) and resume. |
| Command for a gone instance | typed error `InstanceUnreachable`; no queueing. |
| Hang watchdog | a hub outage is not a hang: while `serve` reports `Unavailable` or the node reports a hub outage, the watchdog does not count the run as hung. |
| Duplicate instance name | the hub rejects the second connection with `InstanceNameTaken`. |
| Instance heartbeat | every 60 s; a manifest is valid for `MANIFEST_TTL` (180 s, three missed heartbeats), see section 5.1. |
| Rejected peer (wrong token, unknown node or tag) | the hub rejects the registration with `InstanceNotAllowed`; no manifest is accepted, nothing is queued. |
| Hub state backup | the hub writes a coherent snapshot of `state.json`, `messages.jsonl`, and mutation replay ledger / command state to `HUB_BACKUP_DIR` every `HUB_BACKUP_MINUTES` (default 30) under the same lock; the last 10 backups are kept. |

## 9. Security

- **The hub is the most valuable target.** It holds the state of every instance, every message and every queued
  command. Its listening address is only the configured tailnet address (or the configured address in token mode).
- **Tailscale ACLs** should let the instances reach only the hub's port (for example
  `tcp:7777` from `tag:agent-instance` to `tag:agent-lead`). No other port of the hub is opened.
- **Identity.** In tailscale mode the identity of a peer is the node name or tag reported by `tailscaled` for the
  source address of the connection. Agents cannot read the node key, so an agent cannot impersonate an instance
  by copying a secret. In token mode the identity is the token, and **the agents of an instance can read its
  token** (they run as the same user); a token therefore identifies the instance, not a person.
- **Per-instance policy.** Each instance accepts only its own repositories (`INSTANCE_ACCEPT_REPOS`) and its own
  capabilities. The hub never makes an instance run work for a repository or capability it did not accept.
- **Approval of sensitive capabilities.** Capabilities in `sign:*` and publishing require the human's approval on
  the instance that would run the work, before the instance runs it. Approval is recorded locally (section 12,
  question 3).
- Secrets stay on their instance (`secrets.env`). One exception: in token mode each instance sends its own token in the
  `Authorization` header to the hub. The hub keeps the expected tokens in its `secrets.env` and never forwards a token
  to another instance.

## 10. Compatibility with existing installations

- **Single-instance mode is unchanged.** No new variable is required. Without `LEAD_INSTANCE`/`HUB_LISTEN` the
  bus uses the local files and every existing test runs unchanged.
- **`bus/state.json`.** The new member state `unreachable` and the new board field `requires` are additions. Old
  files load unchanged (`testdata/state-legacy.json` keeps loading, `TestLegacyStateLoads`). A new fixture of the
  format before the change and a test that loads it are required for each change of a persisted type.
- **`bus/commands.json`** (new, hub only) and **`bus/instances.json`** (new, hub only) are created on demand.
- **Subcommand output and exit codes** of `agentbus` that `bin/agent` parses stay as they are. New subcommands
  are additions.
- **MCP tool names and arguments** stay. New tools (capability discovery, `requires` on board items) are
  additions.
- **Log events** stay; new events are added to the `LogEvent` list with a comment.
- **Errors on the wire** are new; the local error types and messages stay.
- **macOS** is new; the Linux path stays as it is. Platform-specific code is isolated behind one platform check.

## 11. Milestones

- M0 design (this document) — approval by the human.
- M1 hub mode: `agentbus hub`, forwarding in `serve`, identity per request, typed errors over the wire (errgen),
  `Unavailable`, long polling, both authentication modes.
- M2 instances: `agent node`, registration, manifest and heartbeat, command channel, qualified names,
  cross-instance teams, questions flow (decisions 16–20), `agent answer` from any instance.
- M3 capabilities: detection and declaration, `requires` on board items, routing of `delegate` and
  investigations, discovery tool.
- M4 macOS port.
- M5 views and robustness: aggregated `agent live`/`agent status`, backups, `unreachable` handling, README,
  templates, Security section.

## 12. Open decisions and recorded consensus

### Current task status
This document is submitted to the human for M0 design approval in round 1 of task `20261022-154419-gemini-1`.
No implementation code will proceed until the human approves this design via `questions-1.md`.

### Summary of architectural consensus
1. Lead platform: Hub is Linux-only; macOS instances connect as non-lead instances.
2. Read-only instances: Allowed; instances without runners can participate in bus communications.
3. Sensitive capabilities: Persistent local approval list in instance state, managed via `agent approve <capability>`.
4. Token TLS: Self-signed certificate pinned by SHA-256 fingerprint (`HUB_CERT_SHA256`) in `agents.conf`.
5. Timings: `MANIFEST_TTL=180s`, `HUB_BACKUP_MINUTES=30m`, poll timeout 25s, retry budget 30s.
6. Namespaces: Includes `publish:` namespace, approval-gated alongside `sign:*`.
7. `agent node`: Implemented in Go as an `agentbus` subcommand.
8. Retries and replay: Explicit replay ledger with `OutcomeUnknown` (class `InvalidState`) for unrecoverable crash windows.
9. Endpoints: Unified `/v1/tools/{op}` endpoint for MCP tools and CLI operations.
10. Lead-local routing: Local runners on the lead instance talk to the hub over a private Unix domain socket.


## 13. macOS port (M4 design)

(Author: gemini-1; coordinator adds the open points to section 12.)

To enable macOS nodes (such as Mac mini build machines or developer MacBooks) to participate as team-agents instances, `bin/agent` and node execution are adapted to macOS conventions.

### 13.1 Daemon Supervision via launchd

On Linux, `systemd --user` runs background transient units (`systemd-run --user`). On macOS:
- An unprivileged user cannot bootstrap transient jobs into `launchd` domains.
- Per-task launchd plists are not viable.

#### Architecture:
1. **LaunchDaemon:** A system LaunchDaemon (`/Library/LaunchDaemons/com.team-agents.node.plist`) runs `agent node` at system boot under the dedicated user account (e.g. `UserName: dev`).
2. **Process Management:** `agent node` acts as the process supervisor for all agent runs on that machine:
   - Spawns runner processes (`bin/agent _runner`) directly using POSIX process groups (`setpgid`).
   - Configures runner priority: sets `nice -n 10` and background I/O scheduling where supported.
   - Monitors execution, handles signals, streams logs, and enforces timeouts.
   - Cleans up child processes on cancellation (`SIGTERM` followed by `SIGKILL` to process group).

---

### 13.2 BSD vs. GNU Tool Portability

The macOS base system provides BSD core utilities rather than GNU coreutils. All scripts in `bin/agent` must either use portable POSIX constructs or platform-conditional abstractions:

| Utility | GNU Syntax (Linux) | BSD Syntax (macOS) | Portable Solution in `bin/agent` |
|---|---|---|---|
| In-place edit | `sed -i 's/foo/bar/' file` | `sed -i '' 's/foo/bar/' file` | Helper `sed_inplace 's/foo/bar/' file` detecting OS or using temp file |
| File timestamp | `stat -c %Y file` | `stat -f %m file` | Helper `file_mtime file` |
| Date parsing | `date -d @1700000000` | `date -r 1700000000` | Helper `format_timestamp ts` or use `agentbus` Go helper |
| Symlink resolution | `readlink -f path` | `readlink` (no `-f` on older BSD) | Portable shell loop or Python/Go one-liner |
| Temporary files | `mktemp -t prefix.XXXXXX` | `mktemp -t prefix` | Standardize on `mktemp "${TMPDIR:-/tmp}/prefix.XXXXXX"` |
| PCRE grep | `grep -P pattern` | Not supported | Use extended regex `grep -E` or `awk` |
| File locks | `flock` utility | Not installed by default | `agentbus` handles locking internally via Go `syscall.Flock` |

---

### 13.3 Credentials and Keychain

- On macOS, the login Keychain (`login.keychain-db`) is locked unless an interactive graphical user session is unlocked.
- Running headless under a LaunchDaemon or SSH means Keychain queries will prompt or hang.
- **Rule:** In `team-agents`, credentials (API tokens, GitHub tokens, Tailscale auth keys) must **never** be looked up in the macOS Keychain. All credentials are read strictly from `config/secrets.env` (file mode `0600`) and the environment.

---

### 13.4 GUI Session Requirements for iOS Development

Certain macOS capabilities require an active, logged-in Aqua GUI window server session:
- **iOS Simulator (`test:ios-simulator`):** `xcrun simctl` can boot simulators headlessly, but UI test execution (`xcodebuild test`) and Simulator rendering require an active window server session.
- **Apple Code Signing (`sign:apple`):** Interactive keychain prompts fail without a GUI session.
- **Setup candidates (not yet requirements):** which of these a dedicated Mac build node needs is decided by the manual
  verification in M4 (report section "manual verification"), not assumed here: (a) auto-login for the agents user,
  (b) a virtual or dummy display so that the window server runs, (c) fast user switching or a locked screen to keep
  the GUI session alive without physical access.

---

### 13.5 Laptop Power and Battery Awareness

For instances running on Mac laptops (MacBook Pro / Air), running intensive builds on battery drains power and degrades performance.

1. **Detection:** `agent node` checks battery status using `pmset -g batt` (macOS) or `/sys/class/power_supply` (Linux).
2. **Behavior (`PAUSE_ON_BATTERY=true` in `agents.conf`):**
   - When AC power is disconnected and battery drops below threshold (e.g. < 50%):
     - Stop pulling new board items.
     - Save a checkpoint on all running tasks (`checkpoint`).
     - Mark instance capacity as `busy` / `paused` in the heartbeat manifest so the hub routes work elsewhere.
   - When AC power is restored:
     - Automatically resume normal operation and manifest capacity.
