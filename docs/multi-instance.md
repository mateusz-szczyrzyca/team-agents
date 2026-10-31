# Independent installations and remote jobs

This document describes the implemented Linux lead/node design, including remote
team members, task mirrors, and lead-side answers and kicks. The original
nine-round remote-job foundation and the six-round follow-up are complete with
this documentation round. See [the README](../README.md#several-instances) for
installation settings, command details, compatibility notes and event names.
The manual checklist below is for the human; automated checks do not establish
that a real tailnet deployment has passed it.

## Boundaries

Every Linux machine remains an ordinary installation with its own tasks, local
bus, watchdog and web interface. With no lead configured, no lead or node service
runs. One installation is the lead: it owns the node registry and job queue.
Nodes pull assignments and durable control requests in periodic heartbeats;
the lead can run its own node over loopback. Code changes travel through the
shared git remote. A bounded allow-list of task artifacts and structured log
records also travels to the lead for inspection; configuration, credentials and
arbitrary files do not.

Ordinary jobs use the node's local bus. Remote team members use an authenticated
MCP relay to the team's bus on the lead and have qualified `<agent>@<node>`
identities. The coordinator stays local to the lead. A lead outage does not stop
an ordinary running job; a remote member cannot use the shared bus while the
lead is unavailable and sleeps after a relay error. Calls are not queued for
replay. Nodes still initiate all connections, including the SSE doorbell.
There is no token/TLS mode, automatic capability detection or macOS support.

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
| Links | Optional validated GitHub repository URL and requesting task ID; display data only |

An ID never creates two jobs. An identical submit returns the existing job; a
different submission under an existing ID cannot overwrite it. Assignment
identity changes on requeue/reassignment, including reassignment to the same node,
so a delayed report from an older assignment cannot authorize a launch.

### Registry validation

The pure registry validates submissions before changing any state. Logical IDs,
repository names, agent selectors and node pins use at most 128 ASCII characters:
an initial letter or digit, followed by letters, digits, dots, underscores or
hyphens. Requesters are `human` or two such names separated by `@`. These names
are data; the HTTP boundary authenticates and binds a remote
requester to its node.

Base refs follow Git's one-level ref-name rules, with a leading dash also
rejected. An optional SHA is exactly 40 or 64 hexadecimal digits; delegated
requesters must supply it. This is syntax validation, not proof that the commit
exists. The node checkout verifies the repository and commit. Task text
is nonblank and limited to 65,536 bytes.

Capability requirements are a sorted, deduplicated set and hexadecimal SHAs are
normalized to lowercase before comparing retry identity. Identical retries
preserve the current state, submission sequence and history. Jobs with equal
submission timestamps are considered in insertion order. A mutex protects the
whole in-memory operation; callers receive independent copies of mutable slices.
This mutex does not replace the cross-process flock used by durable operations.

A heartbeat replaces the node's declaration only after successful validation.
It returns an existing unstarted assignment before looking for another job;
otherwise it assigns the oldest matching queued job. Assignment identities are
nonzero and never reused during the lifetime of this in-memory registry.
Queued and assigned transition times are retained, and retries add no transition.
The pure ownership guard checks job existence, assigned node and assignment
identity; state reports reuse the lifecycle validator, and
remote authorization belongs to the HTTP boundary.

Waiting diagnostics consider the latest accepted declarations of live nodes,
including the node pin, all required capabilities on one node and matching free
agents. A node is offline at an age greater than or equal to its TTL. A pending
unstarted assignment blocks further admission on that node. Missing individual
capabilities are named first; when capabilities exist only on different nodes,
the diagnostic lists their whole canonical set. Assignment requeue, durable
sequence/identity recovery and started-job lifecycle handling preserve these rules
across restarts.

### Node

The authenticated Tailscale device's short name is its instance name. A heartbeat
updates its declared capabilities, free configured agent names and tiers, web
URL, last-seen time and job reports. Its optional configured-agent inventory
contains names, CLI types, models and tiers; only `FreeAgents` affects routing.
Legacy heartbeats without inventory remain valid. The lead retains the actual
chosen agent in job reports independently of the requested selector. The lead
derives offline status from its last heartbeat and `NODE_TTL_SEC`. Read views expose job and node fields only;
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
A single report must carry its transition time, except for the first start
(`assigned` → `running`) and a terminal state: a report without a time that
changes a non-terminal state cannot be told from a replay of an older report and
is refused, so a replay cannot rewind a job or overwrite its task ID.
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

Questions are mirrored to the lead. The human can answer or kick an authorized
remote task from the lead's CLI, terminal or web interface, without opening the
node's web interface. The durable delivery rules are described below. Job
cancellation and a task kick remain separate actions: cancellation ends the job;
a kick wakes a team member or stops a solo task using its expected run number.

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
- Used by the lead's HTTP listener (`agentbus lead`). The node daemon reaches it
  over HTTP, including remote delegation.
- Additive optional pending-answer and pending-kick records retain requests
  through outages. Legacy files without these records still load. Node-side
  receipt and acknowledgement fields in `node-jobs.json` are optional too.

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

A successful heartbeat atomically acknowledges its entire report batch and
returns at most one assignment (`Job` is `null` when none matches). The
caller’s liveness refresh precedes offline expiry, so its delayed start reports
retain their assignment; a rejected heartbeat rolls back both liveness and
reports. Other silent nodes expire normally. Nodes truncate each report result
to 64 KiB and split report batches to keep the encoded request within 256 KiB.
Replies also carry durable stop, answer, kick and remote-member wake delivery.
JSON field names use PascalCase and timestamps use UTC RFC3339Nano. An error
reply contains `Operation`, `Cause` and `Message`; clients never reconstruct Go
errors from it. Malformed and oversized bodies map to each operation's own cause.

Additional endpoints support authenticated artifact/log sync (`POST /v1/sync`),
remote team MCP calls (`POST /v1/mcp`) and the node-initiated SSE doorbell
(`GET /v1/events`). Sync validates assignment ownership and bounded content;
the MCP relay runs with the lead's granted member identity and restricted tool
set. The doorbell only signals that heartbeat work may be available: it carries
no task state or commands and needs no replay history. Polling remains the
fallback when the stream disconnects.

Browser requests carrying `Origin` or `Sec-Fetch-Site` are refused, including
requests to loopback. Forwarded headers never grant local authority. Listeners
have bounded header, read, write and idle timeouts; shutdown drains requests for
at most 5 seconds and then closes connections. HTTP diagnostics use structured
logging. `agent lead` starts the foreground service; `agentbus lead` is its thin
command adapter, and `install.sh` installs `agentlead.service` only when
`LEAD_LISTEN` is configured.

### Operation failures

Every operation has generated typed causes in `agentbus/errors.json`, including
sync, relay and local lead controls. Storage errors retain their operation's
storage cause. Authorization failures have class `Permission`; missing jobs are
`NotFound`; conflicting submissions, stale assignments and terminal conflicts
are `Conflict`. Validation failures use `InvalidArgument`, and process/transport
failures use `Process`. A sync HTTP `400` maps to `NodeClientInvalidArgument`
(exit `2`); an authorization rejection maps to `NodeClientRejected` (exit `5`).
Cancellation of a finished job is an idempotent no-op that preserves its outcome.
A corrupt storage file is an error, never an empty view or permission to reset.

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

## User interface

The public commands include:

```text
agent run --need <capability>[,…] <agent|normal|worker> "task"
agent run --on <instance> <agent> "task"
agent jobs [<id>]
agent jobs --cancel <id>
agent nodes
agent node [--once]
agent answer <task-id>@<node>
agent kick <task-id>@<node>
```

Nodes poll every `NODE_POLL_SEC` (default 5 seconds); a node is offline after
`NODE_TTL_SEC` (default 90 seconds). Services are installed only on configured instances:
the node service requires `LEAD_URL`, and the lead service requires `LEAD_LISTEN`.
The README and `config/agents.conf` document the configuration.

Completion notifications contain node, branch, optional PR URL and result head.
The lead's Jobs view shows state, node, agent and outcome, with links to the
instance, agent, available task mirror and remote member's local proxy. Instances
shows capabilities, configured and free agents, last heartbeat and offline status.
Agents combines local and remote inventory. A node's task identifies
`job <id> from <lead>`. External node, branch and PR links leave htmx navigation.

`delegate` accepts optional `requires` as a string array. Satisfied requirements stay
with the ordinary local worker dispatcher. Otherwise the local node daemon prepares
a worker job using the requester SHA captured at the delegate call. It fetches the
requester's origin branch into a unique private ref and checks that this SHA is an
ancestor; a missing push fails the helper before submission with an actionable
`NotPushed` cause. Git and HTTP never run while holding the local bus lock.

The node saves the immutable submission in the helper's optional `remote_job`
field before POSTing it. The ID is derived from the requesting instance, local
helper ID, creation time and requester task. A lost submission response or lead
outage retries this same request without rechecking a moving branch or changing
the SHA. There is no additional timeout. Without `LEAD_URL`, an unsatisfied helper
fails locally. Only the node daemon contacts the lead for delegation.

The daemon reads the job outcome and delivers the ordinary `helper_result` notice
to the requesting local agent, with state, node, branch, optional PR and result
head. `stopped` remains visible in the message and maps to a failed helper status.
The local notice identifies an already delivered result if saving helper state
fails, so a retry retains that outcome and does not post it again. Existing bus
state files without `requires`, `base_sha` and `remote_job` still load unchanged;
`testdata/state-before-remote-helpers.json` preserves the previous helper format.
This delegation path uses the ordinary local MCP connection; remote team
membership is a separate, explicitly configured relay described below.

## Task mirrors and lead controls

The lead stores node task mirrors at `<bus>/remote/<node>/<task-id>/`. Metadata
is a validated allow-list, not a copy of `meta.env`; artifact names and content
hashes are checked before replacement. Allowed artifacts are activity, rendered
prompts, question rounds, final/result text and error tails. Each artifact is at
most 256 KiB; a sync request is at most 2 MiB. Activity retains 2,000 lines and
error tails 500. Structured logs accept at most 500 records per request, 64 KiB
per record and retain 20,000 records per node. Cursor/digest receipts support
retry. Terminal tasks still sync changed artifacts; last-known data remains
available while a node is offline. Mirrors expire after 30 days.

Web task and activity routes use `<task-id>@<node>`, including
`/task/<task-id>@<node>` and `/events/task/<task-id>@<node>`. `agent live` also
shows mirrored tasks and their activity, question and result files. The Logs
view has a node selector. Mirrors identify their node and offline state; an
unavailable investigation/notebook tab says so explicitly. A local remote-member
proxy appears once in lists and counters, inherits the mirror's offline and
waiting state, and resolves questions/activity through that mirror. Direct mirror
links remain available. Offline does not change a started job's reported state.

A mirrored question file is read-only. The editable lead copy lives under
`<AGENTS_HOME>/remote-questions/<node>/<task-id>/questions-N.md`. The lead owns
question notification/editing; the node suppresses its local question editor and
notification for these lead-managed jobs. `agent answer` validates completed
verdicts and confirms sending; save-watch delivery requires the answer marker.
The web editor displays the current round in its form. Missing, conflicting or
stale round values return HTTP `409` before changing a draft or queue; reload the
page to answer the current round. A proxy's question tab reads the same mirror.

The lead queues answers durably by job, task, question filename and content digest.
Content is UTF-8, at most 256 KiB, with tabs and newlines allowed. The node applies
only the current waiting question file, then acknowledges that exact scope and
digest. Retries cannot answer a later round. Pending answers and kicks survive
lead/node restarts and heartbeat outages; an acknowledgement removes only the
matching request. Heartbeat replies are bounded to 1 MiB, with at most 64 answer
and 64 kick acknowledgements per request.

A kick carries the expected run through execution and metadata updates. An old
kick cannot stop a newer run. A failed stop is reported and retried without a
false acknowledgement or a false stopped/sleeping status. Successful team-member
kicks enter the existing team wake flow; solo-task kicks stop that task. The
local `agentbus lead-answer` (content on stdin) and `lead-kick` adapters require
node/task identity and use the configured lead storage and TTL; these are local
CLI operations, not unauthenticated HTTP control endpoints.

Activity views follow bounded mirrored tails. Overlap handling avoids replaying
already displayed rows, while a single identical record at a truncated boundary
can remain ambiguous. Terminal mirrors accept late activity updates. Equal-time
logs preserve descending source order. Displayed runtime uses the actual run
start and terminal finish times rather than freezing at a transient wait.

## Remote team members

A node's agent can be a member of a team on the lead, named `<agent>@<node>` — a deliberate reversal of "no
`agent@instance` names inside teams" in the original decisions, decided by the human. It keeps the rest of the
design: the team lives only on the lead's bus; the node relays the member's MCP calls (`POST /v1/mcp`, one
generic relay, the lead running them with `Server.handle` — no per-tool reimplementation); the identity comes from
the lead's grant for that node, never from the request; nodes still pull (a wake rides on the heartbeat reply);
an unreachable lead is a tool error and the member sleeps (no queued calls, no outage accounting). The coordinator
is always local. See README "Several instances".

A remote member's changes reach the team's logic in the agent script on the lead (`agent _remote_member` keeps its
proxy task's status and runs `team_after_run`), so it is put out when it fails and counts in "all asleep" and "all
finished". The doorbell (`GET /v1/events`, node-initiated, server-sent events) only rings: what waits is fetched by
the ordinary heartbeat, so nothing has to be replayed after a broken stream. A lead's shutdown cancels the
streams (its requests' base context), or draining would wait for them.

## Delivery and verification history

The original nine rounds delivered registry, storage, authenticated HTTP,
checkout/start, node recovery, CLI/notifications, delegation and Jobs/Instances
views. The follow-up delivered dashboard/agent links, artifact and log mirroring,
remote views, durable lead controls and frontend controls. Round 6 of that
follow-up reconciles this document and the human checklist; it changes no runtime
behavior or persisted format.

Implementation PRs are split into rounds with a 3,000 counted-line limit.
The completed implementation has executable tests rather than pending roadmap
placeholders. Registry/HTTP concurrency is exercised with race checks; two
temporary homes and stub CLIs cover routing, lead outages, recovery,
checkout failures, controls and result delivery. `make gate` runs lint and unit
checks without real agent CLIs or tailnet services. Real installation acceptance
remains the human checklist below; record its results separately.

### Manual checklist (human, real tailnet)

**Not run by the agent team.** These steps need a real lead and Linux node on
Tailscale, configured as in the README's "Several instances" section. Use a
disposable repository and tasks: task prompts can spend real agent quota, and
service interruptions affect jobs on those installations. Record date, versions,
node names, job/task IDs, observed result and any defect for each step. Run the
service interruption steps only on installations the human has selected for the
exercise. `make gate` uses isolated homes and stubs and does not run this list.

1. **Routing and completion.** Declare `test:remote` only on the destination node.
   From the lead's test repository, run
   `agent run --need test:remote normal "Make a small documented change, commit and push it"`.
   Inspect `agent jobs <job-id>`, `/jobs` and `/instances`: the job reaches that
   node, starts once, then reports its branch and outcome. The requested selector
   remains visible; the remote agent page attributes work to the chosen agent.
   Open node, repository and PR links when available: they navigate outside htmx
   swaps. A fast transition need not remain visible between page refreshes.
2. **Delegation.** On a requester lacking `test:remote`, configure `LEAD_URL` and
   `INSTANCE_NAME` and run its node daemon. Ask a normal task to commit and push
   its branch, then call `delegate` with `requires=["test:remote"]`. Check that
   the worker runs on a capable node at the captured commit and the requester
   gets one `helper_result` with node, branch, optional PR and result head.
3. **Mirrors and logs.** While a remote job produces activity, open its mirror
   link `/task/<task-id>@<node>` and `agent live`. Inspect activity, prompts,
   questions and final/error text as available. Verify node identity, elapsed
   time, bounded history and late activity after completion. In Logs, select the
   node and check new records arrive; timestamp ties retain descending source
   order. The mirror lives under `<bus>/remote/<node>/<task-id>/`, not the local
   task directory. Unavailable investigation/notebook tabs explain their absence.
4. **CLI answers.** Ask a test task to yield a question round. On the lead,
   check the notification, Questions list and waiting counter, then run
   `agent answer <task-id>@<node>`. Complete all verdicts, save and confirm sending.
   The node resumes once with those answers. The editable lead copy is under
   `<AGENTS_HOME>/remote-questions/`; the mirror remains read-only. Check that the
   node does not also launch its local question editor/notification.
5. **Web answers and stale forms.** Keep a round-1 answer form open, answer that
   round through another tab or CLI, and let the task ask round 2. Submit the old
   form: expect HTTP `409`, no new queued answer and no round-2 draft/file change.
   Reload and answer round 2: expect normal delivery and resumption. Repeat
   through a local remote-member proxy if the test team has one; its Questions
   tab must show the mirrored file and the waiting task must count once.
6. **Kicks and cancellation.** On disposable solo and team tasks, use
   `agent kick <task-id>@<node>` or the corresponding terminal/web action.
   A solo task stops; a team member enters the existing sleep/wake flow. To end
   a job, use `agent jobs --cancel <job-id>` on the lead. Cancellation remains
   pending while its node is offline and never invents a stopped outcome. During
   a controlled outage, queue a kick for a run, let the node task advance to a
   newer run, then restore delivery: the old kick must not stop the newer run.
   A deliberately induced stop failure must retain delivery for retry without
   acknowledging success; perform this only in the disposable test environment.
7. **Node offline and return.** While an ordinary job runs, stop
   `agentnode.service` on the selected node for longer than `NODE_TTL_SEC`
   (default `90` seconds). Check offline warnings in Instances, remote agents,
   mirrors and any retained proxy; the job keeps its last state and is never
   restarted elsewhere. New jobs stay queued without an eligible online node.
   If an assignment was already made but has no accepted start, expiry requeues
   it. Start the daemon again: pending reports/artifacts arrive, the node becomes
   online and there is no duplicate task launch.
8. **Lead outage and durable controls.** During an ordinary remote job, stop the
   selected lead service. The node task continues; heartbeats warn and retry.
   Restart the lead and check the saved assignment, transition history and final
   result converge without a second job. Repeat with an answer or kick already
   queued before the outage: only the intended file/run receives it, and retries
   clear the matching request after acknowledgement.
9. **Remote team and doorbell.** Start a test team with a qualified remote member
   as in the README. Check shared board/messages and local coordinator identity,
   then its proxy/mirror deduplication and offline flag. With the event stream
   connected, check wake delivery; interrupt the stream and verify ordinary
   `NODE_POLL_SEC` polling (default `5` seconds) still delivers work. During a
   lead outage, a remote member's relay call fails and the member sleeps; calls
   must not replay later. Restore connectivity and exercise an ordinary wake.
10. **Access boundaries.** From a tailnet device outside `LEAD_NODES`, query
    `http://<lead-tailscale-address>:7780/v1/jobs` (use the configured port): expect
    rejection. On an authorized test node, use a disposable malformed sync
    request to confirm it fails without changing mirrors. Check that mirrored
    metadata/artifacts/logs contain no configuration or credential files. These
    observations supplement the automated ownership, payload-limit and origin
    rejection tests; they do not replace them.
