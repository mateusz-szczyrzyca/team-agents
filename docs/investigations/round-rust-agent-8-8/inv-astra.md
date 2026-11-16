# Item 8.8 — `inv-astra`

## Result

Partial implementation is committed on `agents/inv-astra/20261116-015620`. Implementation is paused at a third existing-test conflict: the history schema needs migration `7`, but an existing test explicitly rejects version `7`. This is a clarification checkpoint, not a completed implementation of item `8.8`.

## Implemented

- Claude's `error_max_turns` produces `StreamOutcome::TurnLimit`; execution failures and successful final messages keep their existing behavior.
- `RunEnd::TurnLimit` is classified separately, with provider quota taking precedence.
- Protocol version `3` rejects version `2` peers; historical fixtures remain unchanged.
- Configuration accepts `max_turns` (default `300`, nonzero) and `turn_limit_resumes` (default `3`, zero allowed).
- Claude's adapter accepts a configured budget for both new and resumed runs; adapters without a turn budget retain their commands.
- The pure resume policy counts consecutive turn-limit ends from persisted runs, resets after another end, bounds automatic resumes and refuses a missing session.

The daemon does not yet use the continuation policy or configured turn budget. Its interim settlement still fails a turn-limit task. The history reader understands the new end, but the database schema does not yet permit recording it. Do not merge or deploy this checkpoint as completed item `8.8`.

## Resolved clarifications

- Round `1`, option `a`: rename the existing Claude turn-limit regression test and change its outcome assertion, preserving its other assertions.
- Round `2`, option `a`: mechanically update protocol-version assertions, inputs and fixture references for version `3`; preserve previous fixtures and unrelated assertions.

## Blocking history contract

- [`crates/agent-runtime/src/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/631f1cf6a01234c6c02deb9f4efaa5a708de4d5f/crates/agent-runtime/src/history/schema.rs) contains migrations `1`–`6`, with closed sets for run reasons and endings. Its contract requires every schema change to use a new migration.
- [`crates/agent-runtime/tests/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/631f1cf6a01234c6c02deb9f4efaa5a708de4d5f/crates/agent-runtime/tests/history/schema.rs#L246-L274) treats version `7` as unsupported and requires rejection without modifying the database or creating a journal.
- [`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/631f1cf6a01234c6c02deb9f4efaa5a708de4d5f/AGENTS.md#L61-L65), Never rule `1`, says: “If the task requires behaviour that an existing test forbids, stop and ask the human in your questions file; wait for the answer.” The previous authorizations cover adapter and protocol tests, not this history assertion.

The task's third questions file requests a narrowly scoped version-boundary update. Its complete text follows.

# Questions — 20261116-015620-inv-astra

## 1. May item 8.8 advance the history test's unsupported schema version?
The authorizations from clarification rounds `1` and `2` remain in force. Recording the new `turn_limit` end and continuation reason requires a new history migration: the existing SQLite `CHECK` constraints reject those values, and shipped migrations must remain unchanged. The current schema is version `6`, so the next migration is version `7`. However, `a_database_of_a_newer_version_is_refused_and_left_byte_for_byte_as_it_was` in [`crates/agent-runtime/tests/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/631f1cf6a01234c6c02deb9f4efaa5a708de4d5f/crates/agent-runtime/tests/history/schema.rs#L246-L274) explicitly requires version `7` to be rejected. [`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/631f1cf6a01234c6c02deb9f4efaa5a708de4d5f/AGENTS.md), Never rule `1`, requires authorization before changing this existing test contract.

a) Authorize changing this test's unsupported-version input and matching assertion from `7` to `8`, plus equivalent history-version-only updates if another existing test pins the same boundary — preserve all byte-for-byte, typed-error and no-journal assertions; preserve shipped migrations and historical fixtures; add migration and recording tests for version `7`. This allows item `8.8` to continue test-first.
b) Keep the existing history-version test unchanged — leave item `8.8` paused until a different history compatibility contract is supplied.

**Recommended:** a) — keeps the rejection test testing a genuinely newer schema while preserving existing databases through an additive migration. (investigator)

**Verdict:**

## Tests

Each behavior was exercised in a compiling assertion-failing $\color{red}{\textbf{RED}}$ commit before its $\color{green}{\textbf{GREEN}}$ implementation.

| Behavior | Evidence |
|---|---|
| Claude distinguishes turn exhaustion | Authorized regression in [`crates/agent-runtime/tests/cli_adapters/claude.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/631f1cf6a01234c6c02deb9f4efaa5a708de4d5f/crates/agent-runtime/tests/cli_adapters/claude.rs); `25` Claude tests passed. |
| Version `3` contract and rejection of version `2` | [`crates/agent-core/tests/protocol_v3.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/631f1cf6a01234c6c02deb9f4efaa5a708de4d5f/crates/agent-core/tests/protocol_v3.rs) and authorized protocol tests; targeted protocol runs passed. |
| Configuration defaults and boundaries | [`crates/agent-core/tests/config_turn_limits.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/631f1cf6a01234c6c02deb9f4efaa5a708de4d5f/crates/agent-core/tests/config_turn_limits.rs); `3` tests passed. |
| Adapter budgets and unchanged unsupported CLIs | [`crates/agent-runtime/tests/turn_limits.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/631f1cf6a01234c6c02deb9f4efaa5a708de4d5f/crates/agent-runtime/tests/turn_limits.rs); `2` tests passed. |
| Turn-limit classification, quota and stop precedence | [`crates/agent-core/tests/turn_limit_outcomes.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/631f1cf6a01234c6c02deb9f4efaa5a708de4d5f/crates/agent-core/tests/turn_limit_outcomes.rs); `1` table-driven test passed. |
| First, last allowed and refused resumes; reset and missing session | [`crates/agent-core/tests/turn_limit_policy.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/631f1cf6a01234c6c02deb9f4efaa5a708de4d5f/crates/agent-core/tests/turn_limit_policy.rs); `2` table-driven tests passed. |

## Checks

- `cargo check --workspace --all-targets --locked`: passed during implementation.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: passed with `0` findings at this checkpoint.
- `GATE_WAIT=1 make gate`: final checkpoint verification follows this report commit; the handoff records its actual result.
- No old Bash or Go suites, real model CLIs, hardware devices or live installation were used.

## Acceptance

| Criterion | Status | Evidence |
|---|---|---|
| `A1`: two runs end `done`, never `failed` | Not met | Daemon continuation and end-to-end tests remain to be implemented. |
| `A2`: configured `max_turns = 50` reaches the fake CLI | Not met | Adapter arguments are tested; engine wiring and daemon launch evidence remain. |

## Deviations

- The adapter-budget RED commit [`18fe46a6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/18fe46a6a5e0f0b6a5f7d05cf2a3f73a59c669db) identifies RED in its body but omits the required subject prefix. Its test failed before implementation; history was not rewritten.
- Existing exhaustive test helpers received additive `TurnLimit` match arms required by the new enum variant; previous branches and expectations remain unchanged.
- No tests changed after their RED commit; no new lint suppressions or dependencies.

## Skipped tests

None added or changed. Remaining item-specific tests are not yet implemented.

## Not done

- History migration, continuation start reason and compatibility tests.
- Daemon queuing, same-session prompt, configured launch budget and exhaustion notification.
- Restart boundary, end-to-end history readback, secret-marker and golden coverage.
- Full protocol/state round trips and generated-input invariant coverage.
- User documentation and example configuration updates.
- Complete behavior-to-test acceptance mapping and final implementation gate.
- Pull request intentionally not opened, as instructed.
