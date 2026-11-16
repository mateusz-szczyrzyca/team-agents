# Item 8.8 — `inv-astra`

## Result

Implemented item `8.8` on `agents/inv-astra/20261116-015620`. Claude turn exhaustion is recorded as `turn_limit`, queues bounded continuation in the same session, and preserves the worktree. Defaults are `300` turns per invocation and `3` consecutive automatic resumes. Exhausting the resume budget fails the task and sends one notification with the count. Quota exhaustion takes precedence.

The count comes from persisted run endings, so another end resets it and a restart retains it. The resumed prompt is short and stored in the prompt catalog. The daemon records both the end and the continuation reason; text and JSON history expose them. No later plan item is implemented.

Implementation and documentation: [`b8c4085c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914).

## Acceptance

| Criterion | Result | Evidence |
|---|---|---|
| `A1` | met | `turn_limit_resumes_same_session_then_done_with_history_and_short_prompt`: a real daemon runs the fake CLI twice, reads `TurnLimit` then `Reported(Done)` through `Reader`, and polls the same socket status API used by `agent status`, refusing every observed `Failed` status. |
| `A2` | met | `configured_max_turns_reaches_the_cli_process`: configuration sets `max_turns = 50`; the fake process records `--max-turns` and `50` as consecutive arguments. |

Both acceptance tests are in [`crates/agent/src/daemon/owner/rounds/tests/turn_limits.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent/src/daemon/owner/rounds/tests/turn_limits.rs). They failed before daemon implementation in the RED commit and pass with the implementation. The test observes status through the public socket API; it does not spawn the status subcommand separately.

## Tests

The behavior table covers the item and its nearest negative cases. Tests use temporary homes, fake CLIs, a local fake GitHub server, and injected time. Database records inserted directly by history tests prove codec/migration invariants; daemon tests prove recording.

