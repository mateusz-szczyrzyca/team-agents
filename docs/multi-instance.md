# Independent installations and remote jobs

This document specifies the approved v1 design. **Round 1 adds the design, error
definitions and pending acceptance tests only. Round 2 implements the internal
in-memory registry, capability parsing, idempotent submission and atomic routing.
No persistent registry, remote job commands, listener, daemon or web views are
available yet.** Later rounds implement the paths below.

## Boundaries

Every Linux machine remains an ordinary installation with its own tasks, teams,
bus, watchdog, notifications and web interface. With no lead configured, nothing
new runs. Agents communicate with their local bus exactly as before. Only the
node daemon and human CLI communicate with the lead; a lead outage cannot affect
an already running agent.

One installation is the lead: it owns a node registry and a job queue. Nodes pull
jobs in periodic heartbeats. The lead can run its own node over loopback. Results
travel through the existing shared git remote; the protocol transfers task text,
job state and branch/result references, never files or executable instructions.

No shared bus, forwarded MCP calls, qualified team-member identities, generic
replay ledger, token/TLS mode, capability detection or macOS support is introduced.

## Data model

All statuses, capability namespaces and operation failures use Go typed values.
Times use `Timestamp`; zero means an event has not happened. New job state lives
in a separate `jobs.json` in the bus directory, atomically replaced under the
existing `bus.lock`. Ordinary `state.json` and `messages.jsonl` retain their
formats. Missing job storage means an empty registry. Finished jobs are retained
for 30 days. Corrupt or unreadable storage fails with the operation's storage
cause; it is never silently reset or overwritten with an empty registry.
Before any persisted type evolves, preserve its previous-format
fixture and test that it still loads.

### Job

| Field | Meaning |
|---|---|
| ID | Submitter-chosen idempotency key; resubmission identifies the same job |
| Repository | Logical name resolved through the receiving node's `NODE_REPOS` |
| Base ref | Git ref name, validated before any git invocation |
| Base SHA | Exact commit required for delegated work; a ref alone is insufficient |
| Requires | All capabilities needed to take the job |
| Agent selector | One locally configured agent name, or tier `normal` / `worker` |
| Target node | Optional instance pin requested by `--on` |
| Task | Text, at most 64 KiB |
| Requester | `human` or `<agent>@<instance>` for result delivery |
| State | `queued`, `assigned`, `running`, `waiting`, `done`, `failed`, `stopped` |
| Assignment | Authenticated node and the identity of its assignment transition |
| Local task | Task ID chosen and persisted by the node |
| Outcome | Chosen agent, branch, optional PR URL and head of final result text |
| History | Timestamped transitions, including repeated waits and resumptions |
| Start evidence | First accepted start transition; never cleared by later reports |
| Cancellation | Pending stop request for a job that crossed the start boundary |

An ID never creates two jobs. An identical submit returns the existing job; a
different submission under an existing ID cannot overwrite it. Assignment
identity changes on requeue/reassignment, including reassignment to the same node,
so a delayed report from an older assignment cannot authorize a launch.

### Internal core available after round 2

The pure registry validates submissions before changing any state. Logical IDs,
repository names, agent selectors and node pins use at most 128 ASCII characters:
an initial letter or digit, followed by letters, digits, dots, underscores or
hyphens. Requesters are `human` or two such names separated by `@`. These names
are data; authenticating and binding a remote requester to its node arrives with
the HTTP boundary in round 4.

Base refs follow Git's one-level ref-name rules, with a leading dash also
rejected. An optional SHA is exactly 40 or 64 hexadecimal digits; delegated
requesters must supply it. This is syntax validation, not proof that the commit
exists. The later node checkout verifies the repository and commit. Task text
is nonblank and limited to 65,536 bytes.

Capability requirements are a sorted, deduplicated set and hexadecimal SHAs are
normalized to lowercase before comparing retry identity. Identical retries
preserve the current state, submission sequence and history. Jobs with equal
submission timestamps are considered in insertion order. A mutex protects the
whole in-memory operation; callers receive independent copies of mutable slices.
This mutex does not replace the cross-process flock planned for round 3.

A heartbeat replaces the node's declaration only after successful validation.
It returns an existing unstarted assignment before looking for another job;
otherwise it assigns the oldest matching queued job. Assignment identities are
nonzero and never reused during the lifetime of this in-memory registry.
Queued and assigned transition times are retained, and retries add no transition.
The pure ownership guard checks job existence, assigned node and assignment
identity; it does not yet implement state reports or authorize a remote caller.

