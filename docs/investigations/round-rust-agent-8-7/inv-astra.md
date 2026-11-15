# Round rust-agent/8.7 — inv-astra

## Status

`QUESTIONS`: the first answer permits replacing the two obsolete parser cases and requires more edge cases. A separate output-contract decision is pending in `questions-2.md` for task `20261115-212152-inv-astra`. This is not an implementation of round `8.7`.

## Authorized changes

- Bare `agent log` must succeed and print recent daemon logs.
- Combined `--round` and `--team` must be accepted and intersect their filters.
- Invalid-ID checks must remain.
- Replacement coverage must include additional edge cases.

## Blocking output contract

The existing scoped CLI tests require a narrative rather than daemon log rows:

- `a_round_is_told_from_its_item_to_its_merge_without_a_daemon` requires item text, prompts, steps, changes, reviews and merge output from `agent log --round ID`.
- `a_team_is_told_round_by_round_in_order` requires the story of every round from `agent log --team ID`.
- `json_prints_every_record_of_the_round_once_and_the_flags_do_not_change_it` requires every history record, including steps and `pull_merged`, from `agent log --round ID --json`.
- Source: [`crates/agent/src/tests/log_command.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/crates/agent/src/tests/log_command.rs#L107-L233).
- The process tests independently pin the narrative output and repeat the obsolete bare-command rejection: [`crates/agent/tests/log_process.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/crates/agent/tests/log_process.rs#L44-L122).

Round `8.7` describes the scope options as filters on daemon log rows and requires each filter alone to agree with the complete JSON output. Keeping a narrative default only when one scope filter is present would change the output category along with the filter.

[`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ad204aa2448806656e45845ff1452be284a104a6/AGENTS.md#L51-L55), Never rule `1`, says: “If the task requires behaviour that an existing test forbids, stop and ask the human in your questions file; wait for the answer.” The first answer specifically authorized the two parser cases; the narrative and JSON output assertions are a separate contract.

## Question

Should scoped `agent log` default to daemon log rows, replacing the existing story-output contract?

- Recommended option `a`: use daemon log rows consistently with and without filters; authorize migrating the existing CLI story-output assertions to precise filtered-log and `--show` coverage. Preserve invalid-ID, error handling, direct-reading and old-schema coverage, and add edge cases. Migrate the duplicate bare-command assertion too.
- Option `b`: retain the narrative default for a lone scope filter; require `--show log` for scoped daemon log rows and apply the round's filter checks in that mode.

The question is written to the harness's `questions-2.md`; its verdict is pending.

## Acceptance

- `A1`: not met; `--show` has not been implemented.
- `A2`: not met; search and following have not been implemented.
- `A3`: not met; retention has not been implemented.

## Checks

- `tools/test-lock.sh inv-astra-log-output-contract cargo test -p agent --bin agent log_command_tests --locked`: `11` passed, `0` failed, `0` ignored, `745` filtered out. This confirms the existing contracts; it does not validate round `8.7`.
- The previous report commit passed `make gate`.
- The required `make gate` is run after committing this report; its result is reported in the final response.
- No production code or existing tests were changed.

## Skipped tests

None added or modified.

## Not done

All implementation and new behavior tests for round `8.7` remain pending. No pull request was opened. No other agent's worktree was inspected, and no agent CLI was run.
