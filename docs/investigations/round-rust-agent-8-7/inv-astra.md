# Round rust-agent/8.7 — inv-astra

## Status

`QUESTIONS`: a tested query slice is implemented. Round `8.7` is incomplete. The next schema migration conflicts with existing tests; the requested permission is in `questions-3.md` for task `20261115-212152-inv-astra`.

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

- `the_pinned_file_is_the_version_of_the_last_migration_and_every_table_of_it_is_strict` fixes the complete table list and requires every table to be `STRICT`: [`crates/agent-runtime/tests/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/crates/agent-runtime/tests/history/schema.rs).
- `older_databases_read_empty_daemon_logs_and_upgrade_to_version_four_without_loss` requires the writer to upgrade to exactly version `4`: [`crates/agent-runtime/tests/history/logs.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/crates/agent-runtime/tests/history/logs.rs).
- FTS5 requires virtual and shadow tables. An in-memory SQLite probe returned `strict = 0` for the virtual table and its `5` shadow tables.
- The required FTS5 index and new schema need a forward migration. Existing migrations must remain readable.

[`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/AGENTS.md), Never rule `1`, says: “If the task requires behaviour that an existing test forbids, stop and ask the human in your questions file; wait for the answer.” Earlier permissions cover the CLI parser cases, not these schema assertions. Neither conflicting test has been changed.

## Question

May the schema-evolution assertions be updated for this round?

- Recommended option `a`: extend the exact inventory, retain `STRICT` checks on ordinary tables, explicitly verify FTS5 virtual and shadow tables, and verify upgrades to the latest version while preserving old-data checks. Add version `4` migration coverage and search edge cases before implementation.
- Option `b`: retain these assertions, which blocks the specified FTS5 migration and requires revised scope.

The full question is in the task's `questions-3.md`, with an empty verdict.

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

- FTS5 indexing and literal search, including the required Unicode and metacharacter cases.
- Following across restart and deletion, with durable monotonic row IDs.
- Views other than `log`, including `usage`.
- Startup and daily retention, gate-file removal, atomic deletion, vacuum and statistics.
- Removal of `activity.jsonl` and migration of its readers and tests.
- Full daemon-run acceptance coverage, golden files, secret-marker checks and the `100,000`-step index-plan checks.
- Remaining configuration, history and state documentation for those features.

No PR was opened. No other agent's worktree was inspected. No real model CLI or old-implementation test suite was run. This branch is a partial implementation awaiting the schema-test decision, not a completed round.
