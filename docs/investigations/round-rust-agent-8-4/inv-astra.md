# Investigation — Rust item 8.4 — `inv-astra`

Continued the stopped work of [#100](https://github.com/mateusz-szczyrzyca/team-agents/pull/100) on `agents/inv-astra/20261114-094819`. Item `8.4` is **not complete**. The human approved both round-5 recommendations. Secret redaction is committed and its complete Rust gate passes `2,317` tests. Bounded Git capture is implemented in the worktree but remains uncommitted: one newly frozen test advances the paused clock to the exact deadline without allowing Tokio's millisecond timer rounding. Round-6 permission asks to correct that fixture, preserving its assertions. The last fully GREEN code commit is linked below; the latest committed code is the next RED cycle.

## Completed fixes

- **Reader invariants:** SQL and the typed reader now agree about unused run-reason fields, nonrandom draw metadata and team-plan roles. Valid neighbouring cases remain accepted. No valid stored record or schema changed for this fix.
- **Sessions:** migration `2` adds append-only `run_sessions`; live parser observations are recorded in order with injected times and exposed by `Reader::sessions`. The supplied session remains in the launch record; echoing it does not create a new observation. A preserved version `1` fixture proves migration and old-data reading. A merged fake OpenCode round proves sessions learned during execution, including a changed session.
- **Startup failures:** failed opener tasks and writer-thread startup retain an unavailable history handle, report the cause and count subsequent records. They no longer silently behave as disabled history.
- **Writer diagnostics:** the binary held the stderr lock throughout daemon execution, blocking the writer's first warning and its subsequent progress. Passing stream handles instead of lifetime-long locks fixes that deadlock. A real daemon process now reports a rejected insert once, counts multiple losses and persists its run end.
- **Git extensions:** history snapshots disable configured clean/process filters and filesystem monitors through per-command Git configuration. They record raw worktree bytes without invoking those external commands or modifying repository configuration, the real index or the object database. Tests prove both filter and filesystem-monitor commands are not executed.
- **Failure proofs:** isolated history permission failure and rejected step inserts each allow a fake GitHub round to merge. A real daemon under queue pressure completes a brief, exposes losses and reports queue saturation once. A second test holds an external SQLite write transaction until the round is merged, proving that history drainage is not required for merge progress.

- **Capture losses:** failed head lookup counts one incomplete launch; failed Git changes capture counts one missing changes record. Each cause is reported once. An unavailable history skips the capture future and counts the omitted record; disabled history does neither.
- **Seeded selection:** production obtains OS entropy and passes the RNG into the daemon owner. A test supplies `8` seeds and compares the recorded coordinator, candidate order, member roles and state against the pure planning decision.
- **Crash atomicity:** the added child-process test waits for WAL growth inside a large transaction, kills the writer, and requires complete rollback of that batch, retention of the previous data, successful integrity checking and a later successful write. It does not accept a batch committed before the kill.

- **Known secrets:** the writer redacts configured GitHub, agent-environment and notification credentials before persistence and before text truncation. Overlapping Unicode values are removed together. Secret-bearing structured identities cause counted omissions and one safe warning. A merged fake-CLI round proves redacted prompts, arguments, steps, final messages and commit subjects, unchanged CLI input, and no marker bytes in the live WAL or checkpointed database.
- **Capture deadline, pending:** a shared absolute deadline covers all Git commands of each snapshot, including scratch handles and executable-busy retries. Each run boundary has a `2`-second budget and at most `1` additional second for process reaping. Existing capture-loss paths handle the resulting typed timeout. This implementation awaits the round-6 fixture correction and a GREEN gate; it is not yet committed.

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
- Capture losses $\color{red}{\textbf{RED}}$: [`57656608`](https://github.com/mateusz-szczyrzyca/team-agents/commit/57656608077a461217928f262d081c8b22d360db).

- Capture losses $\color{green}{\textbf{GREEN}}$: [`47d95511`](https://github.com/mateusz-szczyrzyca/team-agents/commit/47d955117e7320c1d9fd1666e86c08f4ac20a8da).
- Merged-round queue proof: [`19bbb24e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/19bbb24e9f2df16d5c85b8969c4f19143e2ad4db).
- Seeded selection $\color{red}{\textbf{RED}}$: [`81610900`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8161090081b6dd282f41127ff83e8e191f7be656).
- Seeded selection $\color{green}{\textbf{GREEN}}$: [`111cbc0a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/111cbc0ad741a956eabdd6d5051ac225ad295bfc).
- Active-batch kill proof: [`a038882c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a038882cc801f156c01da6ad665d8793be359be8).

- Secret redaction $\color{red}{\textbf{RED}}$: [`ee79f5dd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ee79f5dd1a7dd1c5216bd93d211edf66428b54b4).
- Secret redaction $\color{green}{\textbf{GREEN}}$: [`6c402ada`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6c402ada70d26cf453e429cb4bfecee18cc945e4).
- Capture deadline $\color{red}{\textbf{RED}}$: [`89561885`](https://github.com/mateusz-szczyrzyca/team-agents/commit/89561885b5f568a0f1168f01d602d82a9618d947).

## Tests changed after RED

- Human-authorized `TEST-FIX:` [`88dc7138`](https://github.com/mateusz-szczyrzyca/team-agents/commit/88dc71381c20c0b7e6320916c00c375df15a5dfd) updates [`docs/history.schema.sql`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a038882cc801f156c01da6ad665d8793be359be8/docs/history.schema.sql) and the table inventory in [`crates/agent-runtime/tests/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a038882cc801f156c01da6ad665d8793be359be8/crates/agent-runtime/tests/history/schema.rs) for session migration `2`.
- Human-authorized `TEST-FIX:` [`abc029dd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/abc029ddbcb7858205bf6266fb2621f49c8f9c23) advances the unsupported-version fixture in [`crates/agent/src/daemon/owner/rounds/tests/history_records.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a038882cc801f156c01da6ad665d8793be359be8/crates/agent/src/daemon/owner/rounds/tests/history_records.rs) to one beyond the latest migration. It preserves byte preservation, merged-round and unavailable-status assertions.
- Human-authorized `TEST-FIX:` [`ffaab8fc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ffaab8fc84c7fb8fd21b7466734ee869cf8bc9df) disables history only in the four round-3 fixtures. Its diff adds configuration writes inside those tests; shared fixture defaults and every assertion remain unchanged.
- The requested regression is $\color{red}{\textbf{RED}}$ in [`90c76d79`](https://github.com/mateusz-szczyrzyca/team-agents/commit/90c76d791958b044398076ffd3921dd9cb792202). Without the fix it receives `Teams`; with the committed fix it passes, asserting `Overview` with `Writing` and exactly `lost: 1`.
- Human-authorized `TEST-FIX:` [`2800a04d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2800a04def0efc54a6b67c4b55f9bad23d1fdea5) disables history only inside `a_socket_file_left_by_a_dead_daemon_is_replaced`. It adds `5` fixture lines and changes no assertion. Round-4 permission covers any future fixture with exactly the same non-Git-directory/`Reply::Teams` conflict; no further fixture required this exception.

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
| Secret values are not exported as environment fields | `no_value_of_a_secret_or_of_the_environment_of_the_clis_is_in_the_files_of_the_history` |
| Echoed known credentials are redacted before SQLite; DB and live WAL contain no marker | `a_merged_round_redacts_echoed_secrets_in_prompts_arguments_steps_messages_and_commits` |
| Seeded text, overlapping Unicode credentials, all activity payloads, safe identity omission/count/report | `seeded_free_text_is_redacted_before_any_store_sees_it`; `all_activity_text_fields_and_overlapping_unicode_values_are_redacted`; `secret_in_a_structured_identity_is_omitted_counted_and_reported_once` |
| Shared absolute capture budget, pending GREEN | `an_expired_snapshot_deadline_refuses_to_spawn_git`; `a_second_command_uses_the_snapshots_remaining_budget` pass with pending code; process-group test requires the round-6 clock correction |
| Directory and newer-schema failures permit merged rounds | `a_history_in_the_way_does_not_stop_a_round_and_status_shows_what_was_lost`; `a_history_of_a_newer_version_is_left_as_it_is_and_the_round_goes_on` |
| Permission and SQL write failures permit merged rounds | `read_only_history_storage_leaves_state_writable_and_the_round_merges`; `failing_step_inserts_are_counted_while_other_records_survive_and_the_round_merges` |
| History snapshots do not execute extensions | `history_reads_raw_worktree_bytes_without_running_configured_filters`; `history_reads_do_not_invoke_a_configured_filesystem_monitor` |
| Failed capture counts and unavailable-history capture skipping | `failed_head_and_change_capture_are_visible_as_losses_without_changing_the_run`; `unavailable_history_counts_the_skipped_capture_without_trying_the_invalid_repository`; `failed_git_snapshots_report_once_and_status_counts_each_missing_record` |
| Full queue still permits a merged round | `a_full_history_queue_does_not_prevent_a_round_from_merging` |
| Exact degraded status for a non-Git run directory | `status_of_a_run_outside_a_git_repository_reports_one_lost_capture` |
| Seeded daemon coordinator selection and recorded plan | `a_seeded_daemon_records_the_actual_coordinator_draw_and_candidates` |
| Killing a transaction after it has written uncommitted WAL frames | `killing_after_uncommitted_wal_frames_appear_rolls_back_the_entire_batch` |
| Documented inventory, runnable SQLite queries and normalized output | `every_table_column_and_index_of_the_schema_is_documented`; `the_queries_of_the_page_list_the_runs_of_a_round_print_a_prompt_whole_and_give_the_steps_and_files_of_a_run`; `what_the_queries_of_the_page_print_for_a_round_is_pinned` |

## Checks

- Earlier code gate at [`a038882c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a038882cc801f156c01da6ad665d8793be359be8): `make rust` passes formatting, clippy, `2,313` test executions and documentation; `0` failed, `0` ignored.
- Last complete GREEN gate at [`6c402ada`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6c402ada70d26cf453e429cb4bfecee18cc945e4): `make rust` passes formatting, clippy, `2,317` tests and documentation, with `0` failures and `0` ignored.
- Secret RED gate: `663` daemon tests passed; the new merged-round test failed because the persisted prompt retained the marker. The focused runtime suite failed all `3` new tests on raw text or an uncounted identity omission.
- Deadline RED gate: `664` daemon tests passed; `168` runtime library tests passed and all `3` new deadline tests failed against the forwarding stub.
- Pending deadline gate: formatting and clippy pass; `664/664` daemon tests and `170/171` runtime library tests pass. Only the frozen exact-clock fixture fails. No existing test was changed, no assertion was removed, and temporary diagnostic prints were removed.

- The fixture/capture gate passed `2,309` test executions after round-4 permission; the merged-queue gate passed `2,310`; the seeded gate passed `2,311`.
- Capture-loss $\color{red}{\textbf{RED}}$: new process test observed completed tasks without a loss count; runtime assertions observed `0` instead of `2` and `5` instead of `6` losses. The exact socket regression received `Teams` instead of `Overview`.
- Seeded-selection $\color{red}{\textbf{RED}}$: `662` daemon tests passed; only the new test failed, at seed `2`, with `inv-a` instead of `inv-b`. The incorrect injection stub was isolated from the normal entry point before committing RED; its first version also caused a duplicate run ID during restart and was not committed.
- Focused queue-merge, active-batch kill, runtime capture and daemon process suites passed under the repository test lock.
- `git diff --check` passes.
- `git grep -l 'CREATE TABLE' -- crates` finds one production Rust schema module plus the preserved version-1 migration SQL fixture. The fixture is deliberately a second path, not a second production schema module.
- The secret-echo experiment used only a temporary home and fake CLIs. A merged round produced `persisted_steps=1`, `in_wal=true`, `in_database=true`. The temporary diagnostic test asserted the reproduction, was removed afterwards, and is not presented as a security regression test or a fix. The final committed tests were not changed by the probe.
- Serena was used for symbol discovery. `sqlite3` is installed. No tool installation was needed.

## Scope and deviations

- The human approved a larger item instead of splitting it. Before this continuation's implementation, the planned ceiling was recorded as `7,500` counted lines. The previous PR-1 count including this report was `5,452` lines against actual PR target [`150e2203`](https://github.com/mateusz-szczyrzyca/team-agents/commit/150e2203fe0ebd458c7f6f67c5676ab736378b5d), excluding tests, the lockfile and schema snapshot.
- [`crates/agent/src/main.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a038882cc801f156c01da6ad665d8793be359be8/crates/agent/src/main.rs) is outside the item's named recording locations but its stream-lock fix is necessary for writer failure reporting and progress.
- The agent crate adds the already-approved bundled `rusqlite` workspace dependency for tests that provoke actual SQLite insert failures. Cargo regenerated the lockfile. No new library version or final lint suppression was introduced. The seeded RED commit carried a temporary dead-code expectation for its compiling stub; GREEN removed it.
- Session schema compatibility, session queries and raw snapshot semantics are documented in [`docs/history.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a038882cc801f156c01da6ad665d8793be359be8/docs/history.md); the reader fix adds the history reference to [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a038882cc801f156c01da6ad665d8793be359be8/README.md).
- External process tests reuse real operating-system polling and bounded wall-clock deadlines for process completion; the added kill test uses `yield_now` while waiting for WAL growth. This deviates from strict `TDD-8`; policy-time tests use the injected clock, and no new assertion measures elapsed execution time. The new process fixture kills and reaps its daemon on failure.
- The inherited unrelated `3`-line Bash change in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a038882cc801f156c01da6ad665d8793be359be8/bin/agent) was not altered or tested here.
- No new PR was opened, no later plan item was started, and no live installation, real model CLI or hardware device was used.

## Skipped tests

- Go suites were not run, as explicitly instructed.
- Bats suites were not run, as explicitly instructed.
- `make gate` was replaced by `make rust` to obey those exclusions.
- No Rust test was newly ignored or removed. The secret-redaction gate passed every stage; the next deadline cycle remains RED as described above.

## Not done

- **Capture boundary:** the implementation and documentation are prepared in this worktree, but the process-group regression's frozen injected-clock fixture needs the round-6 correction before GREEN can be committed. Production currently committed at the RED head still has the forwarding deadline stub.
- **Inherited process-test precision:** two older process tests look for `solo` and `done` anywhere in status; the summary header can contain `0 done`. New tests check an actual solo task line ending in `done`; the older frozen predicates remain unchanged.
- The behaviour/test map above covers the principal requirements and this continuation's changes. A complete per-sentence acceptance audit and final checkpoint remain outstanding; this report does not represent item `8.4` as finished.

## Secret reproduction

The isolated configuration supplied `HISTORY_TEST_SECRET` through the secrets environment map. The fake CLI emitted this event using that variable, committed a work file and let the fake reviewers approve and merge the round:

```bash
printf '{"type":"assistant","message":{"content":[{"type":"text","text":"%s"}]}}\n' "$HISTORY_TEST_SECRET"
```

A read-only equivalent of the probe's query finds the persisted event:

```sql
SELECT count(*) FROM steps
WHERE text LIKE '%history-echo-secret-marker-84%';
```

Before the fix the result was `1`, and both live WAL and checkpointed database contained the marker. The committed regression now requires redacted typed records and the absence of those marker bytes in both files. Only explicitly synthetic credentials were used; no real secret was read or printed.

## Answered questions — round 3

The external question file is `questions-3.md` for task `20261114-094819-inv-astra`. The question is retained below as the approval record. The human approved option `a` for exactly four tests, in one fixture-only `TEST-FIX`, and required the new exact-loss regression. The original pause was required by [`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a038882cc801f156c01da6ad665d8793be359be8/AGENTS.md), rule `1`: “Never modify existing tests.” The earlier approvals cover schema fixtures, not these socket tests.

## 1. May four socket-test fixtures disable history explicitly?
The capture-loss fix counts a failed Git head lookup and exposes it through `agent status`, as item `8.4` requires. Four existing socket tests use directories that are not Git repositories and expect `Reply::Teams`. They now receive `Reply::Overview` with `HistoryHealth { state: Writing, lost: 1 }`. In [`crates/agent/src/daemon/tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a038882cc801f156c01da6ad665d8793be359be8/crates/agent/src/daemon/tests.rs), the affected tests are:

- `status_over_the_socket_lists_the_teams_with_their_tasks`
- `status_of_a_team_or_a_task_names_only_its_team_and_an_unknown_one_is_not_found`
- `a_line_that_is_not_a_request_is_answered_with_an_error_and_the_connection_goes_on`
- `a_request_of_a_subscriber_that_arrives_in_parts_around_an_event_is_answered`

With the pending implementation, `make rust` passes formatting and clippy, then reports `656/660` daemon tests passing. Focused process and run-history suites pass `16/16`. Repository rule 1 says “Never modify existing tests”; the earlier permissions do not cover these four fixtures. The proposed change explicitly disables history only for these protocol tests before daemon startup, preserving their existing assertions and leaving shared fixture defaults unchanged. Dedicated history tests continue to prove degraded status and loss counts.

a) Authorize a dedicated `TEST-FIX:` adding `[history] enabled = false` to these four individual fixtures — preserves their protocol contracts while allowing failed history captures to be visible in status.
b) Keep these fixtures frozen — leaves the capture-loss fix blocked because its required status behavior conflicts with their assumptions.

**Recommended:** a) — isolates socket protocol tests from optional history recording without weakening their assertions or the history tests. (investigator)

**Verdict:** a) — approved for exactly the four named fixtures, with a new exact-loss status regression.

## Answered questions — round 4

The human approved a fixture-only `TEST-FIX` for `a_socket_file_left_by_a_dead_daemon_is_replaced`, with every assertion preserved, and extended that permission to any other test failing solely because it starts in a non-Git directory and expects `Reply::Teams`. Applied in the commit listed above. No additional fixture changes were needed.

## Answered questions — round 5

The external question file for task `20261114-094819-inv-astra` is `questions-5.md`. The two questions are reproduced below. The human approved recommendation `a` for both, explicitly authorizing bounded capture delay and known-secret redaction.

## 1. May accurate Git history capture briefly delay a run at its boundaries?
The committed fixes skip history-only Git work when history is unavailable and count capture failures. A saturated writer queue now has a passing merged-round proof. With healthy history, however, the engine still awaits the starting head before spawning the CLI and awaits the final Git snapshot before completing the run. The snapshot includes uncommitted files, which a following run can overwrite. Each local Git command currently has a `120`-second timeout. Taking these snapshots in the background without a boundary would risk attributing a later run's changes to the earlier run. Item `8.4` requires both accurate changes and history that “never stops or delays the work”; these imply different designs at this boundary.
a) Permit a bounded best-effort snapshot barrier — add a short overall capture deadline with cancellation and counted loss on timeout; keep exact attribution for successful captures and document that storage writes never block work, while capture can briefly delay that run.
b) Require no awaited history capture — allow changes to be absent when a stable snapshot cannot be obtained without holding up the next run; count and report those omissions instead of attributing uncertain changes.

**Recommended:** a) — preserves accurate before/after attribution and makes the delay bounded and explicit; it needs an explicit exception to the literal no-delay wording. (investigator)

**Verdict:** a) — human approved the recommendation.

## 2. How should history handle a known secret echoed by a CLI?
An isolated fake-CLI round completed and merged after its script printed the value of an environment variable supplied through `secrets.toml`. The synthetic marker appeared in exactly `1` stored step, in the live WAL, and in `history.db` after checkpointing. The existing secret test passes because its CLI never echoes the injected value. Item `8.4` asks for whole prompts, command lines, final messages and exact steps, while its ground rule says no secret reaches history; secret-bearing output needs an explicit exception to exact recording.
a) Redact known secret values before persistence — replace occurrences with an explicit redaction marker in captured text, including prompts, arguments, steps and final messages; add daemon tests that search both database and WAL bytes. If redaction would invalidate structured identifiers, omit and count that record instead.
b) Omit any record containing a known secret — preserve the exact bytes of retained records, count the omissions and report their cause without printing the secret; less context is available for diagnosis.

**Recommended:** a) — retains useful history while preventing the reproduced disclosure; explicitly permits redacted text to differ from what the CLI received or emitted. (investigator)

**Verdict:** a) — human approved the recommendation.

## Questions — round 6

The external question file is `questions-6.md` for task `20261114-094819-inv-astra`. The permission is required by repository rule `1`, “Never modify existing tests,” and the frozen-RED contract. The previous blanket fixture permission applies only to non-Git directories expecting `Reply::Teams`, so it does not cover this new timing test.

## 1. May the new deadline test advance past Tokio's timer resolution?
The approved secret-redaction fix is committed and passes the complete Rust gate: `2,317` tests, `0` failures, `0` ignored. The bounded-capture implementation is prepared: one absolute `2`-second budget per run boundary, shared by every Git command and startup retry, process-group cancellation on timeout, at most `1` further second for reaping, and existing counted-loss reporting.

I made a mistake in the new RED test `a_snapshot_deadline_kills_git_and_its_descendant_then_returns` in [`crates/agent-runtime/src/git/deadline_tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/89561885b5f568a0f1168f01d602d82a9618d947/crates/agent-runtime/src/git/deadline_tests.rs): it advances the paused clock by exactly `2` seconds and then keeps the runtime runnable while waiting for the process to end. Tokio rounds timer deadlines up to the next millisecond. A diagnostic run confirmed that the timeout fires at `2.001` seconds, after the test has already decided it missed the deadline. The pinned Tokio source confirms that rounding. This is a test-clock fixture error, not a reason to alter production deadlines to fit the test.

With the pending implementation, `make rust` passes formatting and clippy; all `664` daemon tests and `170/171` runtime library tests pass. Only this new frozen test fails. The other deadline tests prove that an expired budget starts no process and that later commands share the remaining budget. Repository rule 1 and TDD's frozen-RED rule require permission to correct the test I committed.

a) Authorize a dedicated `TEST-FIX:` advancing this test's clock by `2,001` milliseconds instead of `2` seconds, with a comment explaining timer resolution — preserves every assertion, including the `2`-second configured limit, process completion and termination of both Git and its descendant; list it under “Tests changed after RED”.
b) Keep this test frozen — leaves the bounded-capture fix unfinished because the test does not let the runtime deliver its timer.

**Recommended:** a) — corrects only the injected-clock fixture and avoids changing production timing to satisfy an overprecise test. (investigator)

**Verdict:**
