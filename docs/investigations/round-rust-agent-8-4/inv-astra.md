# Investigation — Rust item 8.4 — `inv-astra`

Continued the stopped work of [#100](https://github.com/mateusz-szczyrzyca/team-agents/pull/100) on `agents/inv-astra/20261114-094819`. Item `8.4` is **not complete**. Sessions, startup loss accounting, a writer diagnostics deadlock and execution of Git extensions during snapshots are fixed and committed. The last complete Rust gate before the capture-loss work passed `2,305` test executions. A further fix for uncounted Git capture failures is implemented locally. The four authorized socket fixtures are now isolated from history, and the requested new loss-status regression is committed. A fifth frozen socket fixture has exposed an intermittent conflicting assumption; round-4 approval is needed before the fix can become GREEN.

## Completed fixes

- **Reader invariants:** SQL and the typed reader now agree about unused run-reason fields, nonrandom draw metadata and team-plan roles. Valid neighbouring cases remain accepted. No valid stored record or schema changed for this fix.
- **Sessions:** migration `2` adds append-only `run_sessions`; live parser observations are recorded in order with injected times and exposed by `Reader::sessions`. The supplied session remains in the launch record; echoing it does not create a new observation. A preserved version `1` fixture proves migration and old-data reading. A merged fake OpenCode round proves sessions learned during execution, including a changed session.
- **Startup failures:** failed opener tasks and writer-thread startup retain an unavailable history handle, report the cause and count subsequent records. They no longer silently behave as disabled history.
- **Writer diagnostics:** the binary held the stderr lock throughout daemon execution, blocking the writer's first warning and its subsequent progress. Passing stream handles instead of lifetime-long locks fixes that deadlock. A real daemon process now reports a rejected insert once, counts multiple losses and persists its run end.
- **Git extensions:** history snapshots disable configured clean/process filters and filesystem monitors through per-command Git configuration. They record raw worktree bytes without invoking those external commands or modifying repository configuration, the real index or the object database. Tests prove both filter and filesystem-monitor commands are not executed.
- **Failure proofs:** isolated history permission failure and rejected step inserts each allow a fake GitHub round to merge. A real daemon under queue pressure completes a brief, exposes losses and reports queue saturation once. The queue test uses an external SQLite write transaction to block the writer deterministically; its completion proof is a brief, not a merged round.

## Commits

- Reader $\color{red}{\textbf{RED}}$: [`20b096ea`](https://github.com/mateusz-szczyrzyca/team-agents/commit/20b096eaf8cdd962525231e647d343899a9f1be1).
- Reader $\color{green}{\textbf{GREEN}}$: [`4963b7e2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4963b7e2f219cd59ccae50e91dd91e9dfbd2689b).
- Sessions $\color{red}{\textbf{RED}}$: [`4ffb711c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4ffb711cf66606b7ea0a513f92a0647f2928a937).
- Supplied-session characterization: [`13750502`](https://github.com/mateusz-szczyrzyca/team-agents/commit/137505027ff49dd02c707fe6e77e4d4f52b5d3aa).
- Sessions $\color{green}{\textbf{GREEN}}$: [`7f15a607`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7f15a60754a3498e9a3288016ccf72a6ba9e5450).
- Startup losses $\color{red}{\textbf{RED}}$: [`368ac38c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/368ac38c8ea22f6bdb0a6a17a116d7661ce2c76f).
- Startup losses $\color{green}{\textbf{GREEN}}$: [`610cf4f8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/610cf4f82447dedfb0284b07f9bd17d7b40ed880).
- Daemon failure handling $\color{red}{\textbf{RED}}$: [`a155295f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a155295f49eecfb0b1d54bd3189802a990f645f3).
- Writer diagnostics $\color{green}{\textbf{GREEN}}$: [`888d9317`](https://github.com/mateusz-szczyrzyca/team-agents/commit/888d9317510066fdda06634ab36f47126dd24787).
- Git extension isolation $\color{red}{\textbf{RED}}$: [`8d39446c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8d39446c20beded484f61ede98db05bc99b74cc0).
- Git extension isolation $\color{green}{\textbf{GREEN}}$: [`7c9ca8ed`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7c9ca8ed8882370c7454c817307a1ca866e792d7).
- Capture losses $\color{red}{\textbf{RED}}$: [`57656608`](https://github.com/mateusz-szczyrzyca/team-agents/commit/57656608077a461217928f262d081c8b22d360db). Its implementation remains uncommitted pending the fixture permission below; this is not a GREEN head.

## Tests changed after RED

- Human-authorized `TEST-FIX:` [`88dc7138`](https://github.com/mateusz-szczyrzyca/team-agents/commit/88dc71381c20c0b7e6320916c00c375df15a5dfd) updates [`docs/history.schema.sql`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/docs/history.schema.sql) and the table inventory in [`crates/agent-runtime/tests/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/crates/agent-runtime/tests/history/schema.rs) for session migration `2`.
- Human-authorized `TEST-FIX:` [`abc029dd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/abc029ddbcb7858205bf6266fb2621f49c8f9c23) advances the unsupported-version fixture in [`crates/agent/src/daemon/owner/rounds/tests/history_records.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/crates/agent/src/daemon/owner/rounds/tests/history_records.rs) to one beyond the latest migration. It preserves byte preservation, merged-round and unavailable-status assertions.
- Human-authorized `TEST-FIX:` [`ffaab8fc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ffaab8fc84c7fb8fd21b7466734ee869cf8bc9df) disables history only in the four round-3 fixtures. Its diff adds configuration writes inside those tests; shared fixture defaults and every assertion remain unchanged.
- The requested regression is $\color{red}{\textbf{RED}}$ in [`90c76d79`](https://github.com/mateusz-szczyrzyca/team-agents/commit/90c76d791958b044398076ffd3921dd9cb792202). Without the fix it receives `Teams`; with the pending fix it passes, asserting `Overview` with `Writing` and exactly `lost: 1`.
- The additional stale-socket fixture has not been changed. It is outside the explicit four-test approval.

## Tests

This maps the principal implemented requirements and regression proofs. It is not a claim that every sentence of item `8.4` has sufficient coverage; the remaining gaps are explicit below.

| Behaviour | Test or proof |
|---|---|
| Fresh schema equals the pinned schema; all tables STRICT | `a_fresh_database_has_the_schema_the_pinned_file_has`; `the_pinned_file_is_the_version_of_the_last_migration_and_every_table_of_it_is_strict` |
| WAL; forward migration; failed migration rollback; future version byte preservation | `the_database_is_in_write_ahead_log_mode`; `a_database_of_an_older_version_is_migrated_forward_and_keeps_its_rows`; `a_migration_that_fails_leaves_the_database_at_the_version_before_it`; `a_database_of_a_newer_version_is_refused_and_left_byte_for_byte_as_it_was` |
| Typed records round-trip with generated inputs and varying batches | `any_world_of_records_is_read_back_equal_whatever_the_size_of_the_batches` |
| Unicode and byte-limit boundaries; command argument boundaries | `text_in_every_script_and_with_awkward_characters_comes_back_as_it_went_in`; `a_text_at_the_limit_one_under_and_over_is_kept_or_cut_at_a_character_boundary_and_marked`; `the_arguments_of_a_command_line_are_cut_one_by_one_so_the_list_stays_a_list` |
| Idempotence, rejected references and transaction atomicity | `the_same_record_written_twice_is_one_row_and_the_first_write_wins`; `a_record_that_belongs_to_a_run_the_history_never_saw_is_refused`; `a_batch_with_one_refused_record_writes_none_of_its_records` |
| Concurrent run order and whole-record reads | `the_steps_of_several_runs_recorded_at_once_keep_the_order_of_each_run`; `a_reader_that_reads_while_the_writer_writes_sees_whole_records_only` |
| Reader independently rejects invalid variants and selection metadata | `every_run_reason_rejects_fields_of_other_variants_in_sql_and_when_read`; `each_nonrandom_selection_rejects_draw_candidates_in_sql_and_when_read`; `reviewer_is_a_run_role_but_not_a_team_plan_role_in_sql_or_when_read` |
| Team plans, roles, round and run recording | `the_plan_of_a_team_says_how_each_participant_got_its_role`; `a_solo_round_is_in_the_history_from_its_team_to_its_reviews_with_every_run_once` |
| Coordinator/member/worker prompts and actual command lines | `the_prompt_and_the_command_line_of_every_run_equal_what_its_cli_got_byte_for_byte` |
| Worker requester linkage | `a_worker_that_serves_a_request_has_a_run_that_names_the_run_that_asked` |
| Ordered steps and Git changes from actual fake CLI execution | `the_steps_of_a_run_are_what_its_cli_printed_in_order`; `the_changes_of_a_run_are_what_its_cli_did_a_file_added_deleted_modified_renamed_binary_and_one_left_uncommitted` |
| Restart uniqueness; resume/answer/item/wake-up reasons | `a_daemon_stopped_in_the_middle_of_a_run_leaves_one_row_for_each_run_and_the_next_run_goes_on`; `a_stop_and_a_resume_by_the_human_are_a_control_step_of_the_run_and_the_reason_of_the_next`; `the_answers_of_the_human_are_the_reason_of_the_run_that_goes_on`; `the_next_item_of_a_solo_agent_and_the_wake_up_of_a_coordinator_have_their_reasons` |
| Supplied session is not duplicated | `echoing_the_session_supplied_to_the_cli_is_not_a_new_session` |
| Secrets absent from history files | `no_value_of_a_secret_or_of_the_environment_of_the_clis_is_in_the_files_of_the_history` |
| Directory and newer-schema failures permit merged rounds | `a_history_in_the_way_does_not_stop_a_round_and_status_shows_what_was_lost`; `a_history_of_a_newer_version_is_left_as_it_is_and_the_round_goes_on` |
| Permission and SQL write failures permit merged rounds | `read_only_history_storage_leaves_state_writable_and_the_round_merges`; `failing_step_inserts_are_counted_while_other_records_survive_and_the_round_merges` |
| History snapshots do not execute extensions | `history_reads_raw_worktree_bytes_without_running_configured_filters`; `history_reads_do_not_invoke_a_configured_filesystem_monitor` |
| Failed capture counts and unavailable-history capture skipping, pending implementation | `failed_head_and_change_capture_are_visible_as_losses_without_changing_the_run`; `unavailable_history_counts_the_skipped_capture_without_trying_the_invalid_repository`; `failed_git_snapshots_report_once_and_status_counts_each_missing_record` |
| Documented inventory, runnable SQLite queries and normalized output | `every_table_column_and_index_of_the_schema_is_documented`; `the_queries_of_the_page_list_the_runs_of_a_round_print_a_prompt_whole_and_give_the_steps_and_files_of_a_run`; `what_the_queries_of_the_page_print_for_a_round_is_pinned` |

## Checks

- Last complete `make rust` before capture-loss work, at [`7c9ca8ed`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7c9ca8ed8882370c7454c817307a1ca866e792d7): `2,305` passing test executions, `0` failed, `0` ignored; formatting, clippy and documentation passed.
- Capture-loss RED: the new process test timed out with completed tasks but no loss count; the two new runtime assertions observed `0` instead of `2` and `5` instead of `6` losses. These failures are recorded in the RED commit.
- Before the round-3 approval, the pending capture implementation stopped at `656/660` daemon tests because of the four frozen fixtures.
- After the four approved fixture changes, one complete `make rust` run passed `2,308` test executions, formatting, clippy and documentation. This does not establish a stable GREEN result: the later run below exposes a race in a fifth existing fixture.
- With the new regression and pending capture implementation, `make rust` passes formatting and clippy, then stops at `660/661` daemon tests. Only `a_socket_file_left_by_a_dead_daemon_is_replaced` fails; the new exact-loss regression passes. The stale-socket test can request status before or after head-capture failure, so its prior passing run must not hide this failure (`LINT-8`).
- With that implementation, `tools/test-lock.sh capture-focused cargo test --locked -p agent-runtime --test run_history -p agent --test history_process` passes `16/16` tests: `11` runtime and `5` process tests.
- `git diff --check` passes.
- `git grep -l 'CREATE TABLE' -- crates` finds one Rust schema module, [`crates/agent-runtime/src/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/crates/agent-runtime/src/history/schema.rs), plus the preserved migration fixture [`crates/agent-runtime/tests/history/fixtures/schema-v1.sql`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/crates/agent-runtime/tests/history/fixtures/schema-v1.sql). The fixture deliberately retains the old schema for migration verification; it is not a second production schema module.
- An isolated manual fake-CLI probe reproduced clean-filter delays even with unavailable history, and an uncounted failed snapshot. The committed extension-isolation tests cover the first issue; the pending capture-loss tests cover the second. Only temporary homes were used.
- Serena was used for symbol discovery during this continuation. `sqlite3` is installed. No additional tool installation was needed.

## Scope and deviations

- The human approved a larger item instead of splitting it. Before this continuation's implementation, the planned ceiling was recorded as `7,500` counted lines. PR-1 count at capture RED is `5,280` lines against actual PR target [`150e2203`](https://github.com/mateusz-szczyrzyca/team-agents/commit/150e2203fe0ebd458c7f6f67c5676ab736378b5d), excluding tests, the lockfile and schema snapshot; this report replacement and pending implementation change that count.
- [`crates/agent/src/main.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/crates/agent/src/main.rs) is outside the item's named recording locations but its stream-lock fix is necessary for writer failure reporting and progress.
- The agent crate adds the already-approved bundled `rusqlite` workspace dependency for tests that provoke actual SQLite insert failures. Cargo regenerated the lockfile. No new library version or lint suppression was introduced.
- Session schema compatibility, session queries and raw snapshot semantics are documented in [`docs/history.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/docs/history.md); the reader fix adds the history reference to [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/README.md).
- External process tests reuse real operating-system polling and bounded wall-clock deadlines for process completion. This deviates from strict `TDD-8`; policy-time tests use the injected clock, and no new assertion measures elapsed execution time. The new process fixture kills and reaps its daemon on failure.
- The inherited unrelated `3`-line Bash change in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/bin/agent) was not altered or tested here.
- No new PR was opened, no later plan item was started, and no live installation, real model CLI or hardware device was used.

## Skipped tests

- Go suites were not run, as explicitly instructed.
- Bats suites were not run, as explicitly instructed.
- `make gate` was replaced by `make rust` to obey those exclusions.
- No Rust test was newly ignored or removed. After the pending implementation's latest daemon failure, the full gate did not reach subsequent test and documentation stages; focused suites were run separately under the test lock.

## Not done

- **Frozen fixture approval:** the capture-loss implementation remains local in [`crates/agent-runtime/src/history.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/crates/agent-runtime/src/history.rs), [`crates/agent-runtime/src/history/handle.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/crates/agent-runtime/src/history/handle.rs) and [`crates/agent-runtime/src/run.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/crates/agent-runtime/src/run.rs). These links show the committed RED versions, not the pending implementation. Its documentation update also remains uncommitted. That implementation counts capture failures once per missing record, reports once per cause, and avoids Git capture when history is unavailable. It must not be represented as committed or fully gated.
- **Strict no-delay guarantee:** healthy-history Git captures remain awaited before model launch and run completion. Disabling extension commands removes the reproduced filter delay, but slow Git itself can still delay work; this requirement remains unresolved.
- **Merged-round queue proof:** the new full-queue process test proves a brief finishes, not that the same full-queue round merges. That stronger proof is still required.
- **Seeded selection proof:** the inherited coordinator-draw test checks the selected participant and candidate set against state, but does not inject a seeded RNG into the daemon. It does not satisfy the explicit seeded end-to-end draw requirement.
- **Inherited process-test precision:** two older process tests look for `solo` and `done` anywhere in status; the summary header can contain `0 done`. New tests check an actual solo task line ending in `done`; the older frozen predicates remain unchanged.
- The complete per-sentence behaviour/test audit and final checkpoint remain outstanding. The final branch includes a RED test commit and must not be merged as finished item `8.4`.

## Answered questions — round 3

The external question file is `questions-3.md` for task `20261114-094819-inv-astra`. The question is retained below as the approval record. The human approved option `a` for exactly four tests, in one fixture-only `TEST-FIX`, and required the new exact-loss regression. The original pause was required by [`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/AGENTS.md), rule `1`: “Never modify existing tests.” The earlier approvals cover schema fixtures, not these socket tests.

## 1. May four socket-test fixtures disable history explicitly?
The capture-loss fix counts a failed Git head lookup and exposes it through `agent status`, as item `8.4` requires. Four existing socket tests use directories that are not Git repositories and expect `Reply::Teams`. They now receive `Reply::Overview` with `HistoryHealth { state: Writing, lost: 1 }`. In [`crates/agent/src/daemon/tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/57656608077a461217928f262d081c8b22d360db/crates/agent/src/daemon/tests.rs), the affected tests are:

- `status_over_the_socket_lists_the_teams_with_their_tasks`
- `status_of_a_team_or_a_task_names_only_its_team_and_an_unknown_one_is_not_found`
- `a_line_that_is_not_a_request_is_answered_with_an_error_and_the_connection_goes_on`
- `a_request_of_a_subscriber_that_arrives_in_parts_around_an_event_is_answered`

With the pending implementation, `make rust` passes formatting and clippy, then reports `656/660` daemon tests passing. Focused process and run-history suites pass `16/16`. Repository rule 1 says “Never modify existing tests”; the earlier permissions do not cover these four fixtures. The proposed change explicitly disables history only for these protocol tests before daemon startup, preserving their existing assertions and leaving shared fixture defaults unchanged. Dedicated history tests continue to prove degraded status and loss counts.

a) Authorize a dedicated `TEST-FIX:` adding `[history] enabled = false` to these four individual fixtures — preserves their protocol contracts while allowing failed history captures to be visible in status.
b) Keep these fixtures frozen — leaves the capture-loss fix blocked because its required status behavior conflicts with their assumptions.

**Recommended:** a) — isolates socket protocol tests from optional history recording without weakening their assertions or the history tests. (investigator)

**Verdict:** a) — approved for exactly the four named fixtures, with a new exact-loss status regression.

## Questions — round 4

The current external question file is `questions-4.md` for task `20261114-094819-inv-astra`. No fifth fixture change has been made.

## 1. May the stale-socket fixture also disable history explicitly?
The four authorized fixture changes are committed separately, with no assertion changes. The requested new regression, `status_of_a_run_outside_a_git_repository_reports_one_lost_capture`, was committed RED after observing `Reply::Teams`; with the pending fix it passes and asserts `Reply::Overview` with exactly `lost: 1`.

The next `make rust` run passes formatting and clippy and reports `660/661` daemon tests passing. Its sole failure is another existing test in [`crates/agent/src/daemon/tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/90c76d791958b044398076ffd3921dd9cb792202/crates/agent/src/daemon/tests.rs): `a_socket_file_left_by_a_dead_daemon_is_replaced`. This test also starts a run in a directory that is not a Git repository and asserts `Reply::Teams`. Its first status request races the head lookup: it passed in the preceding full gate, then failed once the missing head was counted before status. The new regression waits for the fake CLI to start and has no such ordering ambiguity.

Your round-3 approval explicitly permits exactly four tests, so it excludes this fifth fixture. A search of the daemon's remaining `Reply::Teams` assertions found no other instance of this non-Git fixture conflict: the others use real Git repositories or explicitly disabled history. Repository rule 1 requires permission before modifying the fifth test.

a) Authorize one dedicated `TEST-FIX:` adding `[history] enabled = false` only to the fixture of `a_socket_file_left_by_a_dead_daemon_is_replaced` — keeps its socket-replacement assertion unchanged and lets the new history regression cover the loss behavior.
b) Keep this fixture frozen — leaves the capture-loss fix blocked by an intermittent conflicting status expectation.

**Recommended:** a) — isolates the socket-replacement contract without changing shared fixture defaults or weakening any assertion. (investigator)

**Verdict:**
