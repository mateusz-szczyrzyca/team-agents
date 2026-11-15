# Review of [#102](https://github.com/mateusz-szczyrzyca/team-agents/pull/102) against round 8.6 — `inv-opus`

Reviewed at the PR head [`ed597ff`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ed597ffe9a179c3fd35e69f9e680b45299e9abe5)
(base `main` at [`e8940da`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e8940dabbb8a8545bc5a78ab72bd30d607707a8e)),
against item 8.6 of [`rounds/round-8.6.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/rounds/round-8.6.md),
the review rules (`prompts/investigator/code-review.md`) and the developer rules (`prompts/agent/rust-developer.md`).
Every line number below is at `ed597ff`.

```text
VERDICT: REQUEST_CHANGES
SUMMARY: The PR adds the `[log]` level, a 25-name `LogEvent` enum, migration 4 (`daemon_logs`) and a `HistoryLayer`,
  and A1/A2 pass. But the layer drops every event that has no `event` name — 108 of the 157 events of the daemon,
  "a run failed" among them — from both the history and stderr, where they used to be printed. 21 of the 25 names are
  never provoked by a test, and four mutations of recorded behaviour leave the suite green. Several required spans,
  decisions and outside calls are missing, and the PR description cites tests that do not exist.
SIZE: 3135 changed lines outside tests (limit 5000, relaxed for this plan)
GATE: `agent gate` PASS at ed597ff (make rust: fmt ok, clippy ok, test ok, doc ok)
```

## Criteria

- **A1 met** — `daemon::owner::rounds::daemon_log_tests::malformed_cli_line_leaves_cli_output_unreadable_warn_row_and_debug_assignment_logs_decision_taken`
  passes at the head (`728/728` tests of the `agent` binary pass). It runs the daemon with fake CLIs and finds a `warn`
  row `cli_output_unreadable` with the team, the round, the task and the run, and a `debug` row `decision_taken`
  (`assignable`) with its `input` and `result`
  ([`daemon_log.rs#L39-L131`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/owner/rounds/tests/daemon_log.rs#L39-L131)).
  At its RED commit [`87e169e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/87e169ee) it fails on the
  assertion at `daemon_log.rs:94` (`expected cli_output_unreadable row in daemon_logs`).
- **A2 met** — `log::tests::docs_log_md_lists_every_log_event_name_and_its_level`
  ([`log/tests.rs#L171-L198`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-core/src/log/tests.rs#L171-L198))
  parses the table of `docs/log.md` and compares it with `LogEvent::ALL` and each level; it passes in the gate. The
  evidence the PR gives for A2 is wrong, though (blocker 13).

## Blockers

1. [COR] [`history/log.rs#L569-L579`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/history/log.rs#L569-L579),
   [`commands/daemon.rs#L37`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/commands/daemon.rs#L37)
   — the `fmt` subscriber that printed every event to stderr is removed, and `on_event` returns without writing anything
   for an event without an `event` field or with an unknown name. That is `108` of the `157` `tracing` events in
   `crates/agent/src` and `crates/agent-runtime/src`, among them "a run failed" (`owner.rs:713`), "a run cannot be
   started", "a task of the daemon failed" (`daemon/mod.rs:329`), "the daemon starts" (`commands/daemon.rs:61`) and
   every `server.rs` error. A temporary test (reverted) emitted three such events into a `HistoryLayer` with a stderr
   sink and an open history and got `rows=0 stderr=[]`. The item says "Every `tracing` event of the daemon goes to
   stderr as now and into the history" → print every event to stderr, give every event of the daemon a name (or record
   unnamed ones), and add a test that fails when an event of the daemon is lost.
2. [TDD-2] [`daemon_log.rs#L323-L435`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/owner/rounds/tests/daemon_log.rs#L323-L435)
   — the test that walks the enum writes each name itself (`tracing::warn!(event = variant.as_str())`), so it cannot fail
   for a name that production never produces. Only `4` of the `25` names are provoked through production code
   (`cli_output_unreadable`, `bus_down`, `decision_taken`, `outside_call_completed`); the other 21 appear in tests only
   as hand-written events or hand-built records. Mutations that survived, with the suite at `728/728` green:
   `task_status_changed` recorded as `run_ended` (`owner.rs:1306`, a break of what is recorded), and `run_hung` emitted
   under a misspelt name (`owner.rs:1490`) → for every name, a test that provokes its cause with the fakes (item 8.6,
   "Tests").
3. [TDD-2] [`team/assign.rs#L81`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/team/assign.rs#L81)
   — `if !decided.is_empty()` changed to `if true` survives: no test at `debug` checks that "a decision that does nothing
   is none" → add a test that looks at a team with nothing to assign at `debug` and finds no `decision_taken` row.
4. [TDD-2] [`owner/teams.rs#L300`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/owner/teams.rs#L300)
   — replacing the span of a look with `tracing::Span::none()` survives: nothing tests "events of a look carry the team"
   (every tested event passes `team` itself or runs inside a span the test made by hand) → add a test of an event inside
   a look that has no `team` field of its own.
5. [COR] Decisions the daemon takes from `agent_core::decide` that log no `decision_taken`: `plan_team`
   ([`team/form.rs#L176`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/team/form.rs#L176)),
   `RoundState::transition`
   ([`merge.rs#L185`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/merge.rs#L185),
   `#L201`, [`publish.rs#L388`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/owner/rounds/publish.rs#L388)),
   `first_free_worker`
   ([`owner/teams.rs#L593`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/owner/teams.rs#L593)),
   `classify_outcome`
   ([`run.rs#L431`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/run.rs#L431)),
   `rounds_of_run`
   ([`round_state.rs#L59`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/owner/round_state.rs#L59)).
   The item asks for "every decision of `agent-core::decide` the daemon takes" → log each one that does something, and
   test each.
6. [COR] Spans and outside calls that are missing: no span is opened around a call of git
   ([`git.rs#L897`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/git.rs#L897)),
   the agentbus
   ([`agentbus.rs#L1241`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/agentbus.rs#L1241))
   or the GitHub API
   ([`github.rs#L1373`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/github.rs#L1373)),
   nor around a request served (`server.rs`, the request loop in `owner.rs`); a model CLI run logs no
   `outside_call_completed`. The item: "spans that every look at a team, every run, every request served and every call
   of git, the agentbus, the GitHub API and a model CLI opens", and at `debug` "every outside call with how long it took
   and how it ended" → open the spans, log the model CLI call, and test both.
7. [COR] [`github.rs#L1416`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/github.rs#L1416)
   — a GitHub call that is tried again logs an unnamed `debug` event, which blocker 1 drops; the item asks for a `warn` on
   "a git or GitHub call that fails or is tried again". At
   [`github.rs#L1411`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/github.rs#L1411)
   `error = %error` logs only the top error, not "the whole chain of its causes" → a named `warn` for a retry, and
   `error_chain(&error)`.
8. [API-9] [TDD-12] [`history/log.rs#L35-L58`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/history/log.rs#L35-L58),
   [`#L589-L632`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/history/log.rs#L589-L632)
   — new global mutable state: `SPAN_SCOPES`, `GLOBAL_LAYER`, `GLOBAL_STDERR_ENABLED`, `PANIC_LAYER` and two
   `thread_local!`s. `HistoryLayer::new` installs a thread-default subscriber, never removed, that drops every event;
   its comment says it exists for when "another test thread installed" a global subscriber. The daemon runs on a
   current-thread runtime
   ([`main.rs#L550`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/main.rs#L550)),
   so on that one thread every event outside an explicit `.with_subscriber(...)` is lost; `daemon/mod.rs:313-333`
   works around it with `dispatcher::with_default`. → build one subscriber in `main` and pass the layer in, with no
   statics and no thread-locals.
9. [TDD-8] [`history/log.rs#L636-L649`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/history/log.rs#L636-L649)
   — `install_panic_hook` in tests (`daemon_log.rs:343`, `log/tests.rs:206`) sets a process-wide hook that is never
   removed and does not call the previous hook. Panics of other tests go into that test's layer, and their usual message
   is not printed; the run of the suite showed `daemon_panicked … thread daemon::history::tests::an_opener_task_failure_reports_unavailable_and_counts_every_lost_record`.
   The panic test also uses `catch_unwind` on the test thread, not "a panic in a task of the daemon" → inject the hook,
   and test a panic in a task of a daemon run.
10. [ERR-2] In `history/log.rs`: a clock failure drops the event silently (`#L241`); a scope ID that does not parse is
    dropped silently (`#L363-L398`); an unknown event name is dropped (`#L577`); `drop(…try_init())` (`#L628`);
    `unwrap_or_else(|_| …)` throws the serde error away (`#L281`, `#L660-L661`). And
    [`owner.rs#L567`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/owner.rs#L567):
    `if let Ok(located)` with the `Err` not handled → propagate the error, or report it on stderr as the log's own
    failure.
11. [TYP-6] [`owner.rs#L587`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/owner.rs#L587)
    — `_ => {}` on `StartProblem`, an enum of the workspace → list its variants.
12. [PERF-3] [`write.rs#L543-L595`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/history/write.rs#L543-L595)
    — every `warn` insert scans the earlier `warn` rows of the whole database, with no bound by time or by the last row:
    `O(n²)` over a history that keeps growing. It also merges a warning with an identical one from days before and moves
    that row's `at`. The reader adds an N+1 query per row
    ([`reader.rs#L1255`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/history/reader.rs#L1255))
    → merge only with the newest row in the same scope, behind an index, and read the rounds in one query.
13. [GEN-7] [PR-9] PR description: the A2 command `cargo test -p agent-core --locked docs_log_md_lists_every_log_event_name_with_matching_level`
    runs `0` tests (I ran it), so its quoted output `1 passed; 384 filtered out` is not that command's output. The
    `## Tests` table cites `log_config_defaults_to_info_parses_all_levels_and_rejects_unknown_level` and
    `log_event_round_trips_and_maps_to_expected_severity`, which do not exist. "For every event name a test provokes its
    cause" is untrue (blocker 2) → correct the names, the commands and the claims.
14. [COR] Ground rule "No secret reaches the history or the log: a test runs a round with marker values for the
    secrets" —
    [`log/tests.rs#L228`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/history/log/tests.rs#L228)
    writes hand-made events into a layer; no daemon round runs with marker secrets while the bytes of `daemon_logs` are
    searched → add that end-to-end test.

## Minor

1. [TYP-4] [TYP-3] [`history/log.rs#L652`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/history/log.rs#L652),
   `#L672` — `decision`, `tool`, `operation` and `outcome` are `&str`, and three of them can be swapped by accident → enums.
2. [API-10] [`github.rs#L1382`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/github.rs#L1382)
   — every successful GitHub call is recorded with `exit_code` `Some(200)`, whatever the real status was → the real
   status, or none.
3. [TYP-4] [`owner.rs#L845`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/owner.rs#L845)
   — an empty `String` stands for "no run" in the span → `Option`.
4. [OBS-3] [`git.rs#L956`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/git.rs#L956)
   — `git_call_failed` is logged when the error is built, not where it is handled. So expected answers become `warn`
   rows (the run of the suite showed `origin url`: "No such remote 'origin'"), and callers log the same failure again;
   `owner.rs:1428` logs `state_save_failed` and also returns the error.
5. [OBS-2] [`gate.rs#L298`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/gate.rs#L298)
   — `tracing::error!` for `gate_cannot_start`, whose level is `warn`. The macro's level and the enum's level are two
   sources of truth; nothing keeps them equal.
6. [API-12] [`history/log.rs#L435`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/history/log.rs#L435)
   — `let _ = name;` binds nothing useful.
7. [OBS-2] [`owner/teams.rs#L345`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent/src/daemon/owner/teams.rs#L345)
   — "the bus refuses a request for good: it is dropped" went from `error` to `warn`, although someone must act on it.
8. [TYP-8] [`reader.rs#L1315`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ed597ffe9a179c3fd35e69f9e680b45299e9abe5/crates/agent-runtime/src/history/reader.rs#L1315)
   — `pos.get() as usize` → `usize::try_from`.
9. [GEN-1] `HistoryLayer::install_global` is called twice for `agent daemon`, at `main.rs:571` and at `commands/daemon.rs:37`.
10. [TDD-1] The RED commit [`87e169e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/87e169ee) gives the
    names of the failing tests but not the command and the failing lines.

## Accepted deviations

- [TDD-14] [`36d558f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/36d558fc401b37d890772669d8b57e640a782412)
  `TEST-FIX:` — the schema version `3` → `4` and the two new tables in `schema.rs`, `sessions.rs` and `ledger.rs`. Migration
  4 makes them necessary, and the human allowed it (`questions-1.md`, question 1, verdict a).

## Needs human

- Existing tests changed by `TEST-FIX` [`36d558f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/36d558fc401b37d890772669d8b57e640a782412)
  (accepted above).
- The fix for blocker 1 decides what the journal of `agent daemon` shows from now on: every event as before, or only the
  named ones.

## How this was checked

- Size: the PR-1 command against `e8940da` → `3135` lines; this includes the `255` lines of
  `docs/investigations/round-rust-agent-8-6/inv-astra.md`.
- Gate: `agent gate` in this worktree → `PASS at ed597ff`.
- Test integrity: the step-4 commands of the review rules found no removed, ignored or loosened test, either against the
  base or against any RED commit; the GREEN commits touch no test file.
- RED proved: [`87e169e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/87e169ee) builds; its tests in
  `agent-runtime` fail on their assertions (`log/tests.rs:122`, `:170`, `:209`, `:324`), and A1's test fails at
  `daemon_log.rs:94`.
- Behaviours broken for a moment (all reverted; the worktree is clean):
  - `task_status_changed` recorded as `run_ended` → `728/728` green;
  - `run_hung` misspelt → green;
  - the assignment decision logged when nothing is decided → green;
  - the span of a look removed → green.
- The lost events: a temporary test, then reverted.
- Event count: every `tracing::{error,warn,info,debug,trace}!` in non-test code of `crates/agent/src` and
  `crates/agent-runtime/src`, with or without an `event =` field.
