# Item 7.3 — Gate service (`inv-claude-sonnet`)

Branch `agents/inv-claude-sonnet/20261113-132709`, from `7290eb86`. Not a pull request: the merger takes it in phase 2.

## Task

Item 7.3 of "Rewrite `bin/agent` in Rust": `agent gate`, run by agents in their worktree — a FIFO queue in the daemon, one gate at a
time on the machine, the repository's gate command with a timeout, the result recorded with the commit and whether the tree was
clean (a merge in progress may be gated; it is recorded as not clean). A run waiting on its own gate is never cut by the watchdog.
Expected files: `crates/agent/src/commands/gate.rs`, `crates/agent/src/daemon/gate.rs`. Checks: two `agent gate` calls at once —
the second waits, then runs, both recorded with their commit; a gate over its timeout is stopped and recorded as failed, a run
waiting on it is not cut by the watchdog.

## What and why

- [`crates/agent/src/daemon/gate.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/6a9b2f13a9b04560b00ac4b77a623d75f96a8b53/crates/agent/src/daemon/gate.rs) — the gate service: a bounded FIFO queue (`16` waiting), one job at a time, the gate
  command in the worktree in its own process group with the agents' `PATH`, its output in `tasks/<task>/gate-<n>.log`, the timeout
  (`SIGTERM`, then `SIGKILL`, of the whole group), the record, the event `gate_finished` and a line in the task's activity log.
- [`crates/agent/src/commands/gate.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/6a9b2f13a9b04560b00ac4b77a623d75f96a8b53/crates/agent/src/commands/gate.rs) — `agent gate [TASK]`: the task is the one named or the task of the worktree the command
  runs in (also through a symbolic link in `AGENTS_HOME`); prints the last 60 lines, the verdict, and for a pass on a tree that is
  not clean that it is not a pass of the commit; succeeds only for a pass.
- [`crates/agent/src/daemon/owner/gate.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/6a9b2f13a9b04560b00ac4b77a623d75f96a8b53/crates/agent/src/daemon/owner/gate.rs) — the owner's side: looks up the task, the repository's command and timeout, takes a
  hold on the watchdog of the task's run and the cancel token of that run (a stop of the task stops its gate) and queues the job
  with the client's reply, so the owner never waits for a gate.
- [`crates/agent-runtime/src/run/hold.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/6a9b2f13a9b04560b00ac4b77a623d75f96a8b53/crates/agent-runtime/src/run/hold.rs) — `WatchdogHold` / `HoldGuard`; `RunEngine::run_held`. While a guard lives the run is
  never silent; after the last guard is dropped its silence is counted from that moment.
- `GateRecord { tree: GateTree, result: GateResult }` and `Reply::GateRecorded` in `agent-core`; `Home::gate_log`,
  `Home::gate_record_file`, `Home::worktrees_dir`, `StateStore::save_gate_record` (atomic) in `agent-runtime`.
- Every run now has `AGENTS_HOME` in its environment — without it `agent gate` run by an agent cannot find the daemon.
- `Connection::request_until_answered`: the one request of the client without the 30 s limit (a gate waits behind others).
- Docs: `docs/architecture.md` ("The gate"), `docs/state.md`, `docs/configuration.md`.

## Size

`1,323` changed lines outside tests by the PR-1 command (limit `5,000` for this plan, `1,500` in PR-1; the report excluded); `29` files,
`+3,647` / `−42` in all with tests (after review pass 1). One purpose; no split needed.

## Checks run by me

- `make gate` on the last commit: `PASS` at `6a9b2f1` (cargo fmt, clippy `-D warnings` on every target, all tests, doc).
- New tests, repeated: `cargo test -p agent --bin agent gate` `5/5` times `52 passed`; `--test run_hold` `5/5` `4 passed`;
  `--test gate_process` `3/3` `1 passed`.