| Behavior | Test |
|---|---|
| Recorded Claude `error_max_turns` is distinct, with no final message and preserved activity/session | `a_run_that_hit_its_turn_limit_has_a_distinct_outcome_and_no_final_message` |
| `error_during_execution` remains failure; successful `STATUS:` handling remains unchanged | Existing Claude fixture tests, including `a_result_that_reports_an_error_failed_and_keeps_its_text`; the complete Claude adapter tests passed. |
| Defaults `300` / `3`, positive maximum turns, zero resumes, invalid and maximum inputs | [`crates/agent-core/tests/config_turn_limits.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/tests/config_turn_limits.rs) |
| First, last allowed, first refused continuation, zero and maximum budgets, no session | `first_last_allowed_and_first_refused_resumes_follow_configuration` |
| Every other ending resets the consecutive count | `persisted_runs_reset_the_count_after_every_other_end` |
| Quota wins even with a final status; a human stop wins | `turn_limit_is_distinct_and_quota_takes_precedence` |
| Quota plus `error_max_turns` pauses in a real daemon and records quota, without a resume | `quota_in_a_turn_limited_run_pauses_instead_of_spending_a_resume` |
| Queued continuation, same session, exact short text, then normal completion | `turn_limit_resumes_same_session_then_done_with_history_and_short_prompt` |
| A resumed `STATUS: FAILED` still fails normally | `resumed_prompt_matches_its_golden_and_a_resumed_failure_still_fails` |
| With `turn_limit_resumes = 1`, second exhaustion fails and notifies once with both counts | `one_resume_allows_two_turn_limits_then_fails_and_notifies_once` |
| Zero resumes fails immediately; notification text is pinned; restart does not repeat it | `zero_resumes_notifies_with_the_exact_exhaustion_text_and_restart_does_not_repeat_it` |
| Configured maximum reaches the process | `configured_max_turns_reaches_the_cli_process` |
| New/resumed Claude commands carry boundary budgets; other CLI commands remain unchanged | [`crates/agent-runtime/tests/turn_limits.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-runtime/tests/turn_limits.rs) |
| Restart after persisted end and queue resumes once, retaining the budget | `restart_after_the_saved_turn_limit_resumes_once_and_keeps_the_budget`, with resumed success and repeated exhaustion cases |
| End, continuation reason, same session and round links are recorded | Primary daemon acceptance test and `a_turn_limited_round_continues_through_both_reviews_and_merge` |
| Both a solo agent and a reviewer resume within a complete merged round | `a_turn_limited_round_continues_through_both_reviews_and_merge` |
| Recorded history renders `turn_limit` in JSON and the new human text | Same full-round test; exact relevant text lines are pinned in [`crates/agent/tests/fixtures/turn-limit/story.txt`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent/tests/fixtures/turn-limit/story.txt). |
| Prompt and notification wording | [`crates/agent/tests/fixtures/turn-limit/resume.txt`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent/tests/fixtures/turn-limit/resume.txt) and [`crates/agent/tests/fixtures/turn-limit/exhausted.txt`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent/tests/fixtures/turn-limit/exhausted.txt) |
| Secret marker absent from database and journals after a full round | Full-round test scans every history database file for the configured fake GitHub token marker. |
| An unwritable history path or newer schema does not stop continuation; loss is reported | `turn_limit_continuation_survives_an_unwritable_or_newer_history_and_reports_loss`; newer database bytes remain identical. |
| Generated state/protocol round trips and consecutive-count invariant | `seeded_turn_limit_runs_round_trip_and_count_only_the_consecutive_suffix`, seeds `0`–`99`, with seed in assertion failures |
| Cross-variant fields rejected; historical state still reads | `turn_limit_rejects_fields_of_other_endings_and_old_states_remain_readable` |
| Version `3` rejects version `2` before reading payloads | [`crates/agent-core/tests/protocol_v3.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/tests/protocol_v3.rs), plus retained version `1` rejection and new version `3` fixtures |
| History start/end round trip | `turn_limit_end_and_continuation_reason_round_trip`, seeds `0`–`19` |
| Populated history `6` → `7` preserves records, steps, changes, search and foreign keys | `version_six_upgrade_keeps_records_links_search_and_foreign_keys`, seeds `0`–`9` |
| SQL and reader reject irrelevant fields on the new end/reason | `turn_limit_fields_of_other_variants_are_refused_by_sql_and_reader`, `7` fields |

Supplementary daemon tests are in [`crates/agent/src/daemon/owner/rounds/tests/turn_limit_round.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent/src/daemon/owner/rounds/tests/turn_limit_round.rs) and [`crates/agent/src/daemon/owner/rounds/tests/turn_limit_restart.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent/src/daemon/owner/rounds/tests/turn_limit_restart.rs). The restart test stops the real owner at the persisted settlement/scheduling boundary, drops it, and boots the daemon from that state; it does not rely on a timed process kill. The generated tests use seeded input. No sleeps advance behavioral time.

## Checks

- Targeted adapter, configuration, classification, policy, protocol, history and daemon tests passed during their respective cycles.
- `tools/test-lock.sh turn-limit-coverage cargo test --workspace turn_limit --locked`: passed before the final supplementary quota and story assertions; those additions subsequently passed their targeted tests.
- `tools/test-lock.sh turn-limit-quota cargo test -p agent --bin agent quota_in_a_turn_limited_run --locked`: `1/1` passed.
- `tools/test-lock.sh turn-limit-story cargo test -p agent --bin agent a_turn_limited_round --locked`: `1/1` passed, including story JSON/text and secret-marker checks.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: passed with `0` findings after all code and test changes.
- `cargo fmt --all` and `git diff --check`: passed.
- Test-freeze audit: no deleted test lines after any RED commit; additions only.
- Final handoff check: `GATE_WAIT=1 make gate` is run after this report is committed. The final handoff records its observed result and the exact checked commit; this report does not claim that pending check has already run.

## Mutation checks

Each mutation was temporary and restored before committing. All three commands failed on the intended regression assertion, exit `101`:

| Deliberately broken behavior | Failure |
|---|---|
| Classify turn exhaustion as `Hung` | `turn_limit_is_distinct_and_quota_takes_precedence`: actual `hung`, expected `turn_limit` |
| Launch with default `300` instead of configured `50` | `configured_max_turns_reaches_the_cli_process`: expected configured argument pair absent |
| Record generic `Continuation` instead of `TurnLimit` | Primary daemon history test: actual `Continuation`, expected `TurnLimit` |

## Compatibility and scope

- Protocol becomes `3`: old peers cannot decode the new run-end variant. Clients and daemon must upgrade together. Version `1` and `2` fixtures remain untouched.
- State remains schema `2` under the documented rule: existing state files still satisfy the contract. No counter field is added. Older binaries cannot read newly recorded ends, so downgrade is unsupported.
- History migration `7` rebuilds the closed keyword constraints transactionally. Shipped migrations `1`–`6` remain unchanged. The schema reference was regenerated through the history reader, not hand-edited.
- Beyond the expected file list, [`crates/agent-core/src/history.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/src/history.rs), [`crates/agent-core/src/story/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/src/story), and [`crates/agent-runtime/src/history/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-runtime/src/history) carry the required recorded end/reason and migration.
- [`crates/agent-core/src/protocol.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/src/protocol.rs) and protocol fixtures implement the required version policy.
- [`crates/agent-core/src/prompt/harness.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/src/prompt/harness.rs) and [`crates/agent-runtime/src/run/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-runtime/src/run) connect catalog templates and configured launch budgets to the daemon.
- [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/README.md) documents user-visible behavior; [`docs/history.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/docs/history.md) and [`docs/history.schema.sql`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/docs/history.schema.sql) document the migrated format. Test files and this report supply the required evidence.
- No dependency, toolchain, lint configuration, CI, protected instruction file, or old implementation change.

