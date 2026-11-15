# Round rust-agent/8.7 — inv-astra

## Status

`QUESTIONS`: implementation is paused at the existing-test conflict required by `AGENTS.md`. This report is not an implementation of round `8.7` and must not be treated as a completed round.

## Blocking contract

The existing test `log_refuses_no_subject_two_subjects_and_a_name_that_is_not_an_id_with_status_2` in [`crates/agent/src/tests/log_command.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/crates/agent/src/tests/log_command.rs#L73-L99) requires:

- Bare `agent log` to fail parsing with `MissingRequiredArgument` and exit `2`.
- Combined `--round` and `--team` to fail parsing with `ArgumentConflict` and exit `2`.
- Invalid identifiers to fail validation; this remains appropriate for round `8.7`.

The first two cases conflict directly with the requested default daemon log output and combinable filters. The parser enforces the same restriction through its required, mutually exclusive argument group in [`crates/agent/src/main.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/crates/agent/src/main.rs#L205-L224).

[`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/AGENTS.md#L51-L55), Never rule `1`, says: “If the task requires behaviour that an existing test forbids, stop and ask the human in your questions file; wait for the answer.” The explicit permission in the round to migrate activity-file tests does not mention this CLI test.

## Question

The harness question is in `questions-1.md` for task `20261115-212152-inv-astra`.

Authorize replacing the two obsolete rejection cases with success coverage for default logging and intersected team/round filters, keeping the invalid-ID checks. With that answer, resume implementation of the full item on `agents/inv-astra/20261115-212152`.

## Acceptance

- `A1`: not met; `--show` has not been implemented.
- `A2`: not met; search and following have not been implemented.
- `A3`: not met; retention has not been implemented.

## Checks

- The targeted existing-contract test passed: `1` passed, `0` failed, `0` ignored, `755` filtered out.
- Command: `tools/test-lock.sh inv-astra-log-contract cargo test -p agent --bin agent log_refuses_no_subject_two_subjects_and_a_name_that_is_not_an_id_with_status_2 --locked -- --exact log_command_tests::log_refuses_no_subject_two_subjects_and_a_name_that_is_not_an_id_with_status_2`.
- The required `make gate` is run after committing this report; its result is reported in the final response.
- No production code or existing tests were changed.

## Skipped tests

None added or modified.

## Not done

All implementation and new behavior tests for round `8.7` await the answer. No pull request was opened. No other agent's worktree was inspected, and no agent CLI was run.