- Check 1 of the item, with the real binary ([`crates/agent/tests/gate_process.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/6a9b2f13a9b04560b00ac4b77a623d75f96a8b53/crates/agent/tests/gate_process.rs)): two solo tasks, `agent gate` without
  arguments run at the same moment in both worktrees — the gate script's start/end lines are `start A, end A, start B, end B`, both
  pass at the head of their worktree, both `tasks/<task>/gate.json` hold that commit and `"tree": "clean"`.
- Check 2, in process but without the binary (a timeout in the configuration is whole minutes, so the real binary would need
  one): a gate over its timeout is stopped with what it started and recorded `timed_out`
  (`daemon::gate::tests::a_gate_over_its_timeout_...`); a silent run whose gate takes 3 s with a watchdog of 1 s is not cut while the
  gate waits and runs and is cut `>= 800 ms` after it ended (`daemon::owner::gate::tests::a_run_waiting_on_its_gate_...`, with the daemon,
  the socket and the fake CLI that hangs).

## Tests (each new behaviour and the test that specifies it)

| Behaviour | Test |
|---|---|
| two gates waiting run in order, never overlapping | `daemon::gate::tests::two_gates_waiting_run_one_after_the_other_in_the_order_they_were_asked` |
| a timeout: stopped with its group, recorded `timed_out`, the next gate runs | `..::a_gate_over_its_timeout_...`, `..::the_next_gate_runs_after_one_that_timed_out` |
| record of a clean pass / a failure / a merge in progress / an untracked or changed file | `..::a_gate_that_passes_on_a_clean_tree_...`, `..::a_gate_that_fails_...`, `..::a_gate_on_a_merge_in_progress_...`, `..::a_gate_on_a_tree_with_an_untracked_or_a_changed_file_...` |
| a run waiting on its gate not cut by the watchdog | `run_hold` (engine), `daemon::owner::gate::tests::a_run_waiting_on_its_gate_...` (daemon), `..::a_task_with_a_run_going_has_its_watchdog_held_...` (job) |
| stop of the task stops its gate; a waiting gate of a stopped task is not run | `..::stopping_a_task_stops_its_gate_...`, `daemon::gate::tests::a_gate_of_a_task_that_was_stopped_while_it_waited_is_not_run`, `..::a_gate_that_is_stopped_while_it_runs_...` |
| refusals: unknown task, repository lost, full queue, no service, worktree git cannot read, gate command that cannot start | `daemon::owner::gate::tests::*`, `daemon::gate::tests::a_worktree_git_cannot_read_...`, `..::a_gate_command_that_cannot_be_started_...` |
| the client waits beyond the 30 s of a request | `commands::gate::tests::a_gate_that_waits_for_its_turn_is_waited_for_...` (paused clock) |
| the task of a directory, also through a link | `commands::gate::tests::*task*` |
| `AGENTS_HOME` in the run's environment | `run_hold::the_run_gets_the_home_in_agents_home_...` |

Every error variant of `GateError` has a test except `Path`, `EnvName` and `Supervise` (see Deviations).

Existing tests modified or removed: **none**. The only existing test-side file touched is the stand-in `fake-claude.sh`, where I
added the scenario `env-home` (a new action; the others are unchanged).

## Look at first (existing test that no longer tests what its name says)

`daemon::tests::commands_the_daemon_does_not_serve_yet_are_refused_with_their_name` asserts that `gate` of task 1 is `refused` with
a message containing "gate". It is unchanged and **passes**, but only because its fixture's worktree is a plain directory, not a git
checkout, so the gate is now refused with "the gate cannot run in ...: git cannot read the worktree". The daemon serves `gate`
now; the test's name and intent are out of date. Rule 1 forbids me to change it: the human or the merger should replace it with a
test of another not-yet-served command, or delete it.

## Deviations

- **`GateResult` and `Reply::Gate` are frozen by existing tests** that build them with struct literals
  (`state/item.rs`, `protocol/reply.rs`), so they could not take a `tree` field without changing those tests. I added `GateRecord`
  (`tree` + `result`) and the reply `GateRecorded`; `Reply::Gate` stays readable and the daemon no longer sends it. If the human
  allows editing those two test literals, folding `tree` into `GateResult` and `Reply::Gate` is a smaller model.
- **The record is a file, not in the state** (`tasks/<task>/gate.json`, the last record of the task): `ItemProgress::gate` is per
  item, and the item a gate belongs to (a coordinator's integration) is item 8.1's knowledge. The state is unchanged, so no schema
  or migration. 8.1 reads the record from there (or the service).
- **A tree that is not clean is not refused**, only recorded as not clean (the old script refused it for an agent's own task). I read
  "a merge in progress may be gated; it is recorded as not clean" as: gate always, say what the tree was. The pass of a tree that is
  not clean exits `0` (a coordinator checks a merge before it commits it) and says it is not a pass of the commit.
- **No new fields in `OwnerParts` and `Timings`**: existing test helpers build them with literals. The owner gets the queue with
  `Owner::with_gate` (`None`: no service, every gate refused); the gate's stop grace is the engine's.
- **Files beyond the two of the item**: the item's two files plus `owner/gate.rs`, `owner.rs`, `daemon/mod.rs`, `main.rs`, `client.rs`,
  `commands.rs`, `commands/status.rs` (a new reply kind), `agent-runtime` (`run.rs`, `run/hold.rs`, `run/launch.rs`, `store.rs`), `agent-core`
  (`state/gate.rs`, `state.rs`, `protocol/reply.rs`) and the docs — each is needed for the item to work.
- `outcome_of` has one `unwrap_or(NonZeroI32::MIN)` (a signal sum of zero cannot happen; it would still be a failure; tested). `String::from_utf8_lossy`
  in the tail of the log (the log is for a person; commented). `GateError::Path`, `::EnvName` and `::Supervise` have no test: a path of
  the home is UTF-8 by construction, `PATH` is a valid name, and a failing `wait`/`stop` of a process cannot be provoked.
- No lint suppressions, no new dependencies, no unsafe.
- Intermediate commits: `ace24dd8` (service) and `f14abe9f` do not pass clippy alone (`dead_code` until the next commit wires the
  service); the final commit passes `make gate`.

## Not done / known limits

- A gate whose client went away (the agent was killed) is not noticed while it waits or runs; only a stop of its task, the daemon's
  shutdown or its timeout end it (the server does not watch a connection while it waits for the owner).
- `agent gate --wait` and `agent gate-status` of the old script, and the old prompts that name them (`prompts/protocol.md`,
  `tdd-rounds.md`), are not ported; the Rust prompts catalog does not mention `agent gate` yet. At the cutover the prompts need the
  new behaviour (always waits; a dirty tree is allowed and recorded).
- The queue and the waiting gates are lost when the daemon stops; the last record of each task is kept.
- Item 8.1 must read `tasks/<task>/gate.json` (`GateRecord::counts_as_pass`) for "gate green at the head".

## Suggestions for `AGENTS.md` (not changed by me)

- State that a struct built by literal in an existing test cannot get a field in a later item, and tell the planner to leave room
  (a constructor in the RED commit of the item that adds the type): this cost a parallel `GateRecord` here.
- The "RED commit" bodies of an item that wires something new into `OwnerParts`-like structs should come with builders in the earlier
  item, for the same reason.

## Review pass 1 (`inv-kimi-deepinfra`, [review](https://github.com/mateusz-szczyrzyca/team-agents/pull/96#pullrequestreview-5892004126))

- **Major 1 — only the state before the command was recorded.** Right: a gate that appended to a tracked file was recorded as a
  clean pass. `daemon::gate` now reads the worktree again after the command and its process group have ended; the tree is
  recorded `clean` only when it was clean at both ends and `HEAD` did not move. A gate that changed the tree or committed is
  `not_clean`, at the commit it started from, so `counts_as_pass()` is false. Tests (RED first): `a_gate_that_changes_a_tracked_file_...`,
  `a_gate_that_leaves_an_untracked_file_...`, `a_gate_that_commits_...`, `a_gate_that_fails_after_changing_a_file_...`, and the guard
  `a_gate_that_puts_the_tree_back_as_it_found_it_is_recorded_as_clean`. Consequence: the files a gate leaves in the worktree must
  be ignored by git (`target/`, `build/`), otherwise its pass is not a pass of the commit; documented in `docs/architecture.md`.
- **Major 2 — a stop did not stop the gate of a task with no run.** Right: such a gate had only the daemon's shutdown token. The
  owner now keeps a token per task for gates asked while no run is going (`Owner::gate_stops`) and cancels it when the human stops or
  pauses the task (`move_task`, `halt`). Tests (RED first): `stopping_a_task_with_no_run_stops_its_gate`,
  `pausing_a_task_with_no_run_stops_its_gate`, `stopping_one_task_leaves_the_gate_of_another_task_with_no_run_going`. A token of a
  task that is never stopped stays in the map until the daemon ends: one entry per task that asked for a gate.
- **Major 3 — the PR description.** The block "Round size" is written by the script and counts tests: `agent round-size` says `3,829`
  lines in `30` files, over the `3,000` of the general round limit; the plan's PR-1 limit (tests not counted, `5,000` here) is what
  applies to a PR of this plan, and the PR-1 command gives `1,323` outside tests (the report excluded). The description now has
  the what and why, the checks, the deviations and the reason of every file outside the two of the item; this report has the detail.

## Commits

- [`b5480ac`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b5480ac2fcf6ddfdff578b838f4a328f3d640e3c) RED: a gate record says whether the tree was clean
- [`397f7f7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/397f7f7b79bb0a362faa966195901aea8bbe8dde) GREEN: a gate record counts as a pass only on a clean tree
- [`80df5fe`](https://github.com/mateusz-szczyrzyca/team-agents/commit/80df5febfa15fee8638d422de730befd92732a83) RED: a run waiting on its gate is not cut by the watchdog
- [`c537833`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c5378337e8ede3daa96775fac5b1b06d086bae63) GREEN: hold the watchdog off a run that waits on its gate
- [`c5f8544`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c5f85440ac68fdfdb076cd11c49dadb5d3d13ec8) RED: the gate of a task keeps its log and its record in the task's directory
- [`5b1c9db`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5b1c9db2c95a7f7f92c25f9d7919f95f8265eb3c) GREEN: the gate log and the gate record are files of the task
- [`66119e7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/66119e7e5eb131a6b5b463ea30ee3d1c1c459fb0) RED: the gate service runs one gate at a time and records each
- [`2de14b6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2de14b637bb0ed4c3c5beff05afa5f35382f6809) RED: a gate record has a summary in a few words
- [`f14abe9`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f14abe9fd60b6a73e5c01c950d5ede8b4f99faed) GREEN: the summary of a gate record
- [`ace24dd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ace24dd8dc914ee113d17dd02745814fefa35c9e) GREEN: the gate service runs the queue one gate at a time
- [`a6b86ef`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a6b86ef4c1f8b448150bd760304928611ca7b8c6) RED: the owner queues a gate and holds the watchdog off the run that waits
- [`ec63300`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ec633006fa4e94ec160ed250514dfd86828a2677) GREEN: the owner queues the gate of a task and holds the watchdog off its run
- [`c51d17c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c51d17ca94cdb227a4c195274a17edef52eb95f3) RED: agent gate asks the daemon and says what the gate said
- [`cdc37ff`](https://github.com/mateusz-szczyrzyca/team-agents/commit/cdc37ff77f2cfb02927a77acd13d80c56298e4f1) GREEN: agent gate asks the daemon and says what the gate said
- [`09ae9a5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/09ae9a5a420b6ea6f420db1f26de3172308af701) Test the gate as a process: two agent gate calls at once
- [`0ee3610`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0ee361056e64fcb3cbdef95349c1d66d046f67df) Document the gate service
- [`4079057`](https://github.com/mateusz-szczyrzyca/team-agents/commit/40790574ceb4fb2de61a8624d9ade8290a9dfcd2) Fix a clippy finding in the test of agent gate as a process
- [`f0e30cd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f0e30cd686b3a6ed4c17916dc47da8a8f179f058) Test the failures of the gate that are not its own
- [`6a9b2f1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6a9b2f13a9b04560b00ac4b77a623d75f96a8b53) Name gate in the doc comments of the command line
