# Investigation — Rust item 8.4 — `inv-astra`

Continued the stopped work of [#100](https://github.com/mateusz-szczyrzyca/team-agents/pull/100) on `agents/inv-astra/20261114-094819`. The inherited Rust gate passed. Three reader defects are fixed; item `8.4` remains incomplete. The human authorized the session migration, a larger item and isolated history permission tests. Session recording now passes its focused checks in the worktree; one additional frozen daemon test requires permission before the GREEN implementation can be committed. The first-pass findings below remain as the investigation record; the continuation section gives their current status.

- $\color{red}{\textbf{RED}}$: [`20b096ea`](https://github.com/mateusz-szczyrzyca/team-agents/commit/20b096eaf8cdd962525231e647d343899a9f1be1) adds three tests; each failed because the reader accepted a row that SQLite rejected.
- $\color{green}{\textbf{GREEN}}$: [`4963b7e2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4963b7e2f219cd59ccae50e91dd91e9dfbd2689b) validates unused reason columns, nonrandom selection metadata and team-plan roles; simplifies reason decoding and adds the README history reference.
- Compatibility: no schema, protocol, CLI or valid stored-record change. Only rows already forbidden by SQL constraints are newly refused by the reading API. No existing test, fixture or suppression changed.

## Tests

New tests in [`crates/agent-runtime/tests/history/invariants.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4963b7e2f219cd59ccae50e91dd91e9dfbd2689b/crates/agent-runtime/tests/history/invariants.rs) prove both SQLite enforcement and independent reading after constraints are bypassed; each first proves the valid neighbouring case.
| Behaviour | Test |
|---|---|
| Every run reason rejects columns belonging to another variant | `every_run_reason_rejects_fields_of_other_variants_in_sql_and_when_read` |
| Nonrandom choices cannot carry draw candidates | `each_nonrandom_selection_rejects_draw_candidates_in_sql_and_when_read` |
| Team plans exclude the reviewer role | `reviewer_is_a_run_role_but_not_a_team_plan_role_in_sql_or_when_read` |

## Checks

- `make rust`: baseline `2,287` passing test executions; final code `2,290`, including `51/51` history database tests; `0` failed, `0` ignored. Formatting, clippy and documentation passed. `git diff --check` passed. Go and Bats suites were not run, as instructed; `make rust` replaced the otherwise mandatory `make gate`.
- `git grep -n 'CREATE TABLE' -- crates` finds one module. An isolated daemon with a fake OpenCode CLI completed a run; SQLite returned `(opencode, NULL)` for `SELECT cli, session FROM runs`, and only `(1, message, STATUS: DONE)` for its steps. `PRAGMA integrity_check` returned `ok`. Session marker `ses_investigation_session_84` existed in state and was absent from database bytes after shutdown. No live home or real model CLI was used. Serena was unavailable; symbol discovery used `rg`.

## Not done

- **P1 — sessions learned after launch:** [`crates/agent/src/daemon/owner/history.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4963b7e2f219cd59ccae50e91dd91e9dfbd2689b/crates/agent/src/daemon/owner/history.rs#L91) stores only the prelaunch session; the end record has none. [`crates/agent-runtime/src/cli/opencode.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4963b7e2f219cd59ccae50e91dd91e9dfbd2689b/crates/agent-runtime/src/cli/opencode.rs#L392) consumes the returned ID without an activity event. Codex/Gemini retain their IDs in informational steps, but their structured session field is also empty. Proposed fix: append-only `run_sessions(run, n, at, session)` with a foreign key, typed reading, and a forward migration retaining version `1` data; record each observed change before run end. This needs explicit permission to update the frozen [`docs/history.schema.sql`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4963b7e2f219cd59ccae50e91dd91e9dfbd2689b/docs/history.schema.sql) and table inventory in [`crates/agent-runtime/tests/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4963b7e2f219cd59ccae50e91dd91e9dfbd2689b/crates/agent-runtime/tests/history/schema.rs#L31); the repository forbids changing existing test expectations without permission.
- **P1 — history can delay work:** [`crates/agent-runtime/src/run.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4963b7e2f219cd59ccae50e91dd91e9dfbd2689b/crates/agent-runtime/src/run.rs#L335) awaits the history-only Git head lookup before spawning the CLI and awaits all change collection before reporting completion. [`crates/agent-runtime/src/git/changes.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4963b7e2f219cd59ccae50e91dd91e9dfbd2689b/crates/agent-runtime/src/git/changes.rs#L148) uses `git add -A` on a scratch index, which can execute configured clean filters. These are outside the writer queue; a slow Git operation delays a run, and capture failures only warn without incrementing lost records.
- **P2 — other failure paths:** [`crates/agent/src/daemon/history.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4963b7e2f219cd59ccae50e91dd91e9dfbd2689b/crates/agent/src/daemon/history.rs#L54) returns `History::disabled()` after opener-task or writer-thread startup failure, hiding enabled-history losses.
- **P2 — missing failure proofs:** daemon-level full-queue, read-only-home and actual write-failure proofs are still absent; library tests alone do not meet this item’s stronger end-to-end rule. A wholly read-only home also prevents state updates, so its intended failure isolation needs clarification.
- **Size and handoff:** PR-1 count against PR target [`150e2203`](https://github.com/mateusz-szczyrzyca/team-agents/commit/150e2203fe0ebd458c7f6f67c5676ab736378b5d) is `4,971` before this report (`5,000` including it), excluding tests, lockfile and the schema snapshot. The inherited diff includes an unrelated `3`-line Bash change in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4963b7e2f219cd59ccae50e91dd91e9dfbd2689b/bin/agent); it was not altered or tested here. Split the remaining work within item `8.4` before implementation; do not declare the history checkpoint complete. No PR was opened; no later item was started.

## Approved continuation

The human approved updating the frozen schema snapshot and table inventory in a dedicated `TEST-FIX:` commit, raising this item's size limit instead of splitting it, and isolating permission failures to history storage while state remains writable. Before further implementation, this continuation sets an item-specific ceiling of `7,500` counted lines: the existing `5,000` lines plus room for session migration/capture, bounded capture failures and daemon failure handling. Tests remain excluded; the extra room is a ceiling, not a target. The history checkpoint remains pending until the remaining checks pass.


## Session continuation and current blocker

- $\color{red}{\textbf{RED}}$: [`4ffb711c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4ffb711cf66606b7ea0a513f92a0647f2928a937) adds a merged fake OpenCode round, a live-parser contract for all four CLIs, seeded session round trips, invalid-row checks and a preserved version `1` fixture. Each new test failed on the intended missing behavior before implementation.
- Authorized `TEST-FIX:`: [`88dc7138`](https://github.com/mateusz-szczyrzyca/team-agents/commit/88dc71381c20c0b7e6320916c00c375df15a5dfd) updates only the schema snapshot and table inventory for append-only session observations.
- The pending implementation adds migration `2`, `Reader::sessions`, SQL validation and idempotency, and records changed CLI sessions during output processing. It leaves the original prelaunch session intact and retains version `1` reading and data during upgrade. This implementation remains uncommitted in this worktree pending the question below; it is not represented as a passing GREEN commit.
- Focused checks with the pending implementation: `tools/test-lock.sh sessions-verification cargo test --locked -p agent-runtime --test history --test cli_sessions` passes `55/55` history tests and `1/1` parser contract test. The new daemon session test passes as part of `make rust`.
- `make rust` with the pending implementation passes formatting and clippy; the daemon suite passes `655/656`. Its sole failure is the inherited `a_history_of_a_newer_version_is_left_as_it_is_and_the_round_goes_on`, whose fixture assumes version `2` is unsupported. No existing daemon test was changed. The gate stops there; the complete workspace test and documentation stages have not passed for this continuation.
- The history-only Git delays, hidden startup losses and remaining daemon failure proofs listed above are still open. The read-only-home ambiguity is resolved: only history storage must be unwritable, with state and task storage writable. No later plan item was started.
- New session handling introduces no lint suppression or dependency. The item-specific planned ceiling remains `7,500` counted lines under the human's approval to raise the limit. No Go or Bats suite, real model CLI, hardware device or live installation was used.

## Questions — round 2

The external question file is `questions-2.md` for task `20261114-094819-inv-astra`. Its decision is reproduced here so it is available from the pushed branch.

## 1. May the daemon's frozen newer-schema fixture advance with the supported schema?
The approved session migration makes history schema version `2` supported. One additional existing test, `a_history_of_a_newer_version_is_left_as_it_is_and_the_round_goes_on` in [`crates/agent/src/daemon/owner/rounds/tests/history_records.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/88dc71381c20c0b7e6320916c00c375df15a5dfd/crates/agent/src/daemon/owner/rounds/tests/history_records.rs#L798), hard-codes version `2` as unsupported. With the session implementation, `make rust` passes formatting and clippy, then reports `655` daemon tests passed and this one failure: `HistoryHealth { state: Writing, lost: 23 }`. The new merged OpenCode session test passes; all `55` runtime history tests and the four-CLI parser contract test pass. Your first approval named only the schema snapshot and table inventory, so it does not authorize changing this third frozen test.
a) Authorize a dedicated `TEST-FIX:` for this test's migration fixture and version-message expectation — construct a database one version above the latest supported migration and keep its byte-preservation, merged-round and unavailable-status assertions.
b) Keep this test frozen — prevents completing the approved forward migration without violating the test contract.

**Recommended:** a) — preserves the test's intended newer-schema guarantee as the supported version advances; changes no production behavior. (investigator)

**Verdict:**
