# Item 8.8 — `inv-astra`

## Result

Implementation is blocked pending the human's authorization to change an existing test. No production code, existing test, fixture, dependency or repository instruction has been changed. This report is a clarification checkpoint, not a completed implementation of item `8.8`.

## Blocking contract

[`crates/agent-runtime/tests/cli_adapters/claude.rs:235`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-runtime/tests/cli_adapters/claude.rs#L235-L251) contains `a_run_that_hit_its_turn_limit_failed_and_has_no_final_message`. It replays the `error_max_turns` result and requires:

```rust
assert_eq!(replay.output.stream, StreamOutcome::Failed);
```

The requested item requires turn-limit exhaustion to have a distinct stream outcome and become `RunEnd::TurnLimit`, so the task can resume instead of failing. Keeping the assertion would preserve the old stream contract; equating the new outcome with `Failed` would conceal the distinction this item needs.

[`AGENTS.md`, Never rule 1](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/AGENTS.md#L61-L65) explicitly says: “If the task requires behaviour that an existing test forbids, stop and ask the human in your questions file; wait for the answer.” The broader test-first rules therefore do not authorize changing this test without the human's decision.

## Requested decision

The required task questions file, `questions-1.md`, has been written in the task directory using the prescribed question and verdict format.

- Recommended: authorize changing this test's stream-outcome assertion and renaming it to describe turn-limit exhaustion, while preserving its assertions about the absent final message, retained session and activity event.
- Alternative: retain the existing test and provide a revised stream contract before implementation continues.

After authorization, the change must still be implemented test-first, including the decision boundaries, daemon resumption, configuration, restart, notification and history checks. No authorization has been assumed.

## Checks

Ran under the repository's machine-wide test lock:

```bash
tools/test-lock.sh rust-agent-8.8-conflict cargo test -p agent-runtime --test cli_adapters a_run_that_hit_its_turn_limit_failed_and_has_no_final_message --locked
```

Result: `1` passed, `0` failed, `0` ignored, `83` filtered out. This proves the conflicting assertion is an active, passing baseline contract; it is not a skipped placeholder.

The required final-commit gate is run before pushing this checkpoint; its result is reported in the final handoff. No old Bash or Go suites or real model CLIs are run.

## Acceptance

| Criterion | Status | Evidence |
|---|---|---|
| `A1`: turn-limit run resumes and ends `done`, never `failed` | Not met | Implementation paused at the explicit existing-test conflict. |
| `A2`: configured `max_turns = 50` reaches the CLI | Not met | No configuration or launch changes made. |

## Skipped tests

None added or changed. Item `8.8` tests have not been written because implementation must wait for the decision above.

## Not done

- Distinct turn-limit stream outcome and run end.
- Configurable maximum turns and consecutive resume budget.
- Same-session continuation prompt and daemon settlement/recovery.
- Exhaustion notification and history recording of the continuation reason.
- State/protocol compatibility analysis, regression coverage and user documentation for the implemented feature.
- Pull request: intentionally not opened, as instructed.
