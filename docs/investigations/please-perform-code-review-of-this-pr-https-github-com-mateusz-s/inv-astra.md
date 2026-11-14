# Review of PR #101 against round 8.5 — inv-astra

**VERDICT: REQUEST_CHANGES**

**SUMMARY:** The Rust gate passes, but item `8.5` is not complete: successful pushes disappear from the history, gate capture failures leave the loss count unchanged, truncated loss notes make stories unreadable, and gate paths bypass secret filtering. Human merge commits are omitted; the new reader also has quadratic step lookup, and the golden fixtures violate the required deterministic timing rule.

- PR: [#101](https://github.com/mateusz-szczyrzyca/team-agents/pull/101).
- Reviewed head: [`ce047ec`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ce047ec8215dda14ae86ec7860952351798000a7).
- Compared base: [`ed0bd05`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ed0bd05a7fcabb928927baee1cffa2cdbde4947a).
- Contract: item `8.5` in [`rounds/round-8.5.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ce047ec8215dda14ae86ec7860952351798000a7/rounds/round-8.5.md) and the Rust developer/reviewer rules.
- SIZE: `4,386` changed lines by the conservative outside-tests count; below the task's `5,000` limit, above its approximately `4,000` split-planning threshold.
- GATE: `GATE_WAIT=1 make gate` **PASS** on the reviewed head; `0` findings, Rust checks completed in `71` seconds.

## Blockers

1. **[TDD-2, TDD-6; item 8.5, every push]** [`crates/agent/src/daemon/owner/rounds/publish.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ce047ec8215dda14ae86ec7860952351798000a7/crates/agent/src/daemon/owner/rounds/publish.rs#L182) — After checks send a draft back, `open_pull_request` pushes the corrected head and finds the existing PR, but records neither `PullOpened` nor `PullPushed`; a daemon probe reached reviews on a different head with no corresponding push event. Record the successful push when reusing the draft and test this path, including a subsequent check rejection.
2. **[TDD-2, TDD-6; item 8.5, capture losses]** [`crates/agent/src/daemon/gate.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ce047ec8215dda14ae86ec7860952351798000a7/crates/agent/src/daemon/gate.rs#L317) — A gate that ran but fails the post-command Git inspection exits before history recording; failure to save its state record has the same effect. Both probes left `HistoryHealth.lost` at `0`, with no gate event. Count unavailable content under its capture cause, and record a captured gate independently of whether saving the state-side gate record succeeds.
3. **[TDD-6; item 8.5, write/read equality]** [`crates/agent-runtime/src/history/reader/sequenced.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ce047ec8215dda14ae86ec7860952351798000a7/crates/agent-runtime/src/history/reader/sequenced.rs#L25) — `EVENT_CUTS` omits `detail`, although `HistoryEvent::Lost` explicitly truncates that field; a successfully written long loss note then makes `round_records` fail with `UnknownCutField { field: "detail" }`, so the entire story cannot be read. Accept the field the writer produces and add boundary/non-ASCII round trips for loss notes.
4. **[SEC-2, TDD-6; item 8.5, no secrets]** [`crates/agent-core/src/history/event.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ce047ec8215dda14ae86ec7860952351798000a7/crates/agent-core/src/history/event.rs#L376) — A gate's `record.result.log` is absent from both free-text redaction and identity checking; a known GitHub-token marker inside the absolute log path was present in the actual SQLite file after `History::start_with_secrets` completed. Apply the identity rejection/redaction policy to this path and test database bytes, including a daemon gate whose home path contains the marker.
5. **[TDD-2; item 8.5, merge commit]** [`crates/agent/src/daemon/owner/rounds/merges.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ce047ec8215dda14ae86ec7860952351798000a7/crates/agent/src/daemon/owner/rounds/merges.rs#L168) — Every merge observed through PR polling becomes `Merge::Merged(None)`, so human merges and merges recovered by polling lose their commit without incrementing the loss count. The comment is incorrect: GitHub supplies `merge_commit_sha` after merging. Carry that field through the GitHub response type into `PullMerged`, and test an externally merged PR and the polling/recovery path. [GitHub's endpoint contract](https://docs.github.com/en/rest/pulls/pulls#get-a-pull-request) documents the field; a read-only API request for [#100](https://github.com/mateusz-szczyrzyca/team-agents/pull/100) also returned `merged: true` and a non-null merge SHA.
6. **[PERF-3]** [`crates/agent-runtime/src/history/reader/sequenced.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ce047ec8215dda14ae86ec7860952351798000a7/crates/agent-runtime/src/history/reader/sequenced.rs#L266) — Each step row linearly searches the cached vector of every step of its run; reading `n` sequential steps performs `n(n+1)/2` comparisons, despite loading the vector only once. Index the cache by step number or consume it in ledger order; add a large-history regression that protects the intended complexity.
7. **[TDD-8; round 8.5 testing ground rule]** [`crates/agent/tests/fixtures/team-cli.sh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ce047ec8215dda14ae86ec7860952351798000a7/crates/agent/tests/fixtures/team-cli.sh#L178) — The new `late:` action uses a real `sleep 3` to impose review order, and [`crates/agent/src/daemon/owner/rounds/tests/history_stories.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ce047ec8215dda14ae86ec7860952351798000a7/crates/agent/src/daemon/owner/rounds/tests/history_stories.rs#L107) polls with real sleeps. A delay cannot guarantee process ordering on a loaded host, and this directly contradicts the item's injected-time/no-sleep requirement. Use explicit fixture handshakes for process ordering and injected/paused time for timed behavior; obtain authorization before changing frozen tests.

## Checks

All executable probes used isolated temporary homes or databases, fake model CLIs and the existing fake GitHub server. Tests ran serially through the repository test lock. Temporary probes and mutations were removed; no production code or existing test changes are included in this report commit.

### Focused failure probes

| Probe | Trigger and observed result |
|---|---|
| `review_a_push_after_checks_send_back_is_recorded` | Ran `script:code-only,script:add-test` through the daemon, waited for reviews, stopped the daemon and read the team records. `PullOpened.head` and `ReviewAsked.head` differed; no `PullPushed` held the reviewed head. Failed its assertion. |
| `review_failed_gate_capture_counts_as_history_loss` | Enabled history in the gate fixture and let its gate remove the temporary worktree's Git metadata. Confirmed `GateError::GitAfter`; the loss-count assertion failed with actual `0`, expected `1`. |
| `review_failed_gate_state_save_does_not_erase_history` | Put a directory at the temporary gate-record destination so saving the completed result failed. Confirmed `GateError::Record`; the readable history had `0` gate events and `0` losses. Failed its assertion. |
| `review_lost_detail_truncation_remains_readable` | With a `16`-byte text limit, wrote a `Lost` event whose detail was `"é".repeat(40)`. The write succeeded; reading its round failed with `UnknownCutField` for `detail`. |
| `review_gate_log_path_cannot_persist_known_secret` | Configured token marker `review-secret-marker`, recorded a gate with that marker in its log path, and closed the real SQLite writer. Searching database bytes found the marker. Failed its assertion. |

The first probe extends the existing checks fixture; the next two use the gate fixture; the last two use the runtime history public APIs. These are diagnostic experiments, not new committed tests. The remaining blockers follow directly from the cited control flow/API contract or complexity; no timing benchmark or induced flaky golden run is claimed.

### Required mutation checks

| Temporary mutation | Existing test that caught it | Result |
|---|---|---|
| Omit the gate's history-recording call | `a_gate_the_daemon_ran_is_recorded_with_its_task_head_outcome_and_log_and_linked_to_the_round_of_its_task` | **Detected**: gate count `0`, expected `1`. |
| Force prompt previews even when whole prompts are requested | `the_prompt_is_its_first_lines_and_its_size_unless_the_whole_is_asked_for` | **Detected**: full-prompt assertion failed. |
| Reverse the sequence tie-breaker for equal timestamps | `records_of_the_same_time_keep_the_order_they_were_written_in` | **Detected**: reverse order instead of insertion order. |

After restoring the source, the `story_records` and `story_text` integration suites passed `24/24` tests and the gate-recording test passed `1/1`.

### RED replay

- Replayed [`f7ef159`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f7ef1590ba71ab75030d8a16cbbd3049ab97df58) from a separate `git archive` extraction, without switching the investigation branch.
- Ran the commit's `history_events_tests` selection for the `agent` binary under the test lock.
- The commit compiled; all `5` tests failed on the absent events, matching its commit message: $\color{red}{\textbf{RED}}$ reproduced.
- The current reviewed implementation passed its original gate: $\color{green}{\textbf{GREEN}}$. That does not cover the additional failure probes above.

## Test integrity and deviations

- **Accept — size:** the explicit `5,000`-line limit applies; completing this already substantial item remains within it. The PR description's `4,084` count is stale relative to this reviewed head.
- **Accept — local `expect`:** [`crates/agent-core/src/story/json.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ce047ec8215dda14ae86ec7860952351798000a7/crates/agent-core/src/story/json.rs) confines its disclosed suppression to serializing the closed, JSON-representable event vocabulary; it is not a fallback for failed history reads.
- **Accept for this review — documented best-effort loss notes:** the description reports human approval for bounded, deferred loss tombstones. A permanently unwritable database cannot save its own failure note. This does **not** excuse blocker `2`, where even the in-memory loss count remains zero.
- **Reject — TDD-8 deviation:** inheriting these fixtures from an earlier commit of the same PR does not make their real sleeps comply with the round's requirement; blocker `7` needs an authorized fixture correction or an explicit human waiver.
- **Accept — TEST-FIX [`5d88ce7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5d88ce76ca6d06f85036acfb39cc274fe6cb338c):** updates the schema expectations to the added ledger/version; the disclosed commit also includes schema documentation, so it is not strictly test-only.
- **Accept — TEST-FIX [`9cc791d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9cc791da471dbc2e62017c6056a8113d24e3875e):** corrects the fixture's selection of the owning team.
- **Accept — TEST-FIX [`372a960`](https://github.com/mateusz-szczyrzyca/team-agents/commit/372a960a1bcce99feeb0432acbb732e4451b9e2d):** reflects the fake CLI's actual uncommitted files instead of claiming its tree is clean.
- **Accept — TEST-FIX [`337d736`](https://github.com/mateusz-szczyrzyca/team-agents/commit/337d736f322cadb51b69ac56797fb051e9b1d916):** includes the newly added loss event in the event-count expectation.
- **Accept — approved TEST-FIX [`89fd5ad`](https://github.com/mateusz-szczyrzyca/team-agents/commit/89fd5adf354222d1b8a32ed7fae3379f2703efef):** preserves the successful termination observation rather than performing another racy process-status read; the PR discloses the human authorization.
- **Accept — disclosed test-first exceptions:** the later CLI/golden/coverage additions explain why they test already implemented behavior; the representative earlier RED replay is valid. None of this establishes coverage of the missing failure paths listed above.
- **Compatibility:** the new reader supports pre-ledger histories through temporary views, and the gate includes legacy-read tests. The successful-write/unreadable-loss-note mismatch in blocker `3` is a new read/write compatibility defect.

## Needs human

- The existing frozen fixture tests may not be edited without the repository's required authorization. Decide whether to authorize the deterministic correction for blocker `7` or explicitly waive that requirement; this review does not change them.
- The previously disclosed test corrections and best-effort tombstone approval are accepted within their stated scope, not as permission to omit newly required recording or loss counting.

## Skipped tests

- None added, removed, weakened or skipped by this investigation.
- The old implementation's Go/Bats suites and real model CLIs were not run, as required by the investigation scope.

## Not done

- No fixes, pull request, GitHub review or GitHub comment were submitted; Phase 2 receives this report on the investigation branch.
- Serena was unavailable in the supplied tool catalog; code discovery used `rg` and targeted source reads.
- No hardware devices were used, and no other agent's worktree was inspected.

Test-First: not possible — this change records review findings only; diagnostic experiments were temporary and all source changes were restored.
