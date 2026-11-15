# Round 8.6 investigation — inv-astra

- Investigator: `inv-astra`.
- Branch: `agents/inv-astra/20261115-080930`.
- Examined base: [`9a6057c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9a6057cedb3d68b6265f6482364bc1a9e703a453).
- Deliverable: independent investigation, observed baseline behavior, and implementation/test recommendations. No production code or existing tests were changed.
- Scope: item `8.6` only. The query CLI of `8.7` and later checkpoint work are excluded.

## Result

The existing history writer is a useful foundation, but attaching a tracing layer to it is insufficient. The important work is preserving context across asynchronous boundaries, redacting both destinations, preventing the history writer from logging its own failures back into itself, and covering every decision and state transition.

The probes reproduced a configured secret in stderr, demonstrated that malformed CLI output is absent from the history, and confirmed that `[log]` is not accepted. The existing writer already lets work finish when the database cannot open. Preserve that property.

There is also a concrete implementation constraint: an existing test pins the complete SQLite table list. Adding dedicated log tables requires human authorization to change that test. This report does not change it, request an exemption on the human's behalf, or disguise logs as another kind of record to evade it.

This completes the investigation, not the implementation of item `8.6`.

## Acceptance

| ID | Assessment at the examined base | Evidence |
|---|---|---|
| `A1` | **not met** | A real daemon with a fake Codex CLI completed its task after printing malformed JSON. Its stderr contained a `warn` with task/run IDs, but the database contained only existing history records. A separate daemon startup with `[log] level = "debug"` exited `1`: `unknown field log`. Consequently the requested debug assignment row cannot be obtained through that configuration. |
| `A2` | **not met** | There is no daemon-log event enum in the core. [`docs/log.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/docs/log.md) explicitly defers the daemon log to `8.6`; it has no corresponding event catalog or enum-comparison test. |

These are implementation acceptance assessments, not claims that an attempted implementation failed its tests.

## Checks

- `cargo build -p agent --locked`: passed.
- Isolated daemon probes with fake CLI and bus executables: `4` scenarios completed; results below.
- The probes were rerun through `tools/test-lock.sh`, sequentially. They used temporary homes, temporary repositories, local bare remotes, invented Git identity, and the worktree's prompt catalog. No real model CLI or GitHub API was called.
- `make gate` is to run on the committed report; its actual outcome is reported in the final handoff. The build and probes do not stand in for that gate.
- No old Go/Bats suite was run.
- No hardware device or running installation was used.
- No additional dependency or tool installation was needed.

The exploratory probes use the production clock and generated IDs. They establish baseline observations; they are **not** the deterministic, injected-clock acceptance tests required for the implementation.

### Observations

| Probe | Stimulus | Observed result |
|---|---|---|
| `P1` | Fake Codex prints `not-json`, then valid session, final-message, and completion events. | The task reaches `done`; `run_ends` says `reported` / `DONE`. Exactly `1` malformed-output warning reaches stderr. It has `task` and `run`, but no stable event field, team, or round. There is no daemon-log record in the database. |
| `P2` | Create a directory where the isolated home's `history.db` should be; run the same fake task. | The task still reaches `done`. Status reports history `unavailable` and `12` lost records at the observed snapshot. Exactly `1` history-unavailable warning is printed, alongside the separate malformed-output warning. |
| `P3` | Add `[log]` with `level = "debug"` to an otherwise usable configuration. | Daemon exits `1` with a TOML unknown-field error. |
| `P4` | Configure `PROBE_SECRET` in the secret environment, then have the fake CLI print its value as the malformed line. | The synthetic marker `INV_ASTRA_MARKER_86_EXAMPLE` appears verbatim in daemon stderr. It is absent from the closed database's bytes. This probe did not inspect raw CLI output files or a live WAL. |

`P1`, `P2`, and `P4` also sent an invalid JSON request before starting the task. The daemon returned an `invalid` response and continued serving requests. No warning describing that refusal appeared in the captured stderr.

