# Item 6.1 — the daemon, its socket and the run engine

Investigator `inv-claude-sonnet`, branch `agents/inv-claude-sonnet/20261112-183451`, base
[`e8210e8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e8210e836327a382442fb8b8b3ac39d1afcf040c). Status: done,
pushed, no pull request opened (the merger integrates). All paths below are on the head
[`6cc833b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6cc833ba1afe0514b09bc8841956eced40d3ea16).

## What and why

`agent daemon` — the one owner of the state and of every model run — with its Unix socket, the run engine behind it, the
watchdog, the pause and wake of provider limits, recovery on start, `agent status` as the first client and a systemd user unit.

| Where | What |
|---|---|
| [`crates/agent-runtime/src/run.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cc833ba1afe0514b09bc8841956eced40d3ea16/crates/agent-runtime/src/run.rs), `run/{compose,launch,output}.rs` | **the run engine**: `RunEngine::run` builds a run through its adapter, lays down its files, keeps `worktree_excludes` (`.serena/`) out of git, starts the CLI under the supervisor, feeds its stdout to the parser and the activity log, kills a silent run's whole group, classifies the end. Touches no state |
| `crates/agent-runtime/src/{settings,text_file,clock,harness_prompts}.rs` | `agents.toml` and `secrets.toml` from the home; one bounded reader; `SystemClock`/`SharedClock`; the loader of the second prompt catalog |
| `crates/agent-core/src/prompt/harness.rs`, `prompts/harness/run/*.md` | `HarnessPrompt`: the run prompts (context, 5 roles, brief, item, checkpoint, 3 resume texts, prompt file) and the 7 notifications |
| [`crates/agent/src/daemon/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/6cc833ba1afe0514b09bc8841956eced40d3ea16/crates/agent/src/daemon) | `owner` (the state, one task, CON-3), `server` (the socket), `notifier`, `settle` (what a run's end does), `recovery`, `notice` |
| `crates/agent/src/{client,lines,environment,chain}.rs`, `commands/{status,daemon}.rs`, `main.rs` | the socket client, bounded line framing, `AGENTS_HOME`, `agent status [team-… \| task-…]`, `agent daemon [--prompts-dir DIR]` |
| [`config/systemd/agent-daemon.service`](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cc833ba1afe0514b09bc8841956eced40d3ea16/config/systemd/agent-daemon.service) | `KillMode=mixed`, `TimeoutStopSec=60`, `Restart=on-failure` |
| `docs/architecture.md`, `docs/state.md`, `docs/configuration.md` | the daemon section (tasks, start and a trial, what a run's end does, watchdog, pauses, recovery, socket, home), what the socket serves, when settings are read |

Files outside the item's list, and why: everything under `crates/agent-runtime/` and `crates/agent-core/` (the engine is in the
runtime: it is effects, and its tests need no daemon; the catalog entries are in the core), `prompts/harness/run/` beyond "the
run prompts" (the notifications are texts for the human and live there by the architecture's rule), `crates/agent/src/{lines,
environment,chain}.rs`, `commands/daemon.rs` (parts of `agent daemon` and of the client), `Cargo.toml`/`Cargo.lock`, the three docs
(documentation rule). Existing production files changed, additively: `agent-runtime` `store.rs` (the home's new paths), `git.rs`
(`Git::info_exclude_file`), `notify.rs` (`Notification::title/body`), `lib.rs`; `agent-core` `prompt.rs` (one `mod`/`use`);
`agent` `main.rs` (production part only). The README describes the Bash harness and is untouched; the Rust agent's docs are in `docs/`.

## Behaviour

A task whose status is `queued` is started; a task's run ends one of these ways (`settle.rs`, table in `docs/architecture.md`):

| Run ended | Task goes to |
|---|---|
| `STATUS: DONE` | `done` (solo, review) or `waiting` (coordinator, member, worker) |
| `STATUS: QUESTIONS` | `questions` after the file `tasks/<task>/questions-<round>.md` parses; the round is recorded, the human notified. A missing or unreadable file fails the task |
| `STATUS: FAILED`, no status line | `failed`, the human notified |
| quota, rate limit | `paused`; woken by the clock at the CLI's reset time (+1 min) or after the configured minutes; notified |
| watchdog | `queued`, resumed from its session; after `hang_strikes` hangs without progress `failed` and notified |
| daemon shutdown | `queued`; the run is recorded as ended with no status and no exit code; the next start resumes it |

Recovery on start does the same for a run a crash left going. A pause keeps no field in the state: its wake-up is worked
out again from the last run's end. The prompt of a resumed run is the text for what interrupted it (hang, restart, limit).

## Decisions that need the merger's or the human's eye

1. **A second prompt catalog.** The item puts "the run prompts under `prompts/harness/run/`", and `docs/architecture.md` says
   the harness's texts become variants of `PromptId`. But `prompt::tests::the_rule_files_of_the_layout_are_in_the_catalog` pins
   `PromptId::ALL` to exactly the five rule files, and rule 1 forbids changing it. I did not ask (nobody answers live; asking
   would have ended the task) and added `HarnessPrompt` (own enum, own loader) beside it. If the human allows changing that
   test, the two catalogs can merge (`agent allow`); it is a mechanical change.
2. **`AGENTS_HOME` is required; no default.** A default such as `~/agents` would make a trial touch the live Bash installation.
   The cutover (10.1) decides.
3. **The check "`AGENTS_HOME=$(mktemp -d)`; `agent daemon &` starts" cannot pass as written:** a daemon needs `agents.toml`
   and the prompt files, and refuses a home without them naming the file. The trial that passes (and that
   `tests/daemon_process.rs` runs) is in `docs/architecture.md`: `cp config/agents.example.toml "$AGENTS_HOME/agents.toml"` and
   `agent daemon --prompts-dir "$PWD/prompts"`.
4. **The worktree is `worktrees/<repo>/<task>` in the home** (`Home::worktree_dir`). Creating it and queueing the task is `agent
   run` (7.1); until then tests and trials write the state and the worktree themselves.
5. **`DONE` of a coordinator, member or worker is `waiting`, not `done`:** `done` is final, and a member is given more items.
   7.1 may refine this (the mapping is one function, `settle::settle`).
6. **Task statuses are not extended:** "the task is marked hung" is `failed` with the last run's end `hung` and a notification.
7. **Storage failure ends the daemon.** A state that cannot be saved is not carried on with; `Restart=on-failure` restarts it
   and recovery repairs what was half done.
8. **Time zone.** A reset time without a zone in a provider's message is read in the machine's zone, read once at the start of
   the process (`time`'s `local-offset`, needs one thread); UTC with a warning when the system cannot say.
9. **The engine's file names** inside a run's directory (`out.jsonl`, `err.log`, `prompt.md`) and the `agentbus serve` flags
   (`--agent --task --task-dir --repo --worktree --team --tier --role --home --dir`) are my reading of the Go server's flags;
   7.1 creates the bus team they point at.

## Checks of the item

Run by hand on the real binary (`target/debug/agent`) in temporary homes, and by the tests named in the right column.

| Check | Result | Tests |
|---|---|---|
| `agent daemon &` starts, `agent status` answers, a second `agent daemon` exits with "already running" | `no teams`; `agent: the agent daemon is already running: …`, exit 1 | `daemon_process::the_daemon_starts_status_answers_and_a_second_daemon_is_refused` |
| SIGTERM ends the daemon and its runs, no process of theirs left | exit 0, socket removed, the child of the run gone (`/proc`), the run recorded as interrupted, the task `queued` | `daemon_process::sigterm_ends_the_daemon_and_its_runs_…`, `daemon::tests::a_shutdown_stops_the_runs_whole_…` |
| a fake CLI silent past the (shortened) limit is killed and resumed; after the strikes the task is hung | with `hang_minutes = 1, hang_strikes = 2` the real binary killed the run at 60 s, started the resumed run, killed it at 120 s, marked the task `failed` and wrote the notification; no `sleep` of the fake was left | `daemon::tests::a_hung_run_is_resumed_in_its_session`, `…after_the_configured_strikes_…`, `run_engine::a_silent_run_is_killed_by_the_watchdog_…` |
| a fake CLI reporting an exhausted quota pauses its task until the reset time it gave | state `quota_exhausted`, `resets_at` = 3 h after the run, task `paused`, notification "paused until" 3 h + 1 min | `daemon::tests::an_exhausted_quota_pauses_the_task_until_the_reset_and_it_goes_on_by_itself`, `run_engine::an_exhausted_quota_…` |

The item's other tests: a daemon started and stopped and a second refused (`daemon::tests`, `daemon_process`), `agent status`
over the socket (`daemon::tests::status_over_the_socket_…`, `commands::status::tests`, `command_tests`), runs of a fake CLI that
ends done, with questions, hung and out of quota (`daemon::tests`, `run_engine`), recovery after a restart
(`a_run_that_was_going_when_the_daemon_died_…`, `a_pause_survives_a_restart_…`, `recovery::tests`).

## Tests

`180` new tests, test first (RED commit with failing output in its message, then GREEN); none of the existing tests was
changed (the six tests of `main.rs` pass untouched).

| Commits | |
|---|---|
| [`a7c0c97`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a7c0c97a355503fd61044bc4f57e9fd8cd5121ad) RED → [`d35761e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d35761e6e562b042f7ff9374f028a70ed5aa251a) GREEN | home layout, settings (`4 + 10` failing) |
| [`06ad985`](https://github.com/mateusz-szczyrzyca/team-agents/commit/06ad985b818d9d2e0f0c46381814980730f135d4) → [`1f5dab6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1f5dab69edd370b138cba83969a64f4d80bd7f4c) | the second catalog and its loader (`9`; the catalog test is data: shown red by removing a variable) |
| [`ae1cc7d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ae1cc7d3431fef10d3e303667f28ef267bc12a09) → [`0ee72e0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0ee72e091d8d187dcf756d4b8d870238f499c0c8) | prompt composition (`8`) |
| [`e8152b3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e8152b3562a99d8b12b91aea7980d30ebd549e64) → [`7918c86`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7918c86ba0873ca29233f1d0a0f106d8133d0516) | the engine, `SystemClock`, `Git::info_exclude_file` (`1 + 3 + 20`) |
| [`158154d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/158154de5d71d9763418b5d7e9b84750a9e5e119) → [`2b2256d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2b2256d064b1b8145b7ae8cecea55dc8e7d1661d) | `settle`, `wake_time`, `recover`, notices (`20 + 3`) |
| [`5506a41`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5506a41e93cdcab2f51e8ea8eba29e9c1e9b67af) → [`90a29d8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/90a29d889595dfb86944ece83d5e5bea00d5e8a2) | framing, client, `agent status`, the command line (`36`) |
| [`e9dd4b7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e9dd4b7a8292ff622c642b979c6618327c7a0660) → [`28b812a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/28b812a53e63713f43838bd45c4909bcd3c0642f) | the daemon: owner, server, notifier (`28` in-process tests) |
| [`77ad216`](https://github.com/mateusz-szczyrzyca/team-agents/commit/77ad2162524c49cd993006e2499a2fdc7246358d) → [`72f2dcf`](https://github.com/mateusz-szczyrzyca/team-agents/commit/72f2dcfe1372695689fe939599341b483f75c744) | `agent daemon` as a process (`4`) |
| [`6705b81`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6705b810493f70d9874a1f12d9566349de1c4761) | the unit (its test failed before the file existed) |

Mutation checks (TDD-2), each broken and seen red: the watchdog's silence test, the socket mode, the exclude-file duplicate
check, the wake-up filter, the recovery of a running task, the prompt-file threshold, the version bits of the session UUID.

Two bugs the tests found in my first code: a clock set back made recovery end a run before it started (daemon refused to
start — now `at.max(started_at)` in recovery and in the owner, each pinned), and a test that reused one run directory read the
output of the previous run (the tests use a home each; a run ID names one directory).

The stand-in CLI is a committed script,
[`fake-claude.sh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cc833ba1afe0514b09bc8841956eced40d3ea16/crates/agent-runtime/tests/fixtures/run/fake-claude.sh),
whose scenario is a file in the worktree: no test writes an executable it then runs (the `ETXTBSY` flake below).

## Size

`4,990` changed lines outside tests by the PR-1 command (relaxed limit `5,000`, PR-2's split threshold about `4,000`), without
this report; `9,470` added / `7` deleted in `85` files in all. **I did not plan the split before coding; here it is, along the
commit order:**

- **Part 1/2, `1,900` lines — `agent-core`, `agent-runtime`, `prompts/`, `Cargo.*`:** commits `a7c0c97` … `7918c86`, then
  `dbe8884` (the bounded reader), `6cc833b` (two engine tests). Builds and passes alone: nothing in it needs the daemon.
- **Part 2/2, `3,090` lines — `crates/agent`, `docs/`, `config/`:** the rest, in order. It uses Part 1.

Moving the test builders and the stand-in socket to `crates/agent/src/tests/` (`#[path]`, same module names) is what brought the
count from `5,346` to `4,990`: they are test code, which PR-1 counts by path.

## Gate

`agent gate` at `6cc833b`: **PASS** (`make gate`: fmt, clippy `-D warnings`, tests, doc `-D warnings`, all `--locked`, and the Go,
bats and lint steps). The first run failed on `agent-runtime` `git::tests::a_revision_that_prints_no_commit_id_is_an_error`
with `Text file busy` (`ETXTBSY`) — the known flake of `scripted_git` that the report of 5.1 measured (about 5 % of runs, an
existing test I may not touch); the next run passed.

## Deviations

- **New dependencies: none.** Approved ones now used: `tokio-util` (`CancellationToken`), `tracing-subscriber` (`fmt`, `std`:
  the daemon's log on stderr). Features added: `rand` `std`/`std_rng`/`sys_rng` (`StdRng::try_from_rng(&mut SysRng)`, the OS
  entropy for IDs), `time` `local-offset`, `tokio` `net`/`signal` (and `test-util` in dev). `Cargo.lock` gains `tokio-util`,
  `futures-sink`, `tracing-subscriber`, `sharded-slab`, `thread_local`, `lazy_static`, `num_threads`.
- **`map_err(|_closed| gone())`** twice in `daemon/server.rs`: a closed channel or a dropped answer means the owner is gone,
  which `gone()` tells the client; the unsent message has no cause worth keeping (ERR-2, recorded).
- **`Box<dyn Error + Send + Sync>`** as the source of `DaemonCommandError::Logging`: it is what `tracing_subscriber`'s
  `try_init` returns (ERR-4; a foreign type kept as the source, not used to signal failure).
- **`Err(_elapsed)`** of `tokio::time::timeout` (client, twice): the elapsed time carries only the fact; the error names the wait.
- **No `#[expect]`, no `#[allow]`, no `unsafe`, no skipped test.** One `const … = NonZeroUsize::new(16).unwrap()` (TYP-10).
- `ComposeError::{Harness,Rules,Variable}` and several `RunError` variants (`Supervise`, `Output`, `Activity`, `Classify`,
  `Clock`, `AgentPath`, `PromptFile`) have no test of their own: after the loader's check at start they can only come from a
  mistake of the program, or from a failing disk or clock that a test cannot make without a test-only path (TDD-12).

## Not done (each is an item of the plan or outside this one)

- `agent run`, branches and worktrees, the team's life, `delegate` (7.1); answers and the controls `stop/pause/unpause/resume/say`
  (7.2); the gate (7.3); PRs and reviews (8.1); the dashboard (9.1). The daemon refuses these by name (`refused`).
- A resumed agent's **checkpoint** (read from the agentbus, which 7.1 creates): the template and the composition exist, the
  daemon passes none.
- opencode's `session export` for a resumed session whose result it did not print (the adapter reports such a run as it
  printed it).
- Notifications are not queued across a restart (a notification the daemon could not deliver before it stopped is lost).

## Out of scope (noticed, not touched)

- The `scripted_git` flake above. Suggested fix, as in 5.1: write the script to a temporary name, `sync` and rename it, or
  serialise those tests.
- `make gate` took `14–20 s` here; a full `cargo test --workspace` is `~15 s`.

## Suggestions for `AGENTS.md` and the notebook

- Notebook: the claude stream's `result` of a limit has `is_error: true` and the message in `result`, which `classify_outcome`
  reads as the final message of a run without a `STATUS:` line; `StateStore::open` and a process forked by another test:
  a lock released by a drop can be held a few milliseconds by a child between its fork and its exec, so tests that start a
  daemon seed `state.json` directly.
- `AGENTS.md`: the "Never modify existing tests" rule and a catalog test that pins its whole content stopped a design the
  architecture document describes (decision 1). A sentence on how a round gets such a pin relaxed (the human's `agent allow`)
  would save the detour.
