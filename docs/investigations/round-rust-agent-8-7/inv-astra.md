# Round rust-agent/8.7 — inv-astra

## Result

Implemented round `8.7` locally on `agents/inv-astra/20261115-212152`, but the final gate is blocked by two existing history-failure tests. The human authorized the tests-only synchronization correction in answer `5`; the implementation is restored for that correction and final gate verification.

- Bare `agent log` reads the newest `100` daemon log rows directly, without a daemon connection.
- All `9` row filters intersect; `-n`, JSON lines, literal FTS5 search, durable following, all `9` focused views and database statistics are implemented.
- The human's scoped-story rule is preserved: a lone `--round` or `--team`, optionally with `--prompts`, `--no-steps` or `--json`, still prints the existing story. Row options or `--show log` select log rows. Combined team and round scopes also select rows. Existing story-output assertions are unchanged.
- Retention runs through the sole writer at startup and daily, using the injected clock. Closed rounds expire exclusively before the cutoff; open rounds and shared records of surviving rounds stay. Gate output is removed, empty teams disappear, and `auto_vacuum = FULL` reclaims pages.
- History schema `6` adds FTS5 and durable log cursors through forward migrations. Older histories remain readable without migration. A populated version `4` database migrates without data loss and becomes searchable.
- Rust task activity is recorded in history; `activity.jsonl` is no longer written. Existing Rust consumers and their tests use history reads.

## Acceptance

| ID | Result | Evidence |
|---|---|---|
| `A1` | met | `two_teams_three_rounds_prove_every_filter_pair_view_and_literal_search` checks exact scoped prompt runs and agent roles for both teams and every round. |
| `A2` | met | The same daemon test finds a unique prompt phrase; `log_follow_observes_runs_once_across_daemon_kill_restart_and_retention` observes actual new run starts through the running follower before and after daemon restart, with strictly increasing unique IDs. |
| `A3` | met | `daemon_daily_retention_removes_closed_round_and_gate_but_keeps_old_open_round` sets `keep_days = 1`, advances the injected clock by `2` days, and reads `agent log --stats`: rounds change from `2` to `1`; the closed round and its gate output disappear while the old open round survives, including another startup. |

Daemon acceptance tests: [`crates/agent/src/daemon/owner/rounds/tests/history_queries.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent/src/daemon/owner/rounds/tests/history_queries.rs).

Process following test: [`crates/agent/tests/history_process.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent/tests/history_process.rs).

## Tests

