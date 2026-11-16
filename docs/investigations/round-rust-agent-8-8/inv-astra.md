# Item 8.8 — `inv-astra`

## Result

Implementation is paused for a second, separate existing-test conflict. The human authorized the Claude adapter assertion update in clarification round `1`. That authorization remains valid. No production code or existing tests have been changed; this report is a clarification checkpoint, not the implementation of item `8.8`.

## Resolved clarification

The human selected option `a`: rename `a_run_that_hit_its_turn_limit_failed_and_has_no_final_message` and change its stream-outcome assertion to the distinct turn-limit outcome, preserving its final-message, session and activity assertions. The test is in [`crates/agent-runtime/tests/cli_adapters/claude.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-runtime/tests/cli_adapters/claude.rs#L235-L251).

## Blocking protocol contract

- [`docs/state.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/docs/state.md#L287-L293) requires: “A change to a message that old peers cannot read raises the version.”
- `EventKind::RunEnded` carries `RunEnd` in [`crates/agent-core/src/protocol/event.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/src/protocol/event.rs#L134-L149). Adding `turn_limit` produces an event a version `2` peer cannot deserialize.
- `the_protocol_version_is_one_and_is_written_as_a_number` in [`crates/agent-core/src/protocol.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/src/protocol.rs#L159-L179) pins the current version to `2`, despite the historical test name.
- `a_request_with_an_unknown_protocol_version_is_refused_with_a_typed_error` in [`crates/agent-core/tests/protocol.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/tests/protocol.rs#L134-L148) explicitly rejects protocol `3`.
- Additional protocol-bearing tests and golden lines require a mechanical version update. The clarification enumerates the known files and limits permission to version inputs, expectations, fixture references and compatibility coverage.

[`AGENTS.md`, Never rule 1](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/AGENTS.md#L61-L65) explicitly requires stopping and asking when requested behavior conflicts with an existing test. The adapter-only authorization does not cover these protocol assertions.

## Requested decision

The task's `questions-2.md` asks permission to update protocol-version tests for version `3`, preserve historical fixtures unchanged, add new version `3` fixtures, and test rejection of version `2` plus round trips of `turn_limit`. No authorization is assumed. The question is written in the prescribed task directory; its complete text follows for a durable copy.

# Questions — 20261116-015620-inv-astra

## 1. May item 8.8 update existing protocol tests for the required version bump?
Your answer to question 1 authorizes the Claude adapter assertion change; that permission remains in force. A separate compatibility conflict appears in the protocol: `EventKind::RunEnded` carries `RunEnd`, so a version 2 peer cannot deserialize the new `turn_limit` variant. [`docs/state.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/docs/state.md), “Versions”, requires: “A change to a message that old peers cannot read raises the version.” Existing tests instead pin protocol 2 and reject protocol 3. For example, [`crates/agent-core/src/protocol.rs:162`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/src/protocol.rs#L162) asserts `PROTOCOL_VERSION == 2`, and [`crates/agent-core/tests/protocol.rs:135`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/tests/protocol.rs#L135) requires a protocol 3 request to be rejected. [`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/AGENTS.md), Never rule 1, requires authorization before changing these contracts.

a) Authorize the protocol 2 → 3 update to existing test inputs, assertions, fixture references and descriptive names/comments — retain the existing version 1 and 2 fixtures unchanged, add version 3 fixtures, preserve all non-version assertions, and add tests that reject version 2 and round-trip `turn_limit` under version 3. This permits implementing the documented version rule and continuing item 8.8 test-first.
b) Keep the existing protocol tests unchanged — pause item 8.8 until you provide a revised compatibility contract, since leaving protocol 2 unchanged would conflict with the documented version rule.

Known affected test files are [`crates/agent-core/src/protocol.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/src/protocol.rs), [`crates/agent-core/src/protocol/event.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/src/protocol/event.rs), [`crates/agent-core/src/protocol/reply.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/src/protocol/reply.rs), [`crates/agent-core/tests/protocol.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/tests/protocol.rs), [`crates/agent-core/tests/protocol_rounds.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/tests/protocol_rounds.rs), [`crates/agent-core/tests/public_client.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/tests/public_client.rs), [`crates/agent-core/tests/history_health.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent-core/tests/history_health.rs), and [`crates/agent/src/client/tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/1b0ff0fc44762780900369a95275d281caecfd3e/crates/agent/src/client/tests.rs). The requested permission is limited to protocol-version changes and the corresponding compatibility coverage, including any additional protocol-bearing test discovered during compilation; it does not authorize weakening unrelated assertions.

**Recommended:** a) — follows the version policy required by item 8.8, preserves historical fixtures, and keeps mismatched peers from receiving messages they cannot decode. (investigator)

**Verdict:**

## Checks

The existing version assertion was selected for a baseline run under the machine-wide test lock:

```bash
tools/test-lock.sh rust-agent-8.8-protocol-contract cargo test -p agent-core the_protocol_version_is_one_and_is_written_as_a_number --locked
```

Result: `1` passed, `0` failed, `0` ignored. This confirms the version `2` assertion is an active baseline contract. The final-commit gate result is recorded in the handoff after verification. No old Bash or Go suites or real model CLIs are run.

## Acceptance

| Criterion | Status | Evidence |
|---|---|---|
| `A1`: turn-limit run resumes and ends `done`, never `failed` | Not met | Implementation paused at the protocol test conflict. |
| `A2`: configured `max_turns = 50` reaches the CLI | Not met | No configuration or launch changes made. |

## Skipped tests

None added or changed. Implementation tests await clarification.

## Not done

- Distinct turn-limit stream outcome and run end.
- Configurable maximum turns and consecutive resume budget.
- Same-session continuation prompt and daemon settlement/recovery.
- Exhaustion notification and history recording of the continuation reason.
- Protocol version update, compatibility tests and user documentation.
- Pull request: intentionally not opened, as instructed.