## Existing tests changed with authorization

- Clarification round `1`, answer `a`: the old Claude exhaustion test was renamed and its outcome changed from `Failed` to `TurnLimit`; its final-message, session and activity assertions remain.
- Clarification round `2`, answer `a`: protocol-version-only edits in [`crates/agent-core/src/protocol.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/src/protocol.rs), [`crates/agent-core/src/protocol/event.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/src/protocol/event.rs), [`crates/agent-core/src/protocol/reply.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/src/protocol/reply.rs), [`crates/agent-core/tests/protocol.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/tests/protocol.rs), [`crates/agent-core/tests/protocol_rounds.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/tests/protocol_rounds.rs), [`crates/agent-core/tests/public_client.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/tests/public_client.rs), [`crates/agent-core/tests/history_health.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/tests/history_health.rs), and [`crates/agent/src/client/tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent/src/client/tests.rs). Non-version assertions remain.
- Clarification round `3`, answer `a`: the future-version test in [`crates/agent-runtime/tests/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-runtime/tests/history/schema.rs) now tests `8` instead of `7`. Byte-for-byte, typed-error and no-journal assertions remain.
- Existing exhaustive matches in [`crates/agent-core/tests/outcome.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/tests/outcome.rs) received additive `TurnLimit` arms; no previous branch or assertion changed. Existing daemon rig and CLI fixture received only new helper/action behavior.

## Tests changed after RED

None. No `TEST-FIX:` commits. Supplementary coverage was added after the original RED/GREEN cycles without editing their assertions.

## Deviations

- The adapter-budget RED commit [`18fe46a6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/18fe46a6a5e0f0b6a5f7d05cf2a3f73a59c669db) identifies RED in its body but omits the required subject prefix. The regression failed before implementation; history was not rewritten.
- The history RED commit [`82267458`](https://github.com/mateusz-szczyrzyca/team-agents/commit/822674588f796948cf3e21ec102b8c0b0d9d979f) includes the no-payload `RunWhy::TurnLimit` arm in [`crates/agent/src/daemon/owner/history.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent/src/daemon/owner/history.rs) so the entire workspace compiles at RED and fails only on the two new history assertions.
- While developing the uncommitted supplementary quota test, its new fixture action was initially absent. That test failed with fake exit `64`; adding the action made it pass. This was a corrected fixture setup, not an unchanged flaky rerun.
- No new lint suppressions, unsafe code, ignored tests or dependencies. The positive default uses a compile-time checked `NonZeroU32` constant, following the existing configuration convention; it introduces no runtime panic path.