| Behavior | Test proving it |
|---|---|
| Default newest rows, JSON lines, direct reading, all single filters and pairs | `default_rows_and_intersected_scopes_print_exact_json_and_limit_edges`; `two_teams_three_rounds_prove_every_filter_pair_view_and_literal_search` (`2` teams, `3` rounds, `9` single filters and `36` pairs compared with full JSON output) |
| Inclusive fractional timestamps, zero/one limits, empty matches, conflicting scopes and invalid CLI values | `every_filter_and_pair_matches_the_unfiltered_rows_including_fractional_boundaries` (seeds `87`, `8701`, `8702`); `row_argument_errors_preserve_usage_status_without_opening_history` |
| Preserved scoped stories, direct access without the daemon, legacy reads and error status | Existing story-output tests and process tests; only the explicitly authorized obsolete parser cases change |
| Literal prompt, report and log search; Polish letters, quotes, percent, asterisk and FTS operator words | `two_teams_three_rounds_prove_every_filter_pair_view_and_literal_search`; `populated_version_four_migrates_without_loss_and_indexes_old_and_new_text` (seeds `8703`, `8704`, `8705`) |
| Recorded questions, answers, steps and final messages are searchable | `daemon_recorded_questions_answers_steps_and_final_messages_are_searchable`; `searches_the_tool_name_and_detail_of_a_step` |
| Following prints later rows in order once through restart and deletion | `log_follow_observes_runs_once_across_daemon_kill_restart_and_retention`; `follows_new_rows_once_in_order_across_restart_and_deletion`; `deleting_all_log_rows_and_reopening_never_reuses_a_follow_cursor` |
| Every focused view for round and team; explicit usage notice | `two_teams_three_rounds_prove_every_filter_pair_view_and_literal_search`; `scoped_views_and_literal_search_select_only_their_subject` |
| Tasks have running/completed status, roles and agents; worktrees have their recorded final head | `task_and_worktree_views_show_running_status_and_the_last_recorded_head`; the two-team test |
| Human-readable focused output | `focused_views_of_a_daemon_round_have_normalized_golden_output` (seed `8707`), plus unchanged story goldens |
| Output flush failures are errors | `every_projection_reports_a_failure_to_flush_output` |
| Startup and daily scheduling, disabled retention, exact day boundary and very large retention age | `retention_is_due_at_start_and_each_day_using_the_injected_time`; daemon daily-retention acceptance test |
| Exactly expired closed rounds and all owned rows removed; open and cutoff-boundary rounds stay; zero keeps all | `retention_keeps_open_and_boundary_rounds_and_removes_all_old_owned_rows` (seeds `8710`, `8711`, `8712`) |
| Team with an old and a new round keeps the new round and shared runs | `removing_one_old_round_preserves_the_new_round_and_shared_runs_of_the_team` |
| Gate files removed; failed cleanup leaves database round intact; symlink parents cannot escape home | Daemon daily-retention test; `retention_refuses_a_gate_parent_symlink_and_keeps_the_round_atomic` |
| Readers see whole snapshots during deletion | `retention_keeps_open_and_boundary_rounds_and_removes_all_old_owned_rows` holds a reader snapshot across retention and then reads the committed state |
| Logical and physical file shrink | The retention boundary test; `retention_shrinks_the_checkpointed_database_file` compares actual main-file sizes after closing/checkpointing |
| Cleanup through the sole writer, full/closed queue, cleanup error reporting and continued writes | `retention_runs_through_the_single_writer_and_noting_wrapper`; `full_or_closed_retention_queue_reports_once_without_losing_records`; `refused_retention_is_reported_once_and_later_records_are_written` |
| Database size, every table count, oldest timestamp, empty/legacy histories | `grep_is_a_literal_log_filter_and_stats_work_without_a_daemon`; `statistics_read_a_legacy_database_without_migrating_it`; `statistics_include_the_oldest_record_before_the_ledger_migration` |
| Actual row SQL uses indexes with `100,000` steps | `every_row_filter_and_pair_uses_an_index_with_a_hundred_thousand_steps` checks default, every predicate and every pair, in newest and follow windows; `timestamp_filters_read_version_four_without_requiring_new_indexes` covers legacy SQL |
| End of activity files; task history across runs, ordering, isolation, long Unicode text and explicit truncation metadata | `task_activity_includes_each_run_in_order_and_excludes_other_tasks`; migrated activity tests listed below; negative activity-file assertion |
| No secret reaches SQLite, WAL or search storage | Existing daemon test `a_merged_round_redacts_echoed_secrets_in_prompts_arguments_steps_messages_and_commits` scans database bytes; search is populated only from already-redacted stored fields |
| Unwritable/newer history does not stop work | Existing `a_history_in_the_way_does_not_stop_a_round_and_status_shows_what_was_lost`; `a_history_of_a_newer_version_is_left_as_it_is_and_the_round_goes_on` |

Additional test sources:

- [`crates/agent-runtime/tests/history/retention.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent-runtime/tests/history/retention.rs)
- [`crates/agent-runtime/tests/history/search.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent-runtime/tests/history/search.rs)
- [`crates/agent-runtime/tests/history/queries.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent-runtime/tests/history/queries.rs)
- [`crates/agent-runtime/tests/history/writer.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent-runtime/tests/history/writer.rs)
- [`crates/agent-runtime/src/history/reader/query/tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent-runtime/src/history/reader/query/tests.rs)
- [`crates/agent/tests/log_queries.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent/tests/log_queries.rs)
- [`crates/agent/tests/log_follow.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent/tests/log_follow.rs)
- [`crates/agent/tests/golden/log-focused-views.txt`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent/tests/golden/log-focused-views.txt)

The large query-plan dataset is synthetic and proves query planning, not recording. Recording is exercised separately through daemon runs and the existing recording suites. Deletion table counts and shared-record boundaries use seeded histories written through the public writer API; the daemon acceptance test additionally proves scheduling, gate-file deletion and the public statistics result.

## Tests changed after RED

The first two human answers permit the obsolete bare-command and combined-scope parser assertions to change, including the process duplicate. The third and fourth permit the named schema inventory/version adaptations. The round explicitly permits migration of activity-file tests. All other committed test corrections are isolated below for review.

Authorized existing CLI changes in [`913f1e0c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/913f1e0c3a20cabb0f8aa5192c270b39dc40f20f):

