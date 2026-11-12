# Round rust-agent/4.1 — `inv-claude-sonnet`

Item 4.1 of the plan "Rewrite `bin/agent` in Rust": the local effects of `agent-runtime` — the state store, the prompt
loader, the process supervisor, typed git operations and notifications. Branch `agents/inv-claude-sonnet/20261112-122552`,
based on `196f2e1`. Nothing outside this item was started.

## What and why

The daemon of later items needs five local effects, each behind a small typed boundary, tested on real temporary
files, repositories and processes:

- [`crates/agent-runtime/src/store.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/cac4d9d6c153f295c82d2c1a108ec6e19c67fd69/crates/agent-runtime/src/store.rs) — `Home` (the layout of `AGENTS_HOME`), `StateStore` (`open` = create the home, take an
  exclusive `flock` on `daemon.lock`, sweep the temporary files of a crashed save; `load`; `save` = synced temporary file in
  the home + rename + directory sync; `append_activity`), `StoreError`.
- [`crates/agent-runtime/src/prompts.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/cac4d9d6c153f295c82d2c1a108ec6e19c67fd69/crates/agent-runtime/src/prompts.rs) — `PromptLoader::load(dir, language)` reads every prompt of the catalog (language file,
  then English; at most 1 MiB), checks each against the catalog and reports **all** problems at once (`PromptLoadError::problems`);
  `render` fails only on its values.
- [`crates/agent-runtime/src/process.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/cac4d9d6c153f295c82d2c1a108ec6e19c67fd69/crates/agent-runtime/src/process.rs) — `Supervisor` (at most N processes at once), `ProcessSpec`, `RunningProcess`
  (`wait`, `last_activity`, `stop(grace)`), `ExitReport`; inside the crate `capture` (a helper run bounded in time and bytes).
- [`crates/agent-runtime/src/git.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/cac4d9d6c153f295c82d2c1a108ec6e19c67fd69/crates/agent-runtime/src/git.rs) — `Git` (one checkout, one `CommitIdentity` per run): `rev_parse`, `branch_tip`,
  `branch_create`, `branch_delete`, `worktree_add`, `worktree_remove`, `tree_state`, `merge_state`, `merge_no_ff`, `merge_abort`,
  `push`.
