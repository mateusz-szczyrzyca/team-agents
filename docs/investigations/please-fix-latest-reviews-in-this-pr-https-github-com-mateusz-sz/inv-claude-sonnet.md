# Fixing the latest review of PR #95 — `inv-claude-sonnet`

Task `20261113-115715-inv-claude-sonnet`, branch `agents/inv-claude-sonnet/20261113-115715`, based on
[`32e4dba`](https://github.com/mateusz-szczyrzyca/team-agents/commit/32e4dbae13b73ca48f25b86d3ef8103bb28f8006), the
head of PR [#95](https://github.com/mateusz-szczyrzyca/team-agents/pull/95).

## The review

One review is left on the head: `inv-gemini-argon` (`Needs changes`), with one finding.

- `[major]` `[LINT-2]` `[TDD-8]` `Daemon::bare_owner` in `crates/agent/src/daemon/owner/fixture.rs` opens the
  `StateStore` once. After `daemon.stop()` a child that another test forked keeps the `flock` of `daemon.lock` for a few
  milliseconds, so `a_rescan_reads_the_open_files_whose_save_was_not_reported` failed the gate with `HomeLocked`.
  Fix asked for: retry on `StoreError::HomeLocked`, as `Daemon::start` and the store tests' `reopen` do.

The review's other passes (1–5) are all marked fixed by the reviewer. The comment of the script on the PR only says that
the investigation ended after five passes; it asks for nothing.

## What was done

Test first, two commits:

- $\color{red}{\textbf{RED}}$ `2fca0d1` — `daemon::owner::fixture::tests::a_bare_owner_waits_for_a_home_whose_lock_is_released_a_moment_later`
  holds the lock of a home in a `StateStore` and drops it `100 ms` after `bare_owner` started. It failed with the
  reviewer's panic: `called Result::unwrap() on an Err value: HomeLocked { path: ".../daemon.lock" }` at
  `fixture.rs:203`.
- $\color{green}{\textbf{GREEN}}$ `86b587c` — `Daemon::open_store` retries `StateStore::open` on `StoreError::HomeLocked` every
  `20 ms` until `PATIENCE`, then returns the error to the test; `bare_owner` uses it. Every other error fails at once.

Size: `+45 / −3` lines, all of them in test code (`fixture.rs` is `#[cfg(test)]`; the new module is `fixture/tests.rs`).
No existing test, stub or expected value was changed; no production code was touched; no suppression was added.

## Checks

- `cargo test -p agent --locked --bin agent a_bare_owner_waits`: fails before the fix, passes after.
- `make gate` on `86b587c`: `PASS` (the second run; see below for the first).

## A flaky test the gate showed — not fixed, outside this change

The first `make gate` on `86b587c` failed on another test:
`daemon::owner::teams::tests::an_item_whose_branch_could_not_be_started_is_given_out_when_the_branch_can_be_made`
(`timed out waiting for run 2 of <coordinator task> to be logged`). The rerun passed. What I measured, so the next
person need not repeat it:

- The test is on `main` (from item 7.1) and is not in the diff of PR #95; it does not use `bare_owner`.
- On the PR branch it failed once in `5` isolated runs, then `0/60`, and `0/4` full `agent` test runs on an idle
  machine; `0/60` on `origin/main`.
- Constrained to one CPU (`taskset -c 0`, `--test-threads=16`) the whole `agent` test binary failed once in `8` runs on
  the branch, in a different existing test: `daemon::tests::an_exhausted_quota_pauses_the_task_until_the_reset_and_it_goes_on_by_itself`
  (`assertion left == right failed: [] left: 0 right: 1` at `crates/agent/src/daemon/tests.rs:421`: the test reads the
  notifications as soon as the state says `Paused`, while the notifier writes the file asynchronously). `origin/main`:
  `0/14` the same way.
- The samples are too small to say whether the PR made either more likely. Both look like timing races in tests that
  rely on the order of an asynchronous step; changing them is forbidden to me (rule 1), and the first one, in the
  coordinator's wake-up after the member's `DONE`, may also be a lost wake-up in production code
  (`member_finished` posted while the coordinator's run was still ending `SLEEPING`). A reviewer who sees either fail
  should rerun the gate once before treating it as a regression, and a human or a later round should look at both.

## For the notebook / `AGENTS.md`

- A helper that opens the state store of a home right after a daemon of the same test stopped must retry on
  `HomeLocked`: the lock outlives the daemon by a fork-to-exec window of other tests. Three places do it now
  (`Daemon::start`, `Daemon::open_store`, `reopen` in the store tests).