Waiting diagnostics consider the latest accepted declarations of live nodes,
including the node pin, all required capabilities on one node and matching free
agents. A node is offline at an age greater than or equal to its TTL. A pending
unstarted assignment blocks further admission on that node. Missing individual
capabilities are named first; when capabilities exist only on different nodes,
the diagnostic lists their whole canonical set. Assignment requeue, durable
sequence/identity recovery and started-job lifecycle handling remain round 3.

### Node

The authenticated Tailscale device's short name is its instance name. A heartbeat
updates its declared capabilities, free configured agent names and tiers, web
URL, last-seen time and job reports. The lead derives offline status from its
last heartbeat and `NODE_TTL_SEC`. Read views expose job and node fields only;
they never serialize internal bus state, configuration or credentials.
Routing uses the capabilities and free agents from the latest accepted heartbeat.

Requester names are routing data, not authentication. A remote requester belongs
to the authenticated submitting node. The lead never accepts a requester string
as authority to report another node's jobs or cancel a job.

## Capabilities and selection

`INSTANCE_CAPABILITIES` is a declared array, for example
`(os:linux arch:arm64 hw:rp2350)`. The closed namespace list is `os`, `arch`,
`build`, `test`, `hw`, `net`; values match `[a-z0-9._-]+`. Invalid values or unknown
namespaces stop node startup with a clear typed failure. There is no detection.

A heartbeat can assign at most one job. Select the oldest queued job that matches
the node pin, includes no requirement absent from the node, and has a matching
free configured agent. The node selects the actual agent, using existing provider
rotation if available, otherwise configured order. Capacity selection and task
admission must share the local protection used for ordinary task starts.

Assignment is atomic under the lead lock. Two concurrent heartbeats cannot take
the same job. An existing unstarted assignment can be returned again to its node
after a lost response; returning it does not assign a second job. The node's
durable job-to-task mapping prevents duplicate preparation or launch on retries.

A queued job remains queued when no live node can take it. `agent jobs` explains
the obstacle, such as `no node with hw:rp2350` or `no free normal agent`.

## Lifecycle and recovery

| Current state | Allowed next state / action |
|---|---|
| queued | assigned by heartbeat, or stopped by local cancellation |
| assigned | running after prepared checkout, failed on provisioning, stopped on cancellation, queued on offline expiry |
| running | waiting, done, failed or stopped |
| waiting | running after answers, done, failed or stopped |
| done / failed / stopped | Terminal; identical reports are harmless retries |

### Acknowledged start

1. Persist assignment and local task mapping. Resolve repository and agent through
   local configuration; validate ref and required SHA.
2. Prepare the checkout and task metadata through the same functions used by
   `agent run`. Fetch, checkout or worktree failures fail the job with a typed
   reason. No job is recorded as started before its checkout exists.
3. Send a `running` report carrying the assignment identity, prepared task ID,
   branch and timestamp. Launch only after the lead accepts it. Retry a lost
   acknowledgement with the same report; never infer success from a timeout.
4. Once the lead accepts the start, it never requeues that job. There may be an
   interval with `running` recorded before the process launches; a crash in this
   interval leaves recovery on the same node, never on another machine.

An offline node's unstarted assignments return to the queue. Started jobs retain
their real state with a `node offline` indication until that node reports again.
An expired assignment cannot start: its start report is rejected even if the
same node later receives a new assignment for the same job.
Prepared checkouts that never cross the start boundary remain owned by the
preparing node for cleanup or safe reuse; they cannot authorize a launch.

### Reports and outages

The node durably retains the latest unreported snapshot for every job, including
the cumulative timestamped transition history and assignment/start evidence.
The next successful heartbeat sends it. A snapshot can cover intermediate states
missed during an outage; validate the history, including `running` ↔ `waiting`,
rather than imposing a total order on states. This preserves every transition
time while avoiding a separate queue of network calls.
Lead-owned transitions (assignment, offline requeue and immediate cancellation)
use at least the preceding history time if the lead clock moves backward, so
a successful write remains readable after restart.

The assigned node alone may report. Identical timestamp-and-payload retries are
idempotent. Stale reports cannot regress state, and conflicting reports cannot
replace an accepted terminal outcome. Reports from an earlier assignment cannot
alter the current assignment. The node clears pending state only after a
successful acknowledgement; restart recovery uses its durable task mapping and
local task state, not a new task or a second launch.

### Cancellation