- [`crates/agent-runtime/src/notify.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/cac4d9d6c153f295c82d2c1a108ec6e19c67fd69/crates/agent-runtime/src/notify.rs) — `Notification` (one-line title, bounded body), `Notifier::from_config` / `notify`.

## Task

Quoted from `rounds/round-4.1.md`, "This round's item":

> 4.1 State store, prompts, processes, git and notifications — `agent-runtime`, local effects. The `AGENTS_HOME` layout, atomic
> writes (temporary file and rename — SEC-5), a lock that keeps one daemon per home (`rustix` flock), schema-version check, an
> append-only activity log per task. The prompt loader: reads the catalog's files from the configured directory, validates all
> at startup, applies the language override, renders. The process supervisor: argument vector, environment and working
> directory (`tokio::process`, never a shell string — SEC-3), its own process group, stdout and stderr to files, last-activity
> time, stop of the whole group with a timeout, exit status, everything bounded (CON-5). Typed git operations over the `git`
> command — worktree add and remove, branch, merge without fast-forward, merge in progress, clean-tree check, rev-parse, push —
> with commit identity and signing per run, tested on real temporary repositories. Notifications: the file (title line, body
> already rendered by the caller) and the configured command run with its path; secrets only in its environment.
> Expected result: Files `crates/agent-runtime/src/{store,prompts,process,git,notify}.rs`. Adds `StateStore` (load, save, home lock,
> activity append), `PromptLoader`, `Supervisor` and `RunningProcess`, the git operations, `Notifier`. Tests: atomic save and a
> second lock refused; prompt files missing or with wrong variables; a process group stopped whole and a timeout; git operations
> on temporary repositories; the notification command called with its file and environment.
> Checks: (1) a second lock of the same home fails with the typed error while the first is held; (2) a supervised process that
> started children leaves none of its group behind after a stop (a test checks with the process table); (3) a save interrupted
> before its rename leaves the old state whole and no temporary file in the home.

## Checks of the item

Run `cargo test -p agent-runtime <name>`; every one passes.

- (1) `store::tests::a_second_open_of_the_same_home_is_refused_while_the_first_is_held` — `Err(StoreError::HomeLocked { path })`
  with the lock file's path; `the_home_can_be_opened_again_after_the_store_is_dropped` shows the lock goes with the store.
- (2) `process::tests::stop_leaves_no_process_of_the_group_behind` starts two `sleep` children (one ignoring `SIGTERM`), reads
  their IDs, stops the leader and waits until `/proc/<pid>/stat` shows them gone or zombie. Also
  `stop_kills_a_leader_that_ignores_sigterm_after_the_grace`, `stop_after_the_leader_ended_kills_what_it_left_behind`,
  `dropping_a_running_process_kills_its_group`, `capture_kills_the_group_of_a_run_that_takes_too_long`. Mutation check done by
  hand: removing the last `SIGKILL` of `stop` makes two of these fail.
- (3) `store::tests::a_save_interrupted_before_the_rename_leaves_the_old_state_and_no_temporary_file` (stages the new state,
  drops it before the rename: `state.json` still the old text, home holds only `state.json`, `daemon.lock`, `tasks`), plus
  `open_removes_the_temporary_file_of_a_crashed_save_and_keeps_other_files` (a killed process) and
  `a_failed_rename_is_a_replace_error_and_leaves_no_temporary_file`.

## Size

`2,505` changed lines outside tests (limit `5,000`; the PR-1 command, base `196f2e1`), in `10` files; `Cargo.lock` `+199`
(not counted). `agent-runtime` has `112` tests; the rest of the workspace is untouched.

## Tests, in the order of the history

- [`ed6052e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ed6052eeb50ea6653d283485736f4a09cc2274cc) State store: red
- [`ce94c6e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ce94c6e9e67c2b92ce88aaef93d8c027aa43198d) State store: green
- [`8ca5302`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8ca5302bec1f9094427fde5565f64047278fbdb6) Process supervisor: red
- [`05606a2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/05606a2d2966f83419ff31713662f92ce9b1f9d4) Process supervisor: green
- [`835b30c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/835b30cea43c7cca29b839d1aa38cbbffbd7e87c) Git operations: red
- [`1e22619`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1e2261940cc30553ff843edeb81e17905a536e0e) Git operations: green
- [`750b04a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/750b04ac95947d85e2a6a0d18f8ad71602df3def) Store tests: reopening waits out a forked child
- [`dbcf453`](https://github.com/mateusz-szczyrzyca/team-agents/commit/dbcf4538a5608610b2a1e3431ed1ad743e53083c) Git merge in progress: red
- [`d37b195`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d37b19548b4ad7fb5974839ea9d0072f869bab75) Git merge in progress: green
- [`28f4bdd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/28f4bdd4ab7cc204f830ee720d6a04eb5f6dfe62) Prompt loader: red
- [`19ec6fc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/19ec6fc7c281cfaf1b7a8f24e2db85deb0ff2bf0) Prompt loader: green
- [`963ef4f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/963ef4f32c147cc5a48142a9d19bfd2f0db750ef) Notifications: red
- [`4e241fa`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4e241facabad85cf0e2f495915a708654fe8181e) Notifications named after their time: red
- [`79abeaf`](https://github.com/mateusz-szczyrzyca/team-agents/commit/79abeafc3b5fed3fc8a12609938f90b82c1a2970) Notifications: green
- [`010cd01`](https://github.com/mateusz-szczyrzyca/team-agents/commit/010cd01fc021105bfadda6ef38ac6266728c8e90) u64 limits and tests for refused answers
- [`cac4d9d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/cac4d9d6c153f295c82d2c1a108ec6e19c67fd69) Architecture document

$\color{red}{\textbf{RED}}$ then $\color{green}{\textbf{GREEN}}$ for each of the five modules, as `docs/tdd.md` asks. Two cycles came from the code itself: a second
merge while one waits for its conflicts would have been reported as a new conflict (`MERGE_HEAD` is still there) — found while
writing `merge_no_ff`, written as a test first ($\color{red}{\textbf{RED}}$ `dbcf453`, $\color{green}{\textbf{GREEN}}$ `d37b195`); the notification file name changed
from a random name to `notification-<time>-<n>.md` (the $\color{red}{\textbf{RED}}$ commit `4e241fa` changes the tests of `963ef4f` together with the
signature). The last code commit adds characterisation tests for refusals that had none (`Test-First` line in its message).

No existing test was touched: the only test files of the repository changed are the ones this branch created.

## Gate

- `make rust` — `cargo fmt --check`, clippy `-D warnings`, tests, `cargo doc` with warnings denied, all `--locked`: pass.
- `GATE_WAIT=1 make gate` — `gate: PASS at cac4d9d`.
- `cargo test -p agent-runtime` repeated five times in a row after the fix of the reopen race: `112 passed`, no flake.

## Compatibility

New code only: no file of `~/agents`, no `agentbus` output, no `bin/agent` behaviour changes. The state file is the core's
schema 1 (`TestLegacyStateLoads`-style loading is the core's `state-v1.json` fixture, which `store::tests` round-trips byte
for byte). The activity log format is new (`{"at": RFC 3339, "kind": text | command | file_edit | bus_call | tool_call | error |
info, ...}`).

## Deviations

Lint suppressions (`#[expect]`): none. `unsafe`: none. No test was ignored or skipped.

Files outside the expected list, with the reason:

- `Cargo.toml`, `Cargo.lock`, `crates/agent-runtime/Cargo.toml` — the dependencies of the runtime. All four are on the approved
  list of the plan: `tokio` (features `rt`, `process`, `time`, `io-util`, `fs`, `sync`, `macros` — `macros` for `tokio::join!` in
  the library; without it `cargo doc -p agent-runtime` failed), `rustix` (`std`, `fs`, `process`: `flock` and `killpg` without
  `unsafe`), `tempfile` (atomic writes, SEC-5), `tracing` (`std`: the two `warn` records of cleanup that cannot be returned).
- `crates/agent-runtime/src/lib.rs` — the module declarations.
- `docs/architecture.md` — a section that says what each runtime module does.

Rules read with a judgement call, for the reviewer:

- **ERR-2 fallbacks.** `store.rs::state_read_limit` and `prompts.rs::read_file` turn a `usize` limit into `u64` with a fallback
  that cannot be reached (comment on each). `process.rs` matches the `Elapsed` of a `timeout` as `Err(_grace_over)`,
  `Err(_wait_over)`, `Err(_time_over)`: the elapsed deadline carries no information and each arm acts on it.
  `git.rs::diagnostic`, `notify.rs::diagnostic` use `from_utf8_lossy` on purpose: the text is for a human (comment on each).
- **ERR-1(c).** Two cleanups log instead of return: `RunningProcess::drop` (cannot return) and the reaping of a captured process
  that was killed for running over its time (the timeout is the error being returned).
- **Error variants without a test** (no public way to provoke them): `StoreError::{Lock, ListDir, Render, RenderActivity,
  SyncDir, Worker}`, `ProcessError::{Pid, Wait, Signal, StopTimeout}`, `CaptureError::{Pid, MissingPipe, Io, Signal}`,
  `PromptProblem::Catalog`, `NotifyError::{Write, Worker}`. Each wraps a failure of the operating system or of a thread that
  cannot be produced deterministically (a `flock` failing other than `EWOULDBLOCK`, a `JoinError`, a `kill` refused). Say if you
  want one of them removed rather than carried.
- **TDD-8.** The two tests that reopen a home right after dropping its store retry `HomeLocked` for a bounded time: a child that
  another test of the binary forks holds the open file description of the released lock from `fork` to `exec` (seen once in two
  runs while the 64 MiB state test made forks slow). The refusal test has no retry. Process tests use real short timeouts
  (`200 ms`), not the paused clock: a paused runtime auto-advances while it waits for a real child and would fire them at once.

## Decisions the merger may want to compare with the other investigators

- `StateStore::open` takes the lock and *is* the proof of ownership; there is no separate `HomeLock`. A save is `stage` (a
  `pub(crate)` step, the injection point of the interruption test) then `commit`.
- Activity logging takes the time from the caller (`Timestamp`); texts are cut at 4 KiB on a character boundary with `…`.
- `last_activity` is the later `mtime` of the two output files (or the start): no pump task, no memory; the files are the only
  copy of the output, which is what the later adapters read.
- The supervisor **refuses** at its limit (`ProcessError::Limit`) instead of waiting; the daemon decides whether to queue.
- `stop` always ends with a `SIGKILL` of the group after the leader is reaped. The window in which the group ID could be reused
  is the time between the reap and the signal, and a group ID cannot be reused while any process is still in the group.
- Git: the handle is per checkout and per identity; `commit.gpgsign=false` is passed when there is no key, so that the human's
  signing never leaks in; `GIT_TERMINAL_PROMPT=0` and `LC_ALL=C` always; `with_env` is how a test isolates the configuration
  (`GIT_CONFIG_GLOBAL=/dev/null`) and how the daemon will pass credentials. A branch is spelled `refs/heads/<name>`. No `fetch`
  (not in the item). A conflicting merge is a value (`MergeOutcome::Conflicted`), `merge_no_ff` refuses while a merge waits.
- Notifications are named `notification-<time>-<n>.md` (≤ 100 per second), mode `0600`; `notify` takes the time from the caller.
  What the command prints is stripped of every secret value *before* it is cut.

## Out of scope

- The old `bin/agent` and Go code were not read for porting. `agentbus`, GitHub API (item 4.2) and the adapters (item 5) were not
  started.
- `PromptLoader` cannot test the language override through a real catalog entry: no catalog prompt is `Audience::Human` yet and
  `PromptSpec::new` is `pub(crate)` in the core. The ordering is tested through the private `read_first`; the first human text
  of a later item exercises it end to end.

## Suggestions for `AGENTS.md` (not changed here)

- Under "Rust": a line that `cargo doc -p <crate>` and `cargo build -p <crate>` must work alone — features a crate needs in its
  library (`tokio` `macros`) belong in its normal dependencies, not only in the test build (the workspace gate unified them and
  hid the error until `cargo doc`).
- Under "Tests": tests that hold a `flock` and tests that spawn processes must not be asserted against each other in one
  binary without a bounded retry (see the TDD-8 note above).

I had no `agentbus` tools, so nothing was written to the repository notebook; the two notes above are the notebook entries.
