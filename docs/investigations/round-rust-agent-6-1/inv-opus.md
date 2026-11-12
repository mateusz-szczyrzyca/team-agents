# Round rust-agent/6.1 — inv-opus

Investigator `inv-opus`, branch `agents/inv-opus/20261112-202827`, base
[`c5396bc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c5396bc9893e12d15b60fa54b3a0eb7977277ef9) — the head
of [#93](https://github.com/mateusz-szczyrzyca/team-agents/pull/93), which already implements item 6.1 (by
`inv-claude-sonnet`). I reviewed that implementation on my own against the item and the Rust rules, ran the item's four
checks on the real binary, and fixed, test first, the defects that are small and clearly in the item's scope. The rest are
listed below with a proposed fix.

**Size first:** with my fixes the branch counts `5,039` lines outside tests against `main` (PR-1 command), over the `5,000`
of this plan; #93 alone counts `4,990`. My commits add `61` counted lines. See [Size](#size) for the options. This report
itself should go into a PR description, not into the diff (as #93 did with its own report).

## Summary

| # | Finding | Severity | State |
|---|---|---|---|
| 1 | A run that ends by itself leaves its background processes alive (no process-group sweep) | major | fixed |
| 2 | A subscriber's request read in parts is lost when an event arrives in between (`read_line` not cancel safe, CON-6) | major | fixed |
| 3 | More than `16 MiB` of output unread at a run's end hides its `STATUS:` line: a `DONE` run fails | minor | fixed |
| 4 | The crate doc of `agent` still says it answers only `--version` / `--help` | minor | fixed |
| 5 | Daemon killed outside systemd (SIGKILL, crash): its runs keep going, and recovery starts a second CLI on the same session | major (outside systemd) | open |
| 6 | The hang strike count is in memory only: a restart of the daemon starts it again | minor | open |
| 7 | "Progress" resets the strike count: a command that hangs each time after one step is resumed for ever | design risk | open |
| 8 | A missing or wrong prompts directory is reported as "22 text files cannot be used", naming neither the directory nor a file | minor | open |
| 9 | Small rule findings: notification text built in `.rs`, `bool` parameters, a user unit ordered after a system target, "1 minutes" | minor | open |

## Checks of the item

Run on `target/debug/agent` built from this branch, each in its own `AGENTS_HOME=$(mktemp -d)`:

- **Check 1 — start, status, a second daemon:** passes, but **not as written**. With a bare `mktemp -d` the daemon exits `1`
  ("there is no configuration file …/agents.toml"); with `agents.toml` but no `--prompts-dir` it exits `1` ("cannot load the
  texts of the harness: 22 text files cannot be used", see finding 8). With `cp config/agents.example.toml` and
  `--prompts-dir "$PWD/prompts"` (the trial in `docs/architecture.md`): `agent.sock` is `srw-------`, `agent status` prints
  `no teams` (exit `0`), a second daemon exits `1` with "the agent daemon is already running: …/daemon.lock is locked by it",
  SIGTERM ends the first with exit `0` and removes the socket. #93's report records this deviation (its decision 3).
- **Check 2 — SIGTERM ends the daemon and its runs:** passes for a run that is going (`tests/daemon_process.rs`, and by
  hand). It did **not** hold for a run that had already ended by itself and left a child: that child was never stopped
  (finding 1, fixed). It does not hold when the daemon dies without SIGTERM outside systemd (finding 5).
- **Check 3 — watchdog:** passes by hand with `hang_minutes = 1`, `hang_strikes = 2` and the `hang` scenario of the stand-in
  CLI: the first hang is killed and resumed with `--resume <session>` and the text "A HUNG OPERATION WAS KILLED"; after the second the
  task is `failed` after `125 s` and the notification "claude-1 is hung … 2 times in a row without progress" is written.
- **Check 4 — quota:** passes by hand with the `quota` scenario ("resets in 3 hours"): the task is `paused`, the run's end is
  `quota_exhausted` with `resets_at` `+3 h`, and the notification says it goes on at the reset time plus the `1`-minute margin
  (`22:40:58Z` for a run that ended at `19:39:58Z`). The wake itself is covered by the daemon tests on a manual clock
  (`an_exhausted_quota_pauses_the_task_until_the_reset_and_it_goes_on_by_itself`, `a_pause_survives_a_restart_and_ends_on_time`).

The required tests exist: daemon started and stopped and a second one refused, `agent status` over the socket, runs of a
fake CLI that end done / with questions / hung / out of quota, recovery after a restart.

## Fixed (test first)

### 1. A run that ends by itself leaves its processes alive — major

`RunEngine::run` only waited for a CLI that exited by itself; it called `RunningProcess::stop` only for a cancel and for the
watchdog. `RunningProcess::wait` says itself that "processes the leader started may outlive it: `stop` ends them", and
`Drop` kills nothing once the leader was reaped. So whatever a run left in the background — a test server, a `cargo watch`, an
MCP server the CLI did not stop — ran on with no owner: not killed by the next run, not by the daemon's SIGTERM (it is no
run of the daemon any more), only by systemd's `KillMode=mixed` when the unit stops. That breaks check 2 for every run that
ended before the SIGTERM.

- $\color{red}{\textbf{RED}}$ [`5f60e73`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5f60e7371205843203a3fe9f24d78979c52d9968):
  scenario `done-child` of the stand-in CLI (leaves `sleep 600` behind, ends `STATUS: DONE`);
  `a_run_that_ends_by_itself_leaves_no_process_it_started` timed out with the child alive after `20 s`.
- $\color{green}{\textbf{GREEN}}$ [`350f507`](https://github.com/mateusz-szczyrzyca/team-agents/commit/350f507ec7c68aded510f631040b7823f94550e4):
  after the exit, `process.stop(stop_grace)` — the leader is already reaped, so this is only the last `SIGKILL` to the group
  (no grace wait). `2` lines in
  [`crates/agent-runtime/src/run.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/53772296b39beee9c90b45e69bc168529821526d/crates/agent-runtime/src/run.rs).