- `log_refuses_no_subject_two_subjects_and_a_name_that_is_not_an_id_with_status_2`: remove only the obsolete bare-command and combined-scope rejection cases; invalid IDs stay.
- `an_unknown_round_ends_with_status_3_a_bad_command_line_with_status_2_and_a_closed_history_with_status_1`: change only bare-command exit status to `0`; other error and story assertions stay.

Authorized schema changes:

- [`96dae565`](https://github.com/mateusz-szczyrzyca/team-agents/commit/96dae5654b94c07e77becd798b8f4200bfefb6b6): `the_pinned_file_is_the_version_of_the_last_migration_and_every_table_of_it_is_strict` explicitly inventories the virtual FTS5 table and every shadow table; all ordinary tables remain `STRICT`.
- [`96dae565`](https://github.com/mateusz-szczyrzyca/team-agents/commit/96dae5654b94c07e77becd798b8f4200bfefb6b6): `older_databases_read_empty_daemon_logs_and_upgrade_to_version_four_without_loss` changes only its final version expectation to the pinned schema version; readability and preservation remain.
- [`a9b41f3e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a9b41f3efd95ae4d2d1734b7b42573f068b533f6): `the_migration_gives_the_records_of_a_version_two_file_a_sequence_in_the_order_of_time` changes only its final version expectation.
- [`a9b41f3e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a9b41f3efd95ae4d2d1734b7b42573f068b533f6): `version_one_fixture_remains_readable_and_migrates_without_losing_records` changes only its final version expectation.
- [`af8a22cc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/af8a22cc6ae82618ccb10b9eef379c51232c8d7a): the same exact schema inventory gains the ordinary `STRICT` cursor table for schema `6`.

Activity migration in [`44aefa74`](https://github.com/mateusz-szczyrzyca/team-agents/commit/44aefa74c737f3b5ccde85503ccd474e1fbc1f04):

- Five store tests move to [`crates/agent-runtime/tests/history/activity.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent-runtime/tests/history/activity.rs): `append_activity_writes_one_json_line_per_event_in_order` becomes `append_activity_writes_one_record_per_event_in_order`; `the_activity_log_grows_across_stores_and_is_kept_per_task`; `append_activity_cuts_a_long_text_on_a_character_boundary`; `append_activity_keeps_a_text_at_the_limit_whole`; `append_activity_reports_a_log_that_cannot_be_opened`. Assertions use validated history and explicit cut metadata instead of JSONL and an ellipsis.
- [`crates/agent-runtime/tests/activity_control.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent-runtime/tests/activity_control.rs): both control-client tests retain their action/client/login assertions through a history-backed helper.
- [`crates/agent-runtime/tests/run_engine.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent-runtime/tests/run_engine.rs): `the_activity_of_a_run_is_logged_in_the_task_directory_in_order` reads recorded steps in order.
- [`crates/agent/src/daemon/gate/tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent/src/daemon/gate/tests.rs): `a_gate_that_ends_is_told_to_the_subscribers_and_logged_in_the_activity_of_the_task` reads the recorded gate event, retaining subscriber and result assertions.
- [`crates/agent/src/daemon/owner/controls/tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent/src/daemon/owner/controls/tests.rs): `the_activity_names_who_stopped_the_task_and_the_event_is_published`; `the_activity_names_the_dashboard_login_that_paused_the_task`; `pause_holds_the_task_and_unpause_resumes_its_session`; `resume_starts_a_stopped_task_again`; `stopping_a_team_stops_every_task_of_it_and_the_event_is_for_the_team`; `a_control_that_does_nothing_to_a_task_says_why`; `a_word_to_a_task_is_a_message_to_its_agent_from_the_human`; `a_word_that_the_bus_does_not_take_is_refused_and_not_recorded`. The helper waits for persisted events rather than assuming the asynchronous writer has finished at reply time.
- [`crates/agent/src/daemon/tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent/src/daemon/tests.rs): `the_activity_of_the_run_is_in_the_task_directory` reads history.
- [`crates/agent/tests/steering_process.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent/tests/steering_process.rs): `pause_unpause_and_stop_change_the_status_and_the_activity_names_the_client` reads history.
- [`crates/agent/src/daemon/owner/fixture.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/crates/agent/src/daemon/owner/fixture.rs): the activity helper reads the team's recorded events.

Other test-only corrections:

- [`6ebf9a27`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6ebf9a27341486831cb9535b0b8f16caf006e9a8): Query helper: replace forbidden `unreachable!` with a permitted assertion panic; inputs and expectations unchanged.
- [`34620a7c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/34620a7c8898610bc379307a78a2dc508888feba): Argument-error test: compare bytes directly for clippy diagnostics; same error assertion.
- [`77204bff`](https://github.com/mateusz-szczyrzyca/team-agents/commit/77204bff7c70785a8135e4a8cbc55b571be0c4af): Statistics test: expect the persisted nanosecond timestamp representation, not a shortened representation of the same instant.
- [`e0fa8bf9`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e0fa8bf98ba53e6a498efcbb1ac7a6808c332dc3): Shared query fixture: mark it test-only and use a concrete map constructor; add a legacy-statistics test.
- [`35b14c8f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/35b14c8f4647e881a2b0fb498d3c3e4e90b56b89): Retention test: use a unique search marker; random surviving text could legitimately contain the old single-letter marker.
- [`b72004d7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b72004d7ee2060aef87a8ef3bc49437dc1563403): Retention test: extract unchanged table-count assertions into a helper to satisfy the function-size lint.
- [`35fc8056`](https://github.com/mateusz-szczyrzyca/team-agents/commit/35fc80564655c7e542826eb148765cda94850465): Migrated gate test: put imports before statements; assertions unchanged.
- [`d839b94a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d839b94adc7e46f4ae6016a4197e59988db9110c): Migrated activity tests: remove unused JSON import and timestamp helper.
- [`e3d34972`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e3d3497290deec3dd277621131325444a14b1b03): Daemon query test: compare whole results instead of `is_empty`; expected results unchanged.
- [`7ed7482e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7ed7482e4b0dba10740019c6e58e41a37903e08f): Running-view test: replace the immediately finishing sleep scenario with an explicit gate barrier; eliminates a scheduling race without changing the running-status requirement.
- [`9a2f5ab2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9a2f5ab2eb8b64dba9d1331d62de21d5f1cfc13e): Legacy oldest-record test: compare parsed instants, not equivalent timestamp text formats.
- [`6ae2591d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6ae2591d06b6074baa5f28053b47b6ecb32f1e34): Query-plan and process-follow tests: clippy-required numeric separators and inline format capture; no semantic changes.
- [`ee3dd1f4`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ee3dd1f43673e9c34f5e14d0d68423787904dabf): Golden test: write headings directly to the buffer; identical output and assertions.

## Test-first history

The implementation has separate $\color{red}{\textbf{RED}}$ and $\color{green}{\textbf{GREEN}}$ commits for queries, CLI mode selection, FTS5, views/statistics, retention, durable cursors, scheduling, following, task-step reads, activity removal, task status, final heads/tool search, safe gate deletion, indexed time filtering, legacy compatibility, flush failures and oldest-record statistics. RED commit bodies include their actual failing assertions. Later end-to-end and golden additions strengthen those contracts.

- First query failure: [`ada6304d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ada6304d377c408bcd9c7c7a1408e6dc55aba125); implementation: [`af5008dd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/af5008dd6625cbd838561f116fc57eb6b4b3a840).
- Populated migration/search failure: [`ca47c694`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ca47c694591e19f4e9b0672a4e6d52237beaad07); implementation: [`8cf283ce`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8cf283ce10a161d04b7b70e5d434c869307751f3).
- Retention failure: [`ad3d4a5b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ad3d4a5b615eddc12e3bd91a9cae590e6c7379b8); implementation: [`8057cbef`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8057cbefa944bccd6e4be506bb71a0a1a07bbf0c).
- Follow failure: [`3271720c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3271720c70844d6d018e208eeb501b32b8e93a38); implementation: [`4626595d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4626595d20cb69507608fdc7644385d8839d0ca7).
- Activity-file prohibition: [`ddd6b8df`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ddd6b8df794b77af9a694c3eea2d70734837cc36); implementation: [`83a6747b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/83a6747baf435558908c8606b5d76009e21d14ba).
- Final production fix, legacy oldest statistics: [`bc6d7373`](https://github.com/mateusz-szczyrzyca/team-agents/commit/bc6d7373e2959ba5f7a9c08c76fb830e690099ce); implementation: [`71224f25`](https://github.com/mateusz-szczyrzyca/team-agents/commit/71224f25930710835906b9e97b25029f65fa30fd).

## Checks

- `tools/test-lock.sh inv-astra-history cargo test -p agent-runtime --test history --locked`: `118/118` passed before the final two queue/failure tests were added.
- `tools/test-lock.sh inv-astra-retention-failures cargo test -p agent-runtime --test history retention --locked`: `8/8` passed, including both added failure tests.
- `tools/test-lock.sh inv-astra-search cargo test -p agent daemon_recorded_questions_answers_steps_and_final_messages_are_searchable --locked`: `1/1` passed.
- Focused daemon acceptance, running/final-head, golden, follow-process, physical-shrink, query-plan, legacy and flush tests passed in their targeted development runs.
- `cargo clippy -p agent -p agent-runtime --all-targets --all-features --locked -- -D warnings`: passed; runtime clippy passed again after the final writer tests.
- `cargo fmt --all`: completed.
- `git diff --check`: passed.
- Final handover verification: run `AGENT_GATE_RUN=1 GATE_WAIT=1 make gate` on the report commit before pushing. The final response gives its actual result. `AGENT_GATE_RUN=1` prevents the lock script from invoking the running Bash installation's `agent gate`; the same repository gate and machine-wide lock still run, without writing into the live agents home.

## Compatibility and limits

- Schema upgrades are forward-only; old read-only clients remain compatible with supported older versions. Ordinary tables remain strict, and no existing stored history is rewritten into a legacy shared format.
- `--grep` means an escaped literal token phrase, not arbitrary substring matching; FTS5 punctuation tokenization still applies. A pre-search database has no matches until the writer migrates it.
- Non-log focused views project their complete selected scope. `--stats` describes the whole database and reports logical page bytes, excluding WAL/shared-memory sidecars.
- File removal and SQLite commit cannot be one atomic transaction. A failure rolls back database deletion, but output files removed earlier in the pass can already be absent. Missing files permit recovery on the next pass; paths cannot traverse symlinked directories outside the home.
- Retention removes history and gate output, not authoritative state or worktrees. Disabling history also disables persisted Rust task steps.
- The current Rust CLI has no `show TASK` subcommand to migrate. Its existing activity readers are migrated, and `Reader::steps_of_task` provides task steps across runs; the old Bash implementation is untouched.
- Follow prints new IDs once; a coalesced warning update keeps its ID and is not reprinted. Durable cursors handle an emptied database and daemon restarts, not replacement of the database by an unrelated file.

## Scope and deviations

- No dependency, unsafe code, lint suppression, ignored test, tool installation or hardware access was added.
- Additional files outside the item's short list provide typed CLI/configuration fields, command dispatch, writer queue/cursor migrations, activity recording removal from runtime/daemon owners, test fixtures, the pinned schema, README and the configuration example. All serve round `8.7`.
- The retention RED commit [`ad3d4a5b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ad3d4a5b615eddc12e3bd91a9cae590e6c7379b8) contains the failing test evidence but its subject lacks the required `RED:` prefix; its following implementation is separate. This is a commit-label deviation from `TDD-1`, not an omitted failing run.
- The `100,000`-step planner test checks the actual filtered log SQL. Statistics deliberately count entire tables; that operation cannot promise an indexed lookup instead of a count scan. Focused story projections reuse the existing scoped reading API.
- No later-round work was started. No PR was opened, as requested. No other worktree was inspected, real model CLI was run, or old implementation suite was executed.

## Skipped tests

None added or skipped. No required tool was missing.

## Size

The `PR-1` counting command reports `1777` changed non-test lines before replacing this report; the report remains within the relaxed `5000`-line ceiling. The final response reports the final count.

## Task

Full round item from [`rounds/round-8.7.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8b995504d328219541f9ca56324267eab19a2433/rounds/round-8.7.md), with the human's scoped-story clarification above governing its row-filter interpretation:

> 8. Autopilot, rounds, the history, checkpoints and accounts
> Depends on: 7 (merged before this round)
> [ ] 8.7 `agent log`: questions to the history, search, following and what is kept
>     `agent log` with no words prints the newest rows of the daemon's log. It reads the history directly and narrows
>     by `--team`, `--round`, `--task`, `--run`, `--agent`, `--level`, `--event`, `--since` and `--until`, alone or
>     together; `-n` limits, `--json` prints JSON lines, `-f` follows: rows written later are printed as they come,
>     each once, in order, across a restart of the daemon. `--grep TEXT` searches prompts, steps, final messages,
>     review reports, questions, answers and log messages (FTS5); the text is taken as text, whatever it holds.
>     For a round or a team, `--show` gives one thing only: `tasks` (with role, agent and status), `agents` (with
>     role and how chosen), `worktrees` (with branch and head), `runs`, `prompts`, `changes`, `reviews`, `log`, and
>     `usage`, which says that tokens are not recorded yet (11.1).
>     What is kept: `[history] keep_days` — the daemon deletes, at its start and once a day, the closed rounds older
>     than that with everything of theirs (runs, steps, changes, prompts, reviews, their log rows and the files of
>     their gates' output), the teams all of whose rounds are gone, and the log rows of no round older than `[log]
>     keep_days`; `0` keeps everything; an open round is never deleted. The file shrinks afterwards
>     (`auto_vacuum`). `agent log --stats` shows the size of the database, the rows of each table and the oldest
>     record. The task's `activity.jsonl` is no longer written: what read it — `show` of a task — reads the steps
>     from the history.
>     Expected result:
>     - Files: `crates/agent/src/commands/log.rs`, the queries and the search index in
>       `crates/agent-runtime/src/history/`, the deleting in `crates/agent/src/daemon/history.rs`, the removal of the
>       activity file from `crates/agent-runtime/src/store.rs` and its readers, `[history] keep_days` and `[log]
>       keep_days`, `docs/history.md`, `docs/log.md`, `docs/configuration.md`, `docs/state.md`.
>     - Adds: the filters, `--grep`, `-f`, `--show`, `--stats`; the search index; the deleting; the end of
>       `activity.jsonl`.
>     - Tests: the ground rule on testing these items applies. On a history written by daemon runs of two teams and
>       three rounds with fake CLIs: every filter alone and every pair of filters gives exactly the rows a test
>       works out from `--json` of the whole history; every `--show` for a round and for a team. Search: a word found
>       in a prompt, in a step, in a report and in a log message; Polish letters; a text with quotes, `%`, `*`, `"`
>       and the words `AND`, `OR`, `NEAR` is searched as text and breaks nothing. Following: rows written after the
>       start come in order and once, also when the daemon restarts and when a deletion runs meanwhile. Deleting:
>       exactly the closed rounds older than the limit are gone, from every table (the rows of each table counted
>       before and after), with their gate files; an open round of any age stays; a team with one old and one new
>       round stays with the new one; `keep_days = 0` deletes nothing; reading while the deletion runs sees whole
>       rounds only; the file is smaller afterwards. Every query of `agent log` uses an index: `EXPLAIN QUERY PLAN`
>       names one, for a history of a hundred thousand steps. The existing tests that read `activity.jsonl` read the
>       history instead; they are listed in the PR.
>     - Why: the questions asked of a round and of a team — its logs, tasks, agents, worktrees, prompts — each have
>       one command, and the home no longer grows without end.
>     Checks:
>     - A1. `agent log --round <id> --show prompts` prints the prompts of that round and of no other; `--show agents`
>       lists who took part, with roles.
>     - A2. `agent log --grep '<a word of one prompt>'` finds the run that got it; `agent log -f` shows a run starting
>       while it is watched.
>     - A3. With `keep_days = 1` and the clock moved two days on, a closed round is gone from `agent log --stats` and an
>       open one is still there.

## Gate blocker discovered at handover

- The full gate passed formatting and clippy, then the agent binary suite reported `764` passed and `2` failed.
- `failing_step_inserts_are_counted_while_other_records_survive_and_the_round_merges`: `merge_and_status` requires `Reply::Overview` before the writer has reported the injected failure; actual reply is `Reply::Teams`.
- `sqlite_full_does_not_prevent_merging_or_the_other_records_surviving`: the same status race at the same helper assertion.
- A diagnostic run excluding the global-panic-hook test exposed both panics: `763` passed, `2` failed, `1` filtered out. This was diagnosis, not gate verification.
- The history-failure group passes in isolation (`7/7`); that retry does not replace the failed gate.
- No existing fault-injection test was changed. Question `5` requests permission to synchronize the assertions with the asynchronous writer while preserving every existing behavior assertion.

The human answered question `5` with option `a`. The authorized correction and its validation are recorded below once complete.
