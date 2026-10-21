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

(Author: gemini-1 — to be merged from `docs/multi-instance-auth-manifest.md`.)

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
| `INSTANCE_CAPABILITIES` | array of typed capabilities | same | declared capabilities (section 5) |
| `INSTANCE_ACCEPT_REPOS` | array (optional) | same | local policy: repositories this instance accepts work for |

Token values never go into `agents.conf`; they are read from `secrets.env` (`HUB_TOKEN`, and on the lead one
`HUB_TOKEN_<NAME>` per allowed instance, name upper-cased with `-` replaced by `_`).

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

(Author: gemini-1 — manifest schema and vocabulary; coordinator reviews against decisions 10–12.)

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
| Instance manifest expires (no heartbeat for `MANIFEST_TTL`, default 5 min) | the instance is **gone**: its members become `unreachable`, the claimed board items return to the board, the coordinator receives a message naming the instance. |
| Instance returns | it re-registers; its members go back to their previous state (`active`/`sleeping`) and resume. |
| Command for a gone instance | typed error `InstanceUnreachable`; no queueing. |
| Hang watchdog | a hub outage is not a hang: while `serve` reports `Unavailable` or the node reports a hub outage, the watchdog does not count the run as hung. |
| Duplicate instance name | the hub rejects the second connection with `InstanceNameTaken`. |
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
5. `MANIFEST_TTL` default (5 minutes proposed) and `HUB_BACKUP_MINUTES` (30 proposed).