### 2. A subscriber's request split by an event is lost — major (CON-6)

The connection task waits for the next request line and for the next event in one `select!`. `read_line` collected the line in
a local `Vec` and consumed the bytes from the `BufReader` as it went; when an event won the `select!` while a line was half
read, the bytes went with the dropped future and the rest was read as a line of its own. A request up to the protocol's
`2 MiB` spans many reads of the `8 KiB` buffer, so any subscriber that also sends requests (the dashboard of 9.1) gets
"not a valid request" for a request it sent whole.

- $\color{red}{\textbf{RED}}$ [`445a51b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/445a51b80532d0b1d9da9256a21a04c819ffa1da):
  `lines::tests::a_line_read_in_parts_survives_a_read_that_is_dropped_before_its_end` (paused clock: deterministic) got
  `Some("1}")` for `{"half":1}`;
  `daemon::tests::a_request_of_a_subscriber_that_arrives_in_parts_around_an_event_is_answered` got a response with
  `RequestId(0)` and `Invalid` instead of request `2`'s teams.
- $\color{green}{\textbf{GREEN}}$ [`4674fbf`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4674fbfa7944baaf65a93ce51b598fd54bb5975e):
  `read_line_into(reader, &mut partial, max)` keeps the unfinished line in a buffer the caller owns (the only `.await` is
  `fill_buf`, after which the buffer and the reader change together); `read_line` is `read_line_into` with a buffer of its own,
  so its signature and tests are unchanged; the server keeps one buffer per connection.
  [`crates/agent/src/lines.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/53772296b39beee9c90b45e69bc168529821526d/crates/agent/src/lines.rs),
  [`crates/agent/src/daemon/server.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/53772296b39beee9c90b45e69bc168529821526d/crates/agent/src/daemon/server.rs).

### 3. A burst of output hides the status — minor

A look at the output reads at most `MAX_READ_PER_POLL` (`16 MiB`), so that a CLI printing without pause cannot starve the
watchdog. `Follow::drain` read once more with the same bound after the process ended: what was left beyond it, the final
`result` line among it, was never read, and a run that reported `STATUS: DONE` was recorded as `NoStatus { exit_code: Some(0) }`,
so its task failed. Rare (more than `16 MiB` written since the last look, which is `1 s` in production), but silent.

- $\color{red}{\textbf{RED}}$ [`d5bbc7d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d5bbc7d6fa3a3db3b12276b76dff977533536992):
  scenario `flood` (three `6 MB` lines, then `done`) with a look every `10 min`;
  `the_status_after_more_output_than_one_look_reads_is_still_seen` got `NoStatus { exit_code: Some(0) }`.
