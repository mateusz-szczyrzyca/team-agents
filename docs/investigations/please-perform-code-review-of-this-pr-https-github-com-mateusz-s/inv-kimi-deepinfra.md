# Merged review of PR #101 against item 8.5 — `inv-kimi-deepinfra`

PR: [#101](https://github.com/mateusz-szczyrzyca/team-agents/pull/101). Reviewed head:
[`311120f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/311120fac52d5f7f0716961ea60eb9e502878180).

**VERDICT: REQUEST_CHANGES.** The merged review retains every finding of the only Phase 1 result. Two reproduced
recording/story defects alone require changes; the loss-reporting gap, the missing merge commit and the two surviving
test mutations add independent reasons.

## Phase 1 inputs and merge decisions

The manifest contains one member, `inv-opus`, with recorded result
[`2d296c3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2d296c35984b96b8a2dabaac27053fd7e7158c03).
That commit was fetched from `origin` and fast-forwarded onto this branch unchanged. Its full report is
[`inv-opus.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/2d296c35984b96b8a2dabaac27053fd7e7158c03/docs/investigations/please-perform-code-review-of-this-pr-https-github-com-mateusz-s/inv-opus.md).

- **Taken:** the complete report, its checks, its three mutation experiments and findings `1`–`16`.
- **Merged into this summary:** the major and medium findings become the blockers below; the remaining ten findings
  retain their original minor grouping.
- **Dropped:** nothing. There were no duplicate or contradictory results to resolve.
- **Absent or failed results:** none recorded in the manifest. No other investigator or code implementation was
  published for this phase, so this merge is a review integration only and makes no production change.

## Blocking findings

1. **[COR] Changes are timestamped at run start, not run end.** The lookup in
   [`ledger.rs#L162-L174`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/history/ledger.rs#L162-L174)
   cannot see the end recorded later by
   [`owner/history.rs#L219-L225`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/history.rs#L219-L225).
   Give the capture its own end time or record the end first, and prove equality with the run end in daemon data.
2. **[COR] A multi-round team story repeats `team ended` once per round.** The daemon links the event to every round in
   [`teams.rs#L742-L745`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/teams.rs#L742-L745),
   while each round renders its own linked records in
   [`story/text.rs#L88-L96`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-core/src/story/text.rs#L88-L96).
   Store the event with team scope only or select/render it once; add a two-round daemon-backed story test.
3. **[COR] Capture and queue losses lack the required cause and story tombstone.** Examples in
   [`owner/history.rs#L15-L28`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/history.rs#L15-L28)
   call the cause-less counter in
   [`handle.rs#L331-L336`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/history/handle.rs#L331-L336);
   only a later SQLite refusal gets a note through
   [`notes.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/history/notes.rs).
   Record typed causes and place capture/queue loss notices in the story, or obtain an explicit narrowing of item 8.5.
4. **[COR] A human-made merge loses its merge commit.** `Merged(None)` in
   [`merges.rs#L166-L170`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/rounds/merges.rs#L166-L170)
   follows a response decoder without `merge_commit_sha` in
   [`github.rs#L1717-L1747`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/github.rs#L1717-L1747).
   Decode and persist that field; an unavailable SHA is itself a capture loss with a cause.
5. **[TDD-2 / TDD-7] The gate-duration regression survives the suite.** Changing the recorded start to the finish
   leaves all tests green; the current assertion in
   [`history_events.rs#L391`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/rounds/tests/history_events.rs#L391)
   checks only ordering. Assert the injected clock's exact start and duration.
6. **[TDD-2] Latest-run round attribution is not guarded.** Reversing the latest-run query in
   [`ledger.rs#L145-L159`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/history/ledger.rs#L145-L159)
   survives the suite because
   [`gate_rounds.rs#L33`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/tests/history/gate_rounds.rs#L33)
   has only one run per task. Exercise two runs whose tasks belong to different rounds.

## Minor findings retained

1. **[TYP-4]** Free strings persisted for loss kind/cause in
   [`event.rs#L260-L267`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-core/src/history/event.rs#L260-L267);
   use the existing kind and cause types.
2. **[TYP-4]** Three Boolean parameters in
   [`log.rs#L75-L81`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/commands/log.rs#L75-L81);
   pass the existing `PromptView`, `StepView` and `Format` enums.
3. **[Item 8.5, story point 2]** Reviewer agents and how they were selected are absent from the agents section in
   [`story/text.rs#L184-L227`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-core/src/story/text.rs#L184-L227);
   record and render their role/selection or explicitly narrow the item.
4. **[Story completeness]** A run linked to no round is omitted from the round story and printed after `team ended`
   in the team story by
   [`story/text.rs#L88-L109`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-core/src/story/text.rs#L88-L109);
   place team-wide runs in time order and document any unavoidable scope exclusion.
5. **[Item 8.5, closed PRs]** Only approved PRs reach closure recording in
   [`merges.rs#L115-L131`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/rounds/merges.rs#L115-L131);
   observe and record closure during review too.
6. **[TDD-8]** The golden fixture's real sleeps in
   [`team-cli.sh#L116-L125`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/tests/fixtures/team-cli.sh#L116-L125)
   and
   [`team-cli.sh#L178-L182`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/tests/fixtures/team-cli.sh#L178-L182)
   were added by this PR, so preserving old tests does not justify them; use injected sequencing instead.
7. **[TDD-1]** RED commit
   [`1fe99b1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1fe99b17f3ab022b8e731125be4a2987c9d48b19)
   has two passing tests and two failures through indexing/`unwrap`, not assertions; the historical deviation needs
   an explicit human decision because this merge must not rewrite RED/GREEN history.
8. **[ERR-1]** Bounded loss-note bookkeeping silently clears all scopes and evicts pending notes in
   [`notes.rs#L60-L81`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/history/notes.rs#L60-L81);
   preserve scope and log/count every eviction with its cause.
9. **[TDD-14 / PR-2]** `TEST-FIX`
   [`5d88ce7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5d88ce76ca6d06f85036acfb39cc274fe6cb338c)
   also changes
   [`history.schema.sql`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/docs/history.schema.sql),
   and the inherited `4,084`-line PR passed the approximate split-planning threshold without a recorded plan; both
   disclosed deviations need review, not silent acceptance.
10. **[Cosmetic]** The question notification duplicates the task identity, and a recorded notification can precede
    the question that caused it in
    [`story-question-answered.txt`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/tests/golden/story-question-answered.txt);
    remove the duplication and timestamp/order the events by their actual occurrence.

## Phase 1 evidence retained

`inv-opus` reports a green `GATE_WAIT=1 make gate`, the PR-1 count of `4,084` lines against the `5,000`-line task
limit, both item checks by hand, unknown-round exit `3`, seven golden stories passing in three runs, and the expected
failure pattern when checking RED commit
[`1fe99b1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1fe99b17f3ab022b8e731125be4a2987c9d48b19).
Mutation `2` (reversed equal-time order) was killed; mutations `1` (zero gate duration) and `3` (oldest-run gate
attribution) survived. These are attributed Phase 1 observations, not claims of a second independent run by this
merger; the merger's final-head gate result is reported separately on PR #101.

## Needs human

- The Phase 1 reviewer could not read the human's verdict narrowing loss reporting to best-effort tombstones.
  Finding `3` therefore needs either implementation of the stated item or an explicit accepted narrowing.
- The malformed RED history and the test-plus-documentation `TEST-FIX` above cannot be corrected by silently
  rewriting the preserved commits; accept them explicitly or direct an allowed history strategy.
- Existing tests were not modified by this merge; no skipped tests, dependencies, lint suppressions or unsafe code
  were added.
