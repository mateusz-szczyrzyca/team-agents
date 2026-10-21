# Multi-instance team-agents (design)

Status: **DRAFT for human approval (milestone M0).** No implementation code is written before the human approves
this document. Open decisions are listed in section 12 and are asked through the questions file.

Terminology (decision 5): the **lead instance** is the instance that runs the **hub** (the code name). "Coordinator"
is only the role of an agent in a team (`claim_lead`). The lead instance is never called a coordinator.

Sections written by team members are marked with their author. Everything else is written by the coordinator
(openrouter-haiku).

## 1. Architecture

```
            ┌────────────────────── lead instance (e.g. pi5) ──────────────────────┐
            │  agentbus hub  (HTTP, listens on HUB_LISTEN)                         │
            │   ├─ executes bus logic on its own state files (transact / flock)    │
            │   ├─ instance registry, manifests, command queues                    │
            │   └─ backups of state (section 8)                                    │
            │  local agents: claude-1@pi5 … (their agentbus serve calls the hub    │
            │                 in-process, same code path as remote)               │
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
- **Local instance without a lead.** With no `LEAD_INSTANCE` and no `HUB_LISTEN`, `agentbus serve` uses the local
  files exactly as today (decision 2). The hub mode is a switch in configuration, not a different code base.
- **The lead instance runs agents too** (decision 7). Its local `agentbus serve` calls the hub in-process through
  the same forwarding interface (no HTTP hop), so there is one code path for "call the bus".
- **`agent node`** runs on every non-lead instance (section 6). It is the only process that connects to the hub
  for control: registration, manifest, commands and task state. Only the hub listens (decision 8).

## 2. HTTP protocol

(Author: codex-1 — to be merged from `docs/multi-instance-protocol.md`.)

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
   And `agents.conf` defines allowed instances:
   ```bash
   ALLOWED_INSTANCES=(mac pi5)
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
5. **TLS Certificate Setup:**
   - The hub must be configured with a valid TLS certificate and key (`HUB_TLS_CERT` and `HUB_TLS_KEY`).
   - Certificates may be issued by Let's Encrypt / public ACME, an internal organizational PKI, or a dedicated private CA.
   - Non-lead instances must trust the hub's certificate either via the system trust store or via a pinned CA file configured in `agents.conf`:
     ```bash
     HUB_CA_CERT="/home/dev/agents/config/hub-ca.crt"
     ```

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
| `HUB_RETRY_SECONDS` | `60` | `60` | how long `serve` retries an unreachable hub before returning `Unavailable` |
| `HUB_TLS_CERT`, `HUB_TLS_KEY` | file paths (token mode, required) | — | TLS certificate and key of the hub (section 3.2) |
| `HUB_CA_CERT` | — | file path (token mode, optional) | CA or pinned certificate that the instance trusts for the hub |
| `HUB_BACKUP_MINUTES` | `30` | — | interval of hub state backups |
| `MANIFEST_TTL` | `180` (seconds) | — | how long the hub keeps a manifest without heartbeat (section 5.1) |
| `INSTANCE_CAPABILITIES` | array of typed capabilities | same | declared capabilities (section 5.3); the name follows decision 11 wording and replaces `DECLARED_CAPABILITIES` used in section 5 |
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
              "tier": { "type": "string", "enum": ["normal", "worker"] },
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

#### Configuration Validation Rule
Capabilities must belong to the closed set of recognized capability identifiers defined in `agentbus/types.go`.
- If an unknown or malformed capability appears in `DECLARED_CAPABILITIES` in `agents.conf`, `agent node` and `agentbus` **fail fast on startup** with a configuration error.
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
  with the next queued command for that instance or with `204` after the poll timeout (30 s).
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
| Hub state backup | the hub writes `state.json` and `messages.jsonl` to `HUB_BACKUP_DIR` every `HUB_BACKUP_MINUTES` (default 30) under the same lock; the last 10 backups are kept. |

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
- Secrets stay on their instance (`secrets.env`); they are never sent through the hub.

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