- $\color{green}{\textbf{GREEN}}$ [`b223838`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b223838df4c295a608ce89d77f6172cdb22c8a12):
  `drain` reads part by part up to the length the file had when the process was over (a file something outside the group
  still writes cannot keep it reading); a read without progress also ends the loop, for a file cut shorter meanwhile. No
  test reaches that last guard: nothing in a run cuts its own output.
  [`crates/agent-runtime/src/run/output.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/53772296b39beee9c90b45e69bc168529821526d/crates/agent-runtime/src/run/output.rs).

### 4. Stale crate documentation — minor (DOC-3)

[`5377229`](https://github.com/mateusz-szczyrzyca/team-agents/commit/53772296b39beee9c90b45e69bc168529821526d):
[`crates/agent/src/main.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/53772296b39beee9c90b45e69bc168529821526d/crates/agent/src/main.rs)
said "this version answers `--version` and `--help`". `Test-First: not possible — documentation only.`

## Open (not fixed here)

### 5. A daemon that dies without SIGTERM, outside systemd, leaves its runs going — major outside systemd

Every CLI runs in a process group of its own, so a daemon that is killed (`SIGKILL`, OOM, a panic that aborts) takes none of
them with it. Reproduced on the real binary with the `hang` scenario: after `kill -KILL` of the daemon the run's group
(`pgid 2867876`) was alive; the restarted daemon recovered the task and started a second CLI with `--resume` on the **same
session in the same worktree** next to the first, with the text "a command that was running when it happened did not finish"
— untrue; a SIGTERM to the new daemon left the orphan running too.

Under the shipped unit this does not happen: I checked with a transient user unit (`systemd-run --user -p KillMode=mixed`,
main process `SIGKILL`ed) that systemd kills the rest of the cgroup when the main process dies. So the gap is the daemon run by
hand, which is how the item's check and the trial of `docs/architecture.md` start it. Options:

- a) Record each run's process group with its start time (`/proc/<pid>/stat` field 22, against PID reuse) in its run directory
  when it starts, and at recovery kill a group that is still alive before the task is queued. New on-disk file (docs/state.md).
- b) Document that crash safety needs the systemd unit, and say so in the trial.

