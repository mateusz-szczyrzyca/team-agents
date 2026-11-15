# Round rust-agent/8.7 — inv-astra

## Status

`QUESTIONS`: a tested query slice is implemented. Round `8.7` is incomplete. The FTS5 migration exposed two additional tests that fix the final schema version at `4`; the requested permission is in `questions-4.md` for task `20261115-212152-inv-astra`. The third answer authorized exactly the two previously identified tests, so it does not cover these additional assertions.

## Implemented

- Bare `agent log` reads the newest `100` daemon log rows directly from the database.
- `--team`, `--round`, `--task`, `--run`, `--agent`, `--level`, `--event`, `--since` and `--until` intersect; timestamps include their fractional precision.
- `-n` bounds the newest matching rows, including `0`; rows are printed in insertion order.
- Row-mode `--json` emits structured log rows. Empty matches succeed with empty output.
- A lone `--round` or `--team`, with only `--prompts`, `--no-steps` or `--json`, preserves the existing story exactly, as the second answer requires.
- `--show log`, any implemented row filter, or both scopes together select row mode. Help and [`docs/log.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/db36b50b58c8b2ed5e506d7089fce15dc75f3282/docs/log.md) explain the rule. The pending `--grep` and `-f` will also select row mode when implemented.
- The reading API offers bounded pages after an ID; this is query infrastructure, not a claim that following or deletion-safe cursors are complete.

The implementation is in [`crates/agent-runtime/src/history/reader/query.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/db36b50b58c8b2ed5e506d7089fce15dc75f3282/crates/agent-runtime/src/history/reader/query.rs) and [`crates/agent/src/commands/log/rows.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/db36b50b58c8b2ed5e506d7089fce15dc75f3282/crates/agent/src/commands/log/rows.rs). The CLI and keyword enum changes are in [`crates/agent/src/main.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/db36b50b58c8b2ed5e506d7089fce15dc75f3282/crates/agent/src/main.rs) and [`crates/agent-core/src/log.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/db36b50b58c8b2ed5e506d7089fce15dc75f3282/crates/agent-core/src/log.rs); these additional files are necessary for typed parsing and command dispatch. [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/db36b50b58c8b2ed5e506d7089fce15dc75f3282/README.md) documents the visible change.

## Blocking schema contract

- `the_migration_gives_the_records_of_a_version_two_file_a_sequence_in_the_order_of_time` requires final schema version `4`: [`crates/agent-runtime/tests/history/ledger.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/crates/agent-runtime/tests/history/ledger.rs#L360).
- `version_one_fixture_remains_readable_and_migrates_without_losing_records` requires final schema version `4`: [`crates/agent-runtime/tests/history/sessions.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/crates/agent-runtime/tests/history/sessions.rs#L74).
- Both tests pass their preceding data-preservation assertions and fail at the final version comparison: actual `5`, expected `4`.
- An audit of the Rust schema-version assertions found these two additional hard-coded final-version checks.

[`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/AGENTS.md), Never rule `1`, says: “If the task requires behaviour that an existing test forbids, stop and ask the human in your questions file; wait for the answer.” The third answer explicitly permits changes to exactly the previously named schema and daemon-log tests. Neither newly conflicting test has been changed.

## Question

May only the final-version assertions in these two additional tests use the version declared by the pinned schema file?

- Recommended option `a`: change those assertions in a separate `TEST-FIX:` commit, preserving every readability, data-preservation, session and ordering assertion; list the changes under `Tests changed after RED`.
- Option `b`: retain those assertions, which blocks the forward migration and requires revised scope.

The full question is in the task's `questions-4.md`, with an empty verdict.

## Pending search work

The search work is preserved locally and is not included in the pushed branch while the migration is blocked:

- A test-first migration test uses populated version `4` databases and seeds `8703`, `8704`, `8705`; it verifies preservation, indexed existing and new records, Polish text, quotes, punctuation and FTS operator words treated as text.
- Its compiling stub first failed by returning no matching rows; the FTS5 implementation then passed this test.
- The two test changes authorized by the third answer were isolated in a `TEST-FIX:` commit. Ordinary tables remain subject to `STRICT` checks; the FTS5 virtual table and all shadow tables are named explicitly.
- Runtime clippy passed with `-D warnings`.
- The complete history suite reported `100` passed and `4` failed. Two failures are the newly conflicting version assertions. Two are unfinished implementation work: documenting the new schema objects and reconciling the search helper view with the schema inventory check. These latter failures do not require permission to fix production code or documentation.
- Local recovery uses the task-specific Git stashes named `inv-astra 20261115-212152 search implementation awaiting questions-4` and `inv-astra 20261115-212152 search RED and authorized TEST-FIX awaiting questions-4`. The first stash retains the original RED and TEST-FIX commits through its parent. No pending search code was pushed.

## Tests

These new tests prove queries against records written through the history API. They do not yet provide the round's required daemon-generated, two-team, three-round end-to-end evidence.

| Implemented behavior | Test |
|---|---|
| Each of `9` filters and every pair agrees with complete rows, including fractional timestamp bounds; newest/after/zero windows | `every_filter_and_pair_matches_the_unfiltered_rows_including_fractional_boundaries`, seeded with `87`, `8701`, `8702` |
| Default rows, combined scopes, explicit log view, exact JSON, empty and zero limits, mismatched scope | `default_rows_and_intersected_scopes_print_exact_json_and_limit_edges` |
| Invalid IDs, agent, level, event, timestamp, negative/overflowing count, missing scope and unknown view return usage status | `row_argument_errors_preserve_usage_status_without_opening_history` |
| Existing story text, JSON, direct-reading and old-schema behavior | Existing story and process tests, unchanged except the authorized bare-command status assertion |

New test sources:

- [`crates/agent-runtime/tests/history/queries.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/db36b50b58c8b2ed5e506d7089fce15dc75f3282/crates/agent-runtime/tests/history/queries.rs)
- [`crates/agent/src/commands/log/tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/db36b50b58c8b2ed5e506d7089fce15dc75f3282/crates/agent/src/commands/log/tests.rs)

## Existing tests changed with permission

- Removed only the obsolete bare-command and combined-scope rejection cases from `log_refuses_no_subject_two_subjects_and_a_name_that_is_not_an_id_with_status_2` in [`crates/agent/src/tests/log_command.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/db36b50b58c8b2ed5e506d7089fce15dc75f3282/crates/agent/src/tests/log_command.rs). Invalid-ID cases remain; new tests cover successful cases and additional edges.
- Changed only bare `agent log`'s expected exit status from `2` to `0` in [`crates/agent/tests/log_process.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/db36b50b58c8b2ed5e506d7089fce15dc75f3282/crates/agent/tests/log_process.rs), as expressly authorized in the second answer. Story-output assertions remain unchanged.

## Test-first commits

- $\color{red}{\textbf{RED}}$ [`ada6304d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ada6304d377c408bcd9c7c7a1408e6dc55aba125): query assertion failed because the stub returned no rows instead of the expected `8`.
- $\color{green}{\textbf{GREEN}}$ [`af5008dd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/af5008dd6625cbd838561f116fc57eb6b4b3a840): implement intersecting bounded queries.
- $\color{red}{\textbf{RED}}$ [`913f1e0c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/913f1e0c3a20cabb0f8aa5192c270b39dc40f20f): CLI assertion failed because the stub returned no messages instead of `first`, `second`, `third`.
- $\color{green}{\textbf{GREEN}}$ [`db36b50b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/db36b50b58c8b2ed5e506d7089fce15dc75f3282): print rows and preserve scoped stories.

## Tests changed after RED

- [`6ebf9a27`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6ebf9a27341486831cb9535b0b8f16caf006e9a8): replace the forbidden `unreachable!` with a permitted test `panic!` on an invalid fixture index; expectations unchanged.
- [`34620a7c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/34620a7c8898610bc379307a78a2dc508888feba): replace an emptiness assertion with an equivalent byte comparison required by clippy; expectations unchanged.

## Checks

- `tools/test-lock.sh inv-astra-query-green cargo test -p agent-runtime --test history queries --locked`: `3` passed, `0` failed.
- `tools/test-lock.sh inv-astra-cli-green cargo test -p agent --bin agent log --locked`: `55` passed, `0` failed.
- `tools/test-lock.sh inv-astra-log-process-green cargo test -p agent --test log_process --locked`: `3` passed, `0` failed.
- `cargo clippy -p agent-runtime --all-targets --all-features --locked -- -D warnings`: passed after the recorded test correction.
- `cargo clippy -p agent --all-targets --all-features --locked -- -D warnings`: passed after the recorded test correction.
- `cargo fmt --all`: completed.
- The final committed report is followed by `GATE_WAIT=1 make gate`; its actual result is reported in the final response.

## Acceptance

- `A1`: not met; prompts and agents views remain unimplemented.
- `A2`: not met; FTS5 search and following remain unimplemented.
- `A3`: not met; retention and statistics remain unimplemented.

## Skipped tests

None added or modified. No lint suppressions added. No tools were missing.

## Not done

- Complete and publish the pending FTS5 indexing and literal-search work after the additional test permission; add the remaining end-to-end search coverage.
- Following across restart and deletion, with durable monotonic row IDs.
- Views other than `log`, including `usage`.
- Startup and daily retention, gate-file removal, atomic deletion, vacuum and statistics.
- Removal of `activity.jsonl` and migration of its readers and tests.
- Full daemon-run acceptance coverage, golden files, secret-marker checks and the `100,000`-step index-plan checks.
- Remaining configuration, history and state documentation for those features.

No PR was opened. No other agent's worktree was inspected. No real model CLI or old-implementation test suite was run. This branch is a partial implementation awaiting the schema-test decision, not a completed round.