## 12. Open decisions (asked of the human)

Answers are recorded here after approval.

1. Can the lead instance itself be a macOS machine, or is the hub Linux-only?
2. Is `agent node` required on every non-lead instance, or can a non-lead instance run in a read-only mode
   without a node (no runs, no manifest)?
3. How is the human's approval of `sign:*` and publishing capabilities given: a persistent local approval list
   (`approved-capabilities` in the instance's state, changed by a command), or per run?
4. Token mode TLS: a self-signed certificate pinned by fingerprint in `agents.conf` of the instances, or a CA you
   operate?
5. `MANIFEST_TTL` (180 s) and `HUB_BACKUP_MINUTES` (30 min) — approve or change.
6. The capability namespaces in decision 11 are `os:`, `build:`, `test:`, `hw:`, `model:`, `sign:`. The design
   also proposes `publish:` (publishing packages and releases, which decision 23 names as an approval-gated
   capability). Add the `publish:` namespace, or treat publishing as `sign:*` only?
7. `agent node` implementation: (a) a Go subcommand of `agentbus` (standard library only; can supervise runners
   with process groups), or (b) a Bash daemon in `bin/agent` (no new binary; process-group handling is harder
   and macOS and Linux differ).
8. Retries of mutating MCP calls (proposed by codex-1, section 2): when the hub crashes after an operation but
   before the answer is delivered, the replay ledger returns the stored answer. A crash between execution and the
   ledger write is an explicit "outcome unknown" error; the operation is not re-executed automatically. Accept
   this semantics?

## 13. macOS port (M4 design)

(Author: gemini-1; coordinator adds the open points to section 12.)

To enable macOS nodes (such as Mac mini build machines or developer MacBooks) to participate as team-agents instances, `bin/agent` and node execution are adapted to macOS conventions.

### 7.1 Daemon Supervision via launchd

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

### 7.2 BSD vs. GNU Tool Portability

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

### 7.3 Credentials and Keychain

- On macOS, the login Keychain (`login.keychain-db`) is locked unless an interactive graphical user session is unlocked.
- Running headless under a LaunchDaemon or SSH means Keychain queries will prompt or hang.
- **Rule:** In `team-agents`, credentials (API tokens, GitHub tokens, Tailscale auth keys) must **never** be looked up in the macOS Keychain. All credentials are read strictly from `config/secrets.env` (file mode `0600`) and the environment.

---

### 7.4 GUI Session Requirements for iOS Development

Certain macOS capabilities require an active, logged-in Aqua GUI window server session:
- **iOS Simulator (`test:ios-simulator`):** `xcrun simctl` can boot simulators headlessly, but UI test execution (`xcodebuild test`) and Simulator rendering require an active window server session.
- **Apple Code Signing (`sign:apple`):** Interactive keychain prompts fail without a GUI session.
- **Setup Guidance:** For dedicated Mac build nodes:
  1. Configure macOS auto-login for the agents user.
  2. Connect a dummy HDMI/DisplayPort plug (or virtual display software) to ensure the WindowServer is instantiated.
  3. Lock the screen or use fast user switching to keep the GUI session alive without physical access.

---

### 7.5 Laptop Power and Battery Awareness

For instances running on Mac laptops (MacBook Pro / Air), running intensive builds on battery drains power and degrades performance.

1. **Detection:** `agent node` checks battery status using `pmset -g batt` (macOS) or `/sys/class/power_supply` (Linux).
2. **Behavior (`PAUSE_ON_BATTERY=true` in `agents.conf`):**
   - When AC power is disconnected and battery drops below threshold (e.g. < 50%):
     - Stop pulling new board items.
     - Save a checkpoint on all running tasks (`checkpoint`).
     - Mark instance capacity as `busy` / `paused` in the heartbeat manifest so the hub routes work elsewhere.
   - When AC power is restored:
     - Automatically resume normal operation and manifest capacity.