Recommended: a), in 7.1 or a follow-up PR; it is more code than this PR has room for (see [Size](#size)).

### 6. The strike count does not survive a restart — minor

`Owner::hangs` is a `HashMap` in memory; the Bash version kept `HANGS` in `meta.env`. A daemon restarted between two hangs
(`Restart=on-failure`, a manual restart) gives the task its full strikes again. Option: on recovery, start the count from the
trailing runs whose end is `hung` (counting them as hangs without progress, the conservative reading), or keep the count in
the member's state.

### 7. Progress resets the strike count — design risk, same as the Bash version

`hung()` starts the count at `1` whenever the hung run did anything but start (`made_progress`: any event other than `info` /
`err`). A resumed agent that makes one tool call (progress) and then hangs again — on the same command despite the resume
text's "do not run it again in the same form", or on another — is resumed again: never marked hung, one run per
`hang_minutes` for ever. This is what
`bin/agent` did too (`activity_progress`), and 3.1's `after_hang` leaves the reset to the caller, so it is not a defect of
this PR; the human may want a cap of hangs in a row regardless of progress.

### 8. A prompts directory that cannot be used is reported without a path — minor (ERR-6)

`agent daemon` with `AGENTS_HOME` but without `--prompts-dir` (default `$AGENTS_HOME/prompts`, which a trial does not have)
prints only `agent: cannot load the texts of the harness: 22 text files cannot be used`: neither the directory it looked in nor
one of the files. `HarnessPromptsError` and `PromptLoadError` hold every problem, but their `Display` gives only the count and
nothing logs the list; `docs/architecture.md` says "every problem reported at once". Fix: put the directory into
`DaemonError::HarnessPrompts` / `Prompts`, and log each problem at `error` before returning.

### 9. Small rule findings — minor

- **Notification text in `.rs`** (ground rule "no prompt text in `.rs` files"): `const SIGNAL: &str = "signal"` in
  `daemon/notice.rs` and `format!("the task of the run failed: {}", …)` in `daemon/owner.rs` both end up in a notification for
  the human.
- **TYP-4:** `bool` parameters `Owner::hung(…, made_progress: bool)` and `start::prompt_kind(…, has_session: bool, …)`.
- **Ambiguous mark of an interrupted run:** a run the daemon interrupted and a run a signal killed (OOM) are both
  `NoStatus { exit_code: None }`; `prompt_kind` gives both the "THE HARNESS RESTARTED" text. Today only the first is
  resumed; when 7.2 resumes a failed task, the second gets the wrong text.
- **`config/systemd/agent-daemon.service`:** `After=network-online.target` orders a user unit after a target of the system
  manager, which the user manager does not have; it has no effect.
- **`prompts/harness/run/notice-hung.md`:** "no activity for 1 minutes".
- **The start writes into the home before it checks the configuration:** a daemon that refuses to start
  for want of `agents.toml` has already made `daemon.lock` and `tasks/` in the home (harmless, but surprising in a check).

## Size

The PR-1 command (`<base>` = `origin/main`
[`e8210e8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e8210e836327a382442fb8b8b3ac39d1afcf040c)):

| Tree | Lines outside tests |
|---|---|
| #93 head [`c5396bc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c5396bc9893e12d15b60fa54b3a0eb7977277ef9) | `4,990` |
| this branch [`5377229`](https://github.com/mateusz-szczyrzyca/team-agents/commit/53772296b39beee9c90b45e69bc168529821526d) | `5,039` |

Over the limit by `39`. PR-2 allows a marginal overshoot only when a smaller PR could not build or pass on its own, which is
not the case here. Options for the merger:

- a) Merge #93 as it is and my seven commits as a follow-up PR (they apply on #93's head, each RED/GREEN pair stands alone).
- b) Take the fixes into #93 and win back the `39` lines elsewhere, e.g. in the `+69` lines of `docs/architecture.md`.

## Gate

`make gate` in this worktree runs through the harness's `agent gate`, whose scope for a diff under `crates/` is the Rust gate
(`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
`RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps`): **PASS** at
[`b223838`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b223838df4c295a608ce89d77f6172cdb22c8a12), `1,211` tests
passed, `0` failed, `0` ignored. The last commit changes only a doc comment; `cargo fmt --check` and
`RUSTDOCFLAGS=-Dwarnings cargo doc -p agent` pass on it. No existing test changed; two scenarios were added to the stand-in
CLI ([`crates/agent-runtime/tests/fixtures/run/fake-claude.sh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/53772296b39beee9c90b45e69bc168529821526d/crates/agent-runtime/tests/fixtures/run/fake-claude.sh):
`done-child`, `flood`). One `#[cfg_attr(not(test), expect(dead_code, …))]` lived only in the RED commit
[`445a51b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/445a51b80532d0b1d9da9256a21a04c819ffa1da) and is gone in GREEN.

## Also checked, nothing found

- The `agentbus serve` flags the daemon passes (`--agent --task --task-dir --repo --worktree --team --tier --role --home --dir`)
  all exist in `cmdServe` of `agentbus/main.go`; `--tasks-dir` defaults to `<dir>/../tasks`, which is the home's `tasks/`.
- Socket: bound in a `0700` staging directory, `chmod 0600`, renamed into place; a stale socket of a dead daemon is replaced.
- Shutdown: the server stops accepting, the owner cancels and joins every run (each stopped as a whole group, `SIGTERM` then
  `SIGKILL` after `10 s`), the notifier drains, the socket is removed; `TimeoutStopSec=60` covers the grace.
- The mechanical sweep of §2.6 over the added production lines: the hits are specified fallbacks or recorded in #93
  (`map_err(|_closed| gone())`, `Err(_elapsed)`, `Box<dyn Error>` from `tracing_subscriber`).

## Suggestions for `AGENTS.md` and the notebook

- `make gate` in a task's worktree runs `agent gate`, whose scope decides the steps (only the Rust gate for a diff under
  `crates/`); `AGENTS.md` describes it as all of `make lint test`. Worth one sentence.
- Every `select!` over a read of a stream needs a cancel-safe read (`read_line_into`), and every run needs its group swept
  after its leader exits; both are easy to miss in later items (7.x, 9.1).
