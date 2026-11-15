# Round rust-agent/8.7 — inv-astra

## Status

`QUESTIONS`: the full implementation is preserved locally. Its final gate failed two existing tests that assume asynchronous history failures are visible immediately after a round merges. Question `5` requests permission to synchronize those tests. This report is pushed on top of the previously published query slice; the code whose gate failed is not pushed.

The fourth answer was applied: the two remaining final-schema expectations now use the pinned schema version, in a separate `TEST-FIX:` commit. No additional schema-test permission is needed.

## Blocking tests

- `failing_step_inserts_are_counted_while_other_records_survive_and_the_round_merges`
- `sqlite_full_does_not_prevent_merging_or_the_other_records_surviving`

Both are unchanged existing tests in [`crates/agent/src/daemon/owner/rounds/tests/history_failures.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/crates/agent/src/daemon/owner/rounds/tests/history_failures.rs#L54). Their shared `merge_and_status` helper waits for a merged round, then makes one status request and immediately requires `Reply::Overview`. Under the parallel suite it receives `Reply::Teams`: history has not reported a loss yet. The writer reports the injected step error afterwards.

The failure is at the reply-variant assertion, before either test reaches its loss-count and saved-record assertions. The actual reply shows a closed, merged round and all three agents done. The daemon's work succeeded; the tests raced the independent history writer.

[`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/AGENTS.md), Never rule `1`, says: “If the task requires behaviour that an existing test forbids, stop and ask the human in your questions file; wait for the answer.” Earlier permissions do not cover this helper or these fault-injection tests. Neither was edited.

## Question

The full question is in task `20261115-212152-inv-astra`, `questions-5.md`, with an empty verdict.

- Recommended `a`: a tests-only `TEST-FIX:` correction waits for an observable status condition after proving the round merged; it preserves all existing assertions about health, lost records, completed runs, empty steps and integrity. No arbitrary sleeps or production blocking are introduced. The full-queue test must still merge while its external database lock is held.
- Option `b`: keep the tests unchanged; retain the implementation locally pending another permitted resolution.

## Checks on the full local implementation

- `AGENT_GATE_RUN=1 GATE_WAIT=1 make gate`: formatting and clippy passed; agent binary tests reported `764` passed and `2` failed. The gate stopped there; this is not a green result.
- `tools/test-lock.sh inv-astra-gate-diagnosis cargo test -p agent --bin agent --locked -- --nocapture --skip every_log_event_variant_is_produced_with_its_level_and_ids`: `763` passed, `2` failed, `1` filtered out. Excluding the test that installs a global panic hook made the two exact assertion failures visible. This was a diagnostic run, not a replacement gate.
- `tools/test-lock.sh inv-astra-failure-diagnostics cargo test -p agent --bin agent history_failures_tests --locked -- --nocapture`: `7/7` passed in isolation. The passing retry does not invalidate the parallel failure.
- Complete runtime history suite before the final two queue tests: `118/118` passed.
- Retention tests after adding those queue tests: `8/8` passed.
- Actual daemon acceptance tests for filters, views, search and daily retention passed in the failed gate.
- The process test following actual run starts across daemon kill, restart and retention passed its targeted run.
- The complete gate on the published report commit is run separately; its result is stated in the final response and applies only to that published query slice.

`AGENT_GATE_RUN=1` keeps the repository gate local instead of forwarding it to the running Bash harness. The repository's test lock remains enabled.

## Implemented locally, awaiting the test decision

- Default newest daemon logs; all `9` filters and all `36` pairs; exact JSON lines, bounds and empty results.
- Preserved scoped-story behavior from the second answer; explicit focused views for all `9` subjects.
- Literal FTS5 search of prompts, steps, final messages, review reports, questions, answers and log messages; Polish and operator-character cases.
- Following with persistent monotonic cursors through daemon restart and deletion.
- Startup and daily retention through the sole writer; closed/open/boundary cases; gate-output deletion, shared-round ownership and atomic database snapshots.
- Full auto-vacuum, measured main-file shrink, statistics and legacy readers.
- Schema `6`, with populated version `4` migration and search coverage.
- Removal of Rust activity-file writes and migration of their readers and permitted tests.
- Daemon-generated acceptance coverage, normalized golden output, secret-marker checks and indexed query plans with `100,000` steps.
- Configuration, help and user documentation.

The complete implementation, its commit history and detailed handover report are retained by the local stash named `inv-astra 20261115-212152 complete implementation awaiting questions-5`. Its first parent is the committed implementation; its working-tree payload adds the gate-blocker findings to the detailed report. Restoring only the stash's patch onto this published query slice would omit the implementation: resume from that first parent and reconcile this report commit. Do not drop the stash before recovery is verified.

## Published code

The published branch still contains the earlier query slice, whose production head is [`db36b50b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/db36b50b58c8b2ed5e506d7089fce15dc75f3282): default log rows, intersected filters, bounded pages, exact JSON and preservation of scoped stories. Later commits on the published branch are clarification reports only.

## Acceptance

- `A1`: met by the preserved local implementation's daemon test; not complete in the published slice.
- `A2`: met by preserved local search and process-follow tests; not complete in the published slice.
- `A3`: met by the preserved local daemon retention test with `keep_days = 1` and a `2`-day clock advance; not complete in the published slice.

## Skipped tests

No test was marked ignored or removed. The single diagnostic command above filtered out the global-panic-hook test to expose failure output; the actual gate filtered out none. No lint suppression or dependency was added. No required tool was missing.

## Not done

- Obtain permission for the existing asynchronous-status test correction.
- Restore the preserved implementation, apply only the authorized correction and run the full gate on the final commit.
- Push the completed, verified implementation and final report.

No PR was opened. No other agent's worktree was inspected. No real model CLI, live installation action or old-implementation suite was run.