Normalized relevant stderr from `P1`:

```text
WARN agent_runtime::run: lines of the output of a run could not be read task=TASK run=RUN count=1 first=cannot read a line of CLI output: "not-json": not JSON: expected ident at line 1 column 2
INFO agent::daemon::owner: run ended task=TASK end=Reported { outcome: Done }
```

The history ledger contained these kinds after `P1`: `changes`, `launch`, `notified`, `round`, `round_closed`, `run`, `run_end`, `session`, `step`, `team`, and `team_ended`. There were `2` step records; every other listed kind had `1` record. The CLI's readable activity was recorded; its parser warning was not.

### Reproduction inputs

Build the binary, use an isolated `AGENTS_HOME` and `HOME`, create a temporary Git repository named `example` with a local bare `origin`, and supply the repository prompt directory to `agent daemon --prompts-dir`. Configure one normal Codex agent, two outside investigators, the fake executable's absolute path under `binaries.codex`, and `repos.example.base_branch = "main"`. No GitHub token is needed for this solo probe.

The fake CLI's complete output for `P1` is:

```jsonl
not-json
{"type":"thread.started","thread_id":"example-session"}
{"type":"item.completed","item":{"type":"agent_message","text":"STATUS: DONE"}}
{"type":"turn.completed"}
```

Start `agent run codex-1 "Perform the probe."` in that temporary checkout. Read the members' actual `status` values through the socket until the task is `done`; the unrelated item-count key named `done` is not evidence of completion. Stop only this scratch daemon with `SIGTERM`, wait for it, and query its closed database:

```sql
SELECT kind, count(*) FROM history_records GROUP BY kind ORDER BY kind;
SELECT how, outcome FROM run_ends;
SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name;
```

For `P2`, create the `history.db` directory before starting the scratch daemon. For `P3`, append the log configuration before startup. For `P4`, write the following secret file with mode `0600`, and have the fake CLI print the value from its environment in place of `not-json`:

```toml
[agent_env]
PROBE_SECRET = "INV_ASTRA_MARKER_86_EXAMPLE"
```

## Findings

### F1 — Redact before either destination; the current stderr path leaks the marker