## Size

`550` changed hand-written lines outside tests, generated schema and fixtures (PR-1 counting, limit `5,000`).

## Skipped tests

None added or changed. The old Bash/Go suites and real model CLIs were not run, as instructed. Hardware was not used. No missing tool blocked this work.

## Not done

- No pull request, as instructed; the designated merger will integrate this branch.
- No later plan item, cutover, live installation change, or hardware work.

## Task

The full round item follows, quoted from [`rounds/round-8.8.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/rounds/round-8.8.md).

> 8. Autopilot, rounds, the history, checkpoints and accounts
> Depends on: 7 (merged before this round)
> [ ] 8.8 A run that reaches its turn limit goes on
>     A model CLI may end a run because a budget of its own ran out, not because the work failed: Claude Code ends
>     with `error_max_turns` when a run has used its `--max-turns`. Until now such a run has no `STATUS:` line, so its
>     task is `failed` and the work stays in the worktree with nobody to go on with it — also when the agent had
>     finished and only its last message was missing. It becomes an end of its own, `turn_limit` (a `RunEnd`, in the
>     state, the protocol and the history): the task is queued and resumed in its session with a short text that says
>     what happened and asks it to go on from where it was. That happens at most `[limits] turn_limit_resumes` times
>     in a row (default 3; a run that ends in any other way starts the count again); then the task is `failed` and
>     the human is told, with the count. The limit is no longer a constant of the adapter: `[limits] max_turns`
>     (default 300) is given to every CLI that has such a limit; an adapter whose CLI has none never reports this
>     end. The versions of the state and of the protocol follow the rules of [`docs/state.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/docs/state.md).
>     Expected result:
>     - Files: [`crates/agent-runtime/src/cli/claude.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-runtime/src/cli/claude.rs), the stream's outcome in [`crates/agent-core/src/cli.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/src/cli.rs),
>       [`crates/agent-core/src/decide/outcome.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/src/decide/outcome.rs), [`crates/agent-core/src/state/run.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/src/state/run.rs), the settling of a run in
>       [`crates/agent/src/daemon/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent/src/daemon), `[limits]` in [`crates/agent-core/src/config.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/crates/agent-core/src/config.rs), [`config/agents.example.toml`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/config/agents.example.toml),
>       the text in [`prompts/harness/run/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/prompts/harness/run), [`docs/architecture.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/docs/architecture.md), [`docs/configuration.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/docs/configuration.md), [`docs/state.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b8c4085c4dde1f6dd1f2e2a3d601529ebe438914/docs/state.md).
>     - Adds: the end `turn_limit`; its place in the table of what a run's end does; the count and its limit;
>       `max_turns` and `turn_limit_resumes`; the text of the resumed run.
>     - Tests: the ground rule on testing these items applies. The adapter: recorded output of Claude Code that ends
>       with `error_max_turns` is this end; one that ends with `error_during_execution` is not; a `success` whose
>       message has a `STATUS:` line is unchanged. The decision: a table — the first, the last allowed and the first
>       refused resume, the count starting again after another end, a quota that runs out in the same run (the quota
>       wins). The daemon with a fake CLI that ends at its turn limit: the task is queued and its next run resumes the
>       same session with the text; a fake CLI that finishes on the resumed run ends the task as it would have; with
>       `turn_limit_resumes = 1` the second such end fails the task and notifies the human once; the command line of
>       the fake CLI carries `max_turns` of the configuration; a daemon restarted between the end and the resume
>       resumes once. The history holds the end and the resumed run with why it started.
>     - Why: a run that used up its turns is not a failure; treated as one it loses finished work, as happened with the
>       old harness on item 8.3.
>     Checks:
>     - A1. With a fake CLI that prints a `result` of `error_max_turns` on its first run and `STATUS: DONE` on its
>       second: the task ends `done` after two runs and `agent status` never shows it `failed`.
>     - A2. With `[limits] max_turns = 50` the fake CLI is started with `--max-turns 50`.