Only the human CLI on the lead can cancel. Queued jobs and assignments before
the accepted start boundary become `stopped` immediately; subsequent start
reports are rejected. For started jobs the lead persists a stop request and
returns it in heartbeats until acknowledged. The node stops its own local task
and reports `stopped`; until then the job shows cancellation pending. An offline
node is never falsely described as stopped. If completion wins the race, retain
the actual terminal result. Repeating cancellation has no additional effect.

Questions stay on the node: its notifications, `agent answer` and web interface.
The lead shows `waiting` with a link to that node's `AGENT_WEB_URL`.

## Lead storage

The lead keeps its jobs in `jobs.json` in the bus directory, separate from
`state.json`; job operations never read or write `state.json`.

- Format: version 1, written to a temporary file and renamed into place.
- Lock: the same `bus.lock` flock as `state.json`, through the shared
  `Bus.withLock` helper, which `locked` also uses.
- Time: stored as UTC RFC3339Nano. The snapshot uses its own structs, so the
  ordinary `Timestamp` JSON format does not change.
- Missing file: an empty registry. Corrupt, unsupported or inconsistent file: a
  typed storage failure; the file is never overwritten with an empty registry.
- Retention: a terminal job is removed once its age reaches 30 days, before the
  next successful registry operation. Active jobs are always kept.
- Counters: the sequence and assignment high-water marks survive job removal,
  so identities are never reused.
- Internal only: no CLI or HTTP entry point uses the store yet (round 4 and later).

## HTTP and authorization

Plain HTTP is bound to the configured lead Tailscale address (`LEAD_LISTEN`) and
loopback for its own node. Tailscale provides encryption; there are no tokens or
private TLS certificates. Authenticate the socket peer, never caller-supplied
forwarded headers. Ask local tailscaled with `tailscale whois --json <address>`
using the approach in `live/web_identity.go`. Permit only devices in `LEAD_NODES`
by name or `tag:…`; derive instance identity from the authenticated device.

Exact `127.0.0.1` and `::1` peers are the lead's own node. Do not treat all
`127.0.0.0/8` as that identity: tests use `127.0.0.2` with fake whois as another
device. Loopback assumes local OS users are trusted, as the local bus does.

| Operation | Caller and contract | Error type |
|---|---|---|
| `POST /v1/heartbeat` | Authorized node; update registry, apply its reports, return acknowledgement, stop requests and at most one assignment | `LeadHeartbeatError` |
| `POST /v1/jobs` | Lead CLI or authorized node; idempotent submit | `LeadSubmitError` |
| `POST /v1/jobs/{id}/state` | Assigned node only; validate assignment and transition history | `LeadReportError` |
| `POST /v1/jobs/{id}/cancel` | Local lead human only; immediate stop or durable stop request | `LeadCancelError` |
| `GET /v1/jobs` | Authorized caller; public job fields, node CLI displays its own jobs | `LeadListJobsError` |
| `GET /v1/jobs/{id}` | Authorized caller; public fields of one job | `LeadGetJobError` |
| `GET /v1/nodes` | Authorized caller; public registry fields | `LeadListNodesError` |

Requests and replies are JSON. Each operation has its own causes in
`agentbus/errors.json`; HTTP adapters map failures explicitly. Do not reconstruct
arbitrary Go error types from the wire. Heartbeat reports use the same registry
report implementation as the state endpoint. CLI adapters reuse the same
operations as the daemon; no second implementation of admission or mutation.

### Operation causes

Round 1 declares the following causes; each name is prefixed by its operation
type in Go. `Storage` is generated for each of the seven registry operations.
The definitions are inactive until their implementing rounds, which add tests
for every concrete failure and any additional causes needed by the adapters.

| Type prefix | Declared causes besides Storage |
|---|---|
| LeadHeartbeat | Unauthorized, MalformedBody, InvalidCapabilities, NoSuchJob, StaleAssignment, InvalidState, TerminalConflict |
| LeadSubmit | Unauthorized, MissingArgument, InvalidID, InvalidBase, InvalidCapabilities, TaskTooLarge, MalformedBody, JobConflict |
| LeadReport | Unauthorized, NoSuchJob, MalformedBody, InvalidState, StaleAssignment, TerminalConflict |
| LeadCancel | Unauthorized, NoSuchJob |
| LeadListJobs | Unauthorized, InvalidArgument |
| LeadGetJob | Unauthorized, NoSuchJob |
| LeadListNodes | Unauthorized |
| CapsParse (no storage) | Malformed, UnknownNamespace, InvalidValue |
| LeadWhois (no storage) | UntrustedNode, TailscaleFailed, MalformedOutput |
| NodeClient (no storage) | Unreachable, Rejected, ServerError |