[`crates/agent-runtime/src/run.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/run.rs#L551) retains the first parser error as a formatted cause chain and prints it at the end of the run. That chain includes the rejected output line, as `P4` demonstrates. The default formatter in [`crates/agent/src/commands/daemon.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/commands/daemon.rs#L38) does not use the history redactor.

The implementation must redact the message, formatted causes, structured text fields, and text inherited from spans before both SQLite and stderr. Reuse the configured-secret matching rules in [`crates/agent-runtime/src/history/redact.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/history/redact.rs). Do not assume `%error`, `?result`, a panic payload, or an external-process error is secret-free. Do not dump an entire run request: [`crates/agent-runtime/src/run.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/run.rs#L103) contains prompts, CLI arguments, and MCP command lines.

Preserve useful cause classes and safe context. A blanket replacement of the whole error with an empty string would lose the chain required by the item and `ERR-7`. Redact before truncation and before calculating any persisted duplicate key; a raw secret must not survive in a key or auxiliary table. Test overlapping secrets, Unicode, multiple fields, span fields, panic payloads, and the database plus its WAL while the daemon is running.

### F2 — The writer's diagnostics must bypass the history layer

[`crates/agent-runtime/src/history/handle.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/history/handle.rs#L96) implements `TracingReporter` by emitting a warning. Queue overflow and storage failure reach that reporter. [`crates/agent-runtime/src/history/notes.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/history/notes.rs#L76) also emits diagnostics from inside the history writer. Writer shutdown failures are reported in [`crates/agent/src/daemon/history.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/history.rs#L84).

If a layer feeds those events back into the same history queue, it attempts to persist its own loss diagnostics. Existing once-per-cause suppression reduces repetition but is not the required exclusion. This is a structural feedback risk; the current program has no such layer, so the probes do not demonstrate an existing infinite loop.

Use a separate, redacting stderr reporter for sink failures, or an explicit dedicated target excluded before any queue operation. Audit all writer paths, not just `TracingReporter`. Do not exclude every history-related operational event: failure to capture a run's changes, for example, can be valuable when the database itself is healthy. Preserve lost-record accounting and keep both queue submission and panic reporting nonblocking. Writer-originated failure reports must not create additional lost log records.

### F3 — Two frozen contracts affect the storage design

- [`crates/agent-runtime/tests/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/tests/history/schema.rs#L31) compares the complete table list to `14` named tables. A new log table makes that existing assertion fail. Updating the schema documentation alone does not resolve it. The recommended dedicated-table design therefore needs a narrowly scoped, human-authorized test change before implementation.
- [`crates/agent-runtime/tests/history/ledger.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/tests/history/ledger.rs#L116) exhaustively matches all variants of `agent_core::history::Record`. Adding `Record::Log` makes that existing test fail to compile. Prefer a separate log record type and an additional private writer job, preserving the existing `Record` API; do not add a wildcard to the test.

The existing ledger is not an interchangeable log store: [`crates/agent-runtime/src/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/history/schema.rs#L265) requires a non-null team and has a closed kind list. Startup errors and invalid requests may have no team. Do not invent a team ID or put a global log message in an unrelated team's row.

Human authorization is a prerequisite for the proposed schema-test adjustment, not a prerequisite for recording these investigation findings. No prohibited test change was attempted.

### F4 — A run can belong to several rounds

[`crates/agent-core/src/decide/round/runs.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-core/src/decide/round/runs.rs#L43) explicitly gives coordinators every open round and workers the requester's rounds. [`crates/agent/src/daemon/owner/round_state.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/owner/round_state.rs#L26) already resolves these relationships. A scalar `round` chosen by taking the first element would silently misattribute events.

Use the existing run-round snapshot at launch. A log event needs either one specific round selected by a narrower operation span or the full set inherited from the run. Store one event with round links so it is found from each relevant round without double-counting. A plain team look carries its team; a global event carries no fabricated IDs. Do not recompute a completed run's rounds from whichever item is currently assigned to its task.

### F5 — Context must cross tasks, channels, and threads

The code inspected has no existing tracing spans or instrumented futures in the daemon/runtime source. Opening a span in the socket handler will not establish context when the owner later handles its channel message. The same applies to gate jobs, notification jobs, detached process cleanup, and spawned model runs.

Attach immutable context to asynchronous work at submission, then instrument the future that performs it. Carry request context through the owner message. Explicitly propagate context into blocking calls or writer-related threads where it is required. Use tracing's event scope, including an explicit event parent, and support span fields recorded after creation. Do not hold a span-entry guard or a mutex guard across an await.

The locally installed sources for locked `tracing 0.1.44` confirm separate `Instrument` and `WithSubscriber` facilities. A scoped test subscriber also needs to follow spawned futures; merely wrapping the outer test body does not do that. Test with two interleaved teams and ensure neither acquires the other's IDs.

Do not discard context spans at a debug threshold while keeping warnings: an error-only configuration still needs the run IDs. Event filtering and context collection are different operations.

### F6 — Logging lifetime must include startup, final failure, and shutdown

[`crates/agent/src/commands/daemon.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/commands/daemon.rs#L38) currently installs a global formatter with a hard-coded `INFO` maximum before configuration is loaded. [`crates/agent/src/daemon/mod.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/mod.rs#L194) opens history later, after several fallible setup steps. It closes the writer before the command emits its final stopped event.

Putting a layer only around `Owner::run` loses earlier startup events, background service events, and the final outcome. Establish the logging lifetime at the binary boundary, with a documented stderr-only bootstrap while settings/history are unavailable. Once settings are valid, attach the selected filter and history sink before normal daemon activity. Emit the terminal daemon event before draining and closing the writer; report drain failures only to stderr. Avoid a strong reference cycle between the subscriber, history handle, and reporter.

A panic hook is absent. Install it once at the executable boundary. Capture payload, file, line, and current context synchronously, redact them, enqueue without waiting on the panicking task, and allow normal unwind/termination handling to proceed. A later `JoinError` should not duplicate the panic as a second error event. Do not blindly chain a default hook that prints an unredacted payload after the redacted event.

The database cannot be promised durable logging after an abort, out-of-memory termination, or a full/broken queue. The required task-panic proof should use unwinding and show the surviving daemon flushing the event; stderr is the immediate fallback.

### F7 — Naming existing events does not cover the required behavior

A source inventory found `130` qualified tracing call sites in the daemon command, daemon source, and runtime source, excluding test files/directories and fixture helpers: `23` error, `62` warn, `30` info, `13` debug, and `2` trace. This is a textual inventory, not a count of distinct runtime behaviors.

Several named requirements are missing or only partially represented:

| Boundary | Evidence | Required change |
|---|---|---|
| Invalid request | [`crates/agent/src/daemon/server.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/server.rs#L237) and `P1` | Log the refusal once with its safe cause and request context; do not copy the raw request. |
| Run output | [`crates/agent-runtime/src/run.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/run.rs#L551) | Stable name and complete scope; retain counts without logging the same lines again at finalization. |
| Run outcome | [`crates/agent/src/daemon/owner.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/owner.rs#L716) | Keep the normal run-end transition, and separately name no-status, hang, quota, and rate-limit deviations at their handling boundary. |
| State save | [`crates/agent/src/daemon/owner.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/owner.rs#L1278) | Audit every path returning a store error; one handler logs the full chain before failure reaches the CLI. |
| Recovery | [`crates/agent/src/daemon/owner.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/owner.rs#L495) | Explain what restart corrected, with the affected IDs; the existing aggregate info message is insufficient. |
| Assignment | [`crates/agent/src/daemon/owner/teams.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/owner/teams.rs#L79) and [`crates/agent/src/daemon/owner/teams.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/owner/teams.rs#L422) | Cover both initial assignment during formation and later assignment; instrumenting only `assign_items` misses the former. |
| Bus retry | [`crates/agent/src/daemon/owner/teams.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/owner/teams.rs#L300) | Record what waits and why; coalesce repeated warnings without hiding a different job or cause. |
| GitHub retry | [`crates/agent-runtime/src/github.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/github.rs#L1387) | The existing debug attempt/delay message needs the safe cause and an appropriate warning; retain debug timing/outcome separately. |
| External calls | [`crates/agent-runtime/src/git.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/git.rs), [`crates/agent-runtime/src/agentbus.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/agentbus.rs), and [`crates/agent-runtime/src/github.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/github.rs) | Central call spans and duration/outcome events, including failure and cancellation. |
| Gate | [`crates/agent/src/daemon/gate.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/gate.rs) | Cover start failure, timeout, and dirty tree, not just the branch that returns an error. |

Log actual state differences only after the state update is accepted/persisted. An attempted write that fails is not a successful transition. Do not convert every published team summary into an info event: summaries may repeat unchanged state.

Keep decision logging at the daemon call site, leaving `agent-core::decide` pure. Use compact typed input/result summaries, and no event for a no-op. Outside-call failures get a debug outcome at the call boundary and one warning/error where handled, rather than a warning at every propagation layer (`ERR-10`).

### F8 — Coalescing is durable counting, not suppressing subsequent calls

The required `5` identical warnings must produce one readable row with count `5`. An in-memory set that suppresses the last `4` warnings fails that requirement. Delaying the first write until a warning stops also loses the initial failure when the daemon dies.

Recommended contract: write the first occurrence immediately; update count and last-observed time transactionally for repeats within the same unresolved incident. Identity includes event, level, complete scope, operation, and stable cause details. Exclude elapsed time, retry number, observation time, and the accumulated count. A changed cause, task, run, or operation must not merge. Recovery closes the incident; recurrence starts another row. Document the restart policy explicitly, preferably starting a new incident while retaining prior counts.

Bound any aggregation cache. Redact before identity calculation, use deterministic field ordering, and check numeric overflow. Persist the count update without changing the event's original ordering position. A future follow API may need to observe count updates; implementing that API is `8.7`, not this round.

### F9 — The available wall clock has whole-second precision

[`crates/agent-runtime/src/clock.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/clock.rs#L9) deliberately rounds `SystemClock` to seconds. It is suitable for existing state IDs but would report most external calls as taking zero time if used by subtraction. Using raw `Instant::now()` instead would make duration assertions depend on real time.

Use the injected clock for event timestamps and a separately injectable monotonic timer for elapsed duration, or another explicitly designed deterministic timing abstraction. Preserve the current ID/time contracts. Test clock rollback and subsecond external calls; do not change the global ID clock's precision as an incidental logging fix.

## Recommended implementation shape

This is a proposed design, not code committed by this investigation.

1. Define `LogEvent` and `LogLevel` as keyword enums using the existing strum pattern in [`crates/agent-core/src/kinds.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-core/src/kinds.rs). Declare each text once. Associate every event with a fixed documented level. Add a validated log configuration with default `info`, and refuse `trace` or unknown values.
2. Define a separate validated log entry with timestamp, level, module, stable event, redacted message, bounded structured fields, context, and positive occurrence count. Model global/team/task/run contexts explicitly instead of permitting arbitrary contradictory ID combinations. Preserve structured booleans and numbers; format typed errors only at the reporting boundary.
3. Add a layer under the history runtime that collects span fields and produces the same sanitized event for stderr and history. Use a registry with span extensions and handle both span creation and later field updates. Make names explicit; compiler-generated tracing metadata names and free-form messages are not stable event identifiers.
4. Extend the existing bounded queue with a private log job and a log-capable store path. Preserve `History::record(Record)` and existing store implementations. If a new store capability is optional, unsupported writes must return a typed error, never silently succeed. SQLite remains owned by the same writer thread; the layer does not open another writer connection.
5. Subject to the authorization in `F3`, add a forward migration from schema version `3`, dedicated strict log storage, and round links. Add typed reading methods sufficient to verify log entries without inventing the later query CLI. Old histories remain readable; old data remains unchanged; newer schemas are refused without mutation. Startup/unknown-team diagnostics must remain representable even if recording the team itself failed.
6. Add explicit event names at every owned call site and the missing handling boundaries in `F7`. Define how third-party tracing targets are handled; do not silently discard an unnamed event from the harness. The event enum must not become a generic catch-all that hides missing instrumentation. The two current trace-only no-subscriber messages are normal empty-consumer outcomes; they do not justify an extra configurable level.
7. Wire initialization, selected event filtering, the panic hook, and orderly drain/close at the executable boundary. Keep context collection enabled even when its span's informational level would otherwise be filtered.
8. Document the catalog, configuration, schema, duplicate-count contract, failure behavior, and compatibility. Finish every behavior with its acceptance evidence, not merely a successful unit test of the visitor.

The expected file list needs small supporting changes outside its literal entries: exports in [`crates/agent-core/src/lib.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-core/src/lib.rs) and [`crates/agent-runtime/src/history.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/src/history.rs); queue/database/reader/migration code; [`docs/history.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/docs/history.md) and [`docs/history.schema.sql`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/docs/history.schema.sql) for the persisted format; and [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/README.md) for the user-visible configuration. Explain those in the integration description. The report does not propose changes to the old implementation.

## Tests to add in the implementation

All names below are **proposed**, not tests run or added here. Use new test modules and frozen RED commits. Existing fixtures are reusable; do not rewrite their behavior. Every event-catalog member must be observed from a real daemon scenario, not manufactured by looping over the enum and emitting it directly.

| Behavior from the item | Proposed proof |
|---|---|
| Every tracing event keeps time, level, module, name, message, fields, and context | `daemon_events_round_trip_with_injected_time`: fake run plus fake services, read complete typed rows back; pin normalized human output. |
| Malformed CLI output (`A1`) | `malformed_cli_line_is_a_scoped_warning`: reject one line, accept the next, finish the run; assert exact level/name/team/round/task/run/cause/count. |
| Debug assignment (`A1`) | `initial_and_later_assignments_explain_input_and_result`: a multi-item daemon team, assert initial and subsequent assignments, not only a direct core call. |
| Every enum name has a real producer | `every_log_event_is_produced_by_daemon_scenarios`: collect observed names from the scenario corpus, compare the set with enum iteration, and validate each event's expected level and scope. |
| Catalog matches enum (`A2`) | `documented_log_catalog_equals_the_enum`: parse a dedicated catalog table, compare exact names, levels, uniqueness, and nonempty descriptions. A mention in prose does not count. |
| Idle look writes nothing at info | `one_hundred_unchanged_looks_are_quiet`: establish an idle team, take the baseline after startup, drive `100` looks with injected scheduling, flush, assert no additional info rows. A status query alone is not a team look. |
| Warning repetition | `five_retries_are_one_warning_with_count_five`: make the bus fail in the same way across `5` driven attempts; inspect during retries and after flush. |
| Nearest nonduplicate cases | `distinct_causes_and_scopes_do_not_coalesce`: change just one scope/cause/operation component; recover then fail again; assert separate incidents. |
| Level filtering | `every_log_threshold_keeps_exactly_its_levels`: error keeps error; warn adds warn; info adds info; debug adds debug. Check absent/default config, all explicit values, and refusal of invalid values. |
| Span inheritance | `run_and_look_scopes_survive_interleaving`: simultaneous teams, nested external calls, owner channel, gate/notification jobs, explicit parent, and late span-field recording. Error-only filtering must preserve IDs. |
| Several rounds per run | `coordinator_and_worker_events_keep_all_their_rounds`: real coordinator and delegated worker; read the same event ID through each round, with no duplicate count. |
| Effective and no-op decisions | `only_effective_decisions_are_logged`: exercise formation, assignment, round transitions, and outcome handling; compare safe input/result summaries and nearest no-op branches. |
| Every outside call | `external_calls_record_elapsed_time_and_outcome`: fake git/bus/model processes and fake GitHub server; success, refusal, timeout, retry, cancellation, and injected subsecond duration. |
| No status, hang, quota, rate limit | Separate daemon scenarios for each outcome; assert warning/error and normal state-transition rows without duplicate handling warnings. |
| State changes | `accepted_state_transitions_have_before_and_after`: team, round, item, task, run start/end; unchanged summary and failed save are negative cases. |
| Bus down and waiting work | `bus_failure_names_pending_work_and_recovery`: pending work remains queued and eventually proceeds; lost/retried jobs cannot be mistaken for success. |
| Gate failures | Separate scenarios for spawn failure, timeout, and dirty tree; inspect rows and continuing daemon service. |
| Invalid questions/configuration names | Bad questions file and task referencing a removed configured agent, with adjacent valid cases and exact cause chains. |
| Invalid request | `invalid_request_is_logged_once_and_connection_continues`: malformed JSON, unsupported protocol, excessive frame, and a subsequent valid request; no raw secret-bearing payload in output. |
| Restart correction | `restart_records_each_repaired_run`: kill a scratch daemon at a controlled barrier, reopen the same isolated home, inspect correction rows and preserved work. |
| Log cannot be written | Unopenable database, newer schema, transient writer failure, queue saturation, and writer exit: stderr diagnostics, correct loss counts, no feedback rows, and a later fake task still finishes. |
| Panic hook | `task_panic_records_redacted_message_location_and_scope`: panic from an injected collaborator in a daemon task in an isolated process; verify stderr, flushed history, and no second panic error for its join. |
| Secrets absent | `secret_markers_never_reach_either_log_sink`: actual fake run with secret markers in malformed output, errors, fields, and spans; scan database and WAL bytes plus stderr. Repeat with overlapping and multibyte secrets. |
| Persistence and reader invariants | Pre-change version-`3` fixture survives migration; newer schema bytes unchanged; invalid level/name/scope/count/time ordering rejected by the API and by reading tampered rows. Hand-inserted rows prove only the reader. |
| Generated invariants | Seeded round trips, scope ordering, and sum of warning counts over interleaved sequences; print the seed on failure. |
| Startup/shutdown and human output | Capture complete stderr and read-back formatting in normalized golden files, including early failure, final shutdown, and a database failure during drain. |

Reuse the boundary mechanisms in [`crates/agent/src/daemon/owner/rounds/tests/rig.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/owner/rounds/tests/rig.rs) and the real-daemon recording approach in [`crates/agent/src/daemon/owner/rounds/tests/history_acceptance.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent/src/daemon/owner/rounds/tests/history_acceptance.rs). The queue tests in [`crates/agent-runtime/tests/history/writer.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/9a6057cedb3d68b6265f6482364bc1a9e703a453/crates/agent-runtime/tests/history/writer.rs) show deterministic ways to block and release the writer. Add new tests rather than editing those helpers or existing assertions.

For review mutation evidence, break at least these three independent behaviors: omit task/run context from a recorded warning; fail to increment duplicate count; allow a configured secret through one sink. The new tests must fail for their assertions in each case. These mutations were not performed in this investigation because there is no implementation to mutate.

## Integration sequence and size

The full feature spans many existing call sites. A preliminary non-test estimate is `3,000–4,500` changed lines, including instrumentation, storage, configuration, and documentation; this is an estimate, not a measured implementation diff. Re-estimate after enumerating event variants. If it approaches `4,000`, plan a split before coding under the relaxed `PR-2` threshold; `5,000` is the hard ceiling. Never reduce the acceptance corpus to fit.

Recommended TDD order after resolving `F3`: types/config and redaction; persistence/reader/coalescing; the layer and lifetime; real malformed-output and assignment acceptance; all remaining event producers; failure/panic/secret/restart proofs; documentation audit. Each behavior has its own compiling $\color{red}{\textbf{RED}}$ commit and $\color{green}{\textbf{GREEN}}$ implementation. Any split must retain a usable, gated intermediate state and must not start `8.7`.

## Compatibility

This report changes no runtime behavior, configuration, schema, protocol, or command output. The proposed implementation should default old configurations to `info`, migrate old history forward without losing records, keep the existing story-record API readable, and explicitly document the additional log schema. The single-writer guarantee and fail-open behavior of history are existing contracts to preserve.

## Skipped tests

None were added, skipped, weakened, or changed. Implementation tests in the table are pending work, not skipped tests in this branch.

## Not done

- No production implementation of item `8.6`.
- No claim that `A1` or `A2` is met.
- No fake-GitHub end-to-end logging run; the manual probes used a solo task without a GitHub token.
- No deterministic acceptance corpus, schema migration, coalescing, panic injection, queue-full injection, or crash/restart logging experiment.
- No automatic change to the frozen schema/table-list assertion; that needs the human's authorization if the recommended schema design is selected.
- No pull request, coordination with other investigators, or inspection of other worktrees.

## Test-first and deviations

Documentation-only investigation: exempt from test-first under `TDD-1`. No lint suppressions, test fixes, production stubs, or dependency changes. The current stderr marker exposure is reported rather than fixed without a regression test. Repository-notebook observations are recorded here because this investigator has no agentbus tools.
