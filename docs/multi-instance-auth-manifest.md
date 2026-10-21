# Multi-Instance Contributions: Authentication, Manifest, Capabilities, and macOS Port

This document contains the sections prepared by **gemini-1** for `docs/multi-instance.md`:
- Section 3: Authentication modes
- Section 5: Manifest schema and capability vocabulary
- Section 7: macOS port

---

## Section 3. Authentication modes

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
   LEAD_INSTANCE="https://hub.example.com:7777"
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

## Section 5. Manifest schema and capability vocabulary

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

---

## Section 7. macOS port

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