Unauthorized, UntrustedNode and Rejected have class `Permission`; NoSuchJob is
`NotFound`; JobConflict, StaleAssignment and TerminalConflict are `Conflict`;
InvalidState is `InvalidState`. Whois process/output failures and node transport
or server failures are `Process`. Other validation causes are `InvalidArgument`.
Unauthorized includes an authenticated node reporting another node's job;
StaleAssignment is reserved for superseded assignment evidence from its owner.
HTTP adapters bound request bodies before decoding; oversized heartbeat or
report bodies map to MalformedBody. Submit task text has its own TaskTooLarge
validation independently of the HTTP body limit.
Cancellation has no terminal-conflict cause: cancelling a finished job is an
idempotent no-op that preserves its actual outcome.

Repository names resolve only through local `NODE_REPOS`, such as
`([team-agents]=/srv/src/team-agents)`. Network input cannot choose executables,
filesystem paths, shell options, configuration or environment variables. A base
must be a ref name, never an option; reuse the validation approach from
`agentbus round-size`. Task text remains text. Agents are selected only from
local configuration. Any failure leaves a typed job failure, never a partially
started task reported as successful.

External programs use package-variable boundaries. Test defaults fail loudly
unless a test installs a fake, including whois; no real agent CLI or tailscaled
is invoked by tests. All test homes are temporary, with loopback networking only.

## Planned user interface

These commands are planned, not shipped in round 1:

```text
agent run --need <capability>[,…] <agent|normal|worker> "task"
agent run --on <instance> <agent> "task"
agent jobs [<id>]
agent jobs --cancel <id>
agent nodes
agent node
```

Nodes poll every `NODE_POLL_SEC` (default 10); a node is offline after
`NODE_TTL_SEC` (default 90). Services are installed only on configured instances:
the node service requires `LEAD_URL`. Configuration templates and README setup
instructions arrive with the working service/command rounds.

Completion notifications contain node, branch, optional PR URL and result head.
The lead's Jobs view shows state, node, agent and outcome, linking to the node
for questions. Instances shows capabilities, free agents, last heartbeat and
offline status. A node's task identifies `job <id> from <lead>`.

`delegate` gains optional `requires`. If local capabilities are insufficient,
require the member's commit to be pushed, submit its exact SHA as a job and
deliver the ordinary helper-result message with state, node, branch and result.
This changes neither team member identity nor the local MCP connection.

## Approved implementation rounds

| Round | Scope | Counted line estimate |
|---|---|---:|
| 1 | Design, typed causes, pending acceptance roadmap, README status | 900 |
| 2 | Pure registry model, capabilities, submit, routing, atomic assignment | 1,900 |
| 3 | Persistence, reports, cancellation, offline handling, 30-day retention | 1,900 |
| 4 | HTTP, Tailscale identity, authorization, lead service | 2,100 |
| 5 | Shared task start and allow-listed checkout adapter | 2,000 |
| 6 | Node daemon, durable reports/mapping, recovery, service, two-home tests | 2,200 |
| 7 | CLI commands and lead notifications | 1,900 |
| 8 | Remote delegation and helper results | 1,900 |
| 9 | Web views, complete setup documentation, human manual checklist | 2,200 |

Each PR stays under 3,000 counted added/deleted lines (generated files excluded)
and aims for 2,000. Every behavior starts with executable RED/GREEN tests in its
owning round. Pending tests identify their future round; no test due in the
current round remains pending. Existing test behavior remains unchanged. The
explicitly approved lint baseline and resume-ID collision repair were reviewed separately in prerequisite PR #7,
which is merged before this round.
Unfinished entry points stay inactive and every round passes `make lint test`.

The original roadmap markers span layers: the round-2 other-node report marker
first exercises the pure assignment-ownership guard; round 3 adds complete report
validation and persistence. Capability parsing, including malformed values, is
tested in round 2; the round-4 capability marker checks boundary rejection, and
round 6 enforces daemon startup failure. Per-operation authorization gets
executable cause tests with the round-4 HTTP handlers. These markers are a
roadmap, not a complete inventory of the tests required in each round.

Registry and HTTP concurrency get race tests. Two temporary installations with
stub CLIs cover capability routing, result commits, lead outage/recovery,
checkout failure, unknown repositories and delegated results. The final manual
checklist covers real tailnet capability jobs, delegated work, node offline/back
and lead restart during a job. The human runs that checklist; automated tests
do not claim it was executed.
