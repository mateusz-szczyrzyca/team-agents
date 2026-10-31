# Investigation teams

This document describes the design, lifecycle, and operational boundaries of investigation
teams in team-agents (`agent run --investigation`). Investigation teams are dedicated to deep,
multi-perspective analysis and resolution of complex topics exclusively using investigators
configured in `INVESTIGATORS_TYPE`.

## Boundaries and core principles

1. **Investigators only at every step.** No normal agents, workers, or helpers participate.
   The team conductor is an internal shell/system orchestration task (`systemd --user`), not a
   normal AI agent. Tools such as `delegate` or worker board requests are prohibited within an
   investigation team; every investigator performs all necessary exploration, coding, and review
   autonomously.
2. **Strict independence during investigation.** Members work on isolated branches with no bus
   communication between them and without reading each other's worktrees.
3. **Pull request on success.** A successful workflow produces a unified pull request opened by
   a designated merger, followed by an independent review and human hand-off. Failures before
   publication, including all Phase 1 runs failing or exhausted merger replacements, wait for
   the human with the reason instead of claiming a PR was produced.
4. **Human in control.** No code is ever merged automatically. The final stage always notifies
   and transfers control to the human.
5. **No runtime side effects in Round 1.** During Round 1, the CLI validates flags, options, and
   arguments, but execution is intentionally inactive ("Investigation teams are not available yet").
   No tasks, worktrees, git branches, or systemd units are created for valid `--investigation` runs.

## CLI invocation and validation

### Command syntax

```bash
agent run --investigation --team "A, B, C" "topic text"
agent run --investigation --team "A, B, C" -f topic.md
```

### Options and argument rules

- `--investigation`: Activates investigation team mode.
- `--team`: Required argument listing at least two distinct investigator names separated by commas.
  Spaces following commas (e.g. `--team "inv-astra, inv-gemini-argon"`) are accepted and trimmed.
- Topic specification: Provided as positional arguments, via `-f <file>`, or through standard input.

### Refusal rules (fail-fast validation)

The command must be rejected immediately before any background task or worktree is created when:
1. `--team` is omitted or empty.
2. `--team` contains fewer than two names.
3. `--team` contains duplicate names.
4. Any name in `--team` is not a configured investigator in `INVESTIGATORS_TYPE` of `agents.conf`.
   The refusal message lists all configured investigators.
5. `--investigation` is combined with incompatible flags:
   - `--autopilot`: Refused (investigation teams manage their own review and hand-off lifecycle).
   - `--verify`: Refused (review is an intrinsic phase of investigation teams).
   - `--need` or `--on`: Refused (investigators run exclusively on the lead host).
   - Remote member specifications (`agent@node`): Refused.
   - `--resume`, `--last-team` or `--same-round`: Refused when combined with `--investigation` (resuming an
     investigation team uses `agent run --resume <team>` without `--investigation`).

## Lifecycle and phases

### Phase 1 — Independent work

- **Trigger:** Initial run launch.
- **Execution:** All investigators specified in `--team` start simultaneously in independent worktrees.
- **Branches:** Each investigator works on an isolated branch created from the frozen base commit:
  `agents/<investigator>/<timestamp>`.
- **Isolation:** Investigators do not communicate via the bus and do not inspect other members'
  worktrees.
- **Outputs:**
  - Code changes: Follow full repository rules (`AGENTS.md`, test-first / TDD, gate verification).
    Changes are committed and pushed to the investigator's branch; no PR is opened.
  - Non-code findings: Written to `docs/investigations/<topic-slug>/<investigator>.md`.
- **Termination:** Each run concludes independently with `STATUS: DONE` or `STATUS: FAILED`.
  A failed run or a run without a published code/findings result does not stop the others. Capture
  the result commit and status before choosing the merger; a bare `DONE` without a result is not success.
- **Dashboard stage:** `phase 1: N investigators working (A, B, C)`.

### Phase 2 — The merge

- **Trigger:** All Phase 1 investigator runs have terminated.
- **Candidate eligibility:** Only investigators whose Phase 1 run concluded with `STATUS: DONE`
  are eligible to serve as the merger. If no Phase 1 run ended `DONE`, merge is skipped, and the
  team transitions to Phase 4 waiting for the human.
- **Selection:** One investigator is chosen uniformly at random among eligible candidates.
  Names of eligible candidates are sorted, and selection is made via an unbiased draw seeded by
  `AGENT_INVESTIGATION_SEED`.
- **Execution:** The selected merger creates a new integration branch from the frozen base commit.
  It inspects the branches of all Phase 1 runs:
  - Preserves unique findings and implementations.
  - De-duplicates redundant or overlapping solutions.
  - Resolves conflicts and contradictions.
- **Publication:** The merger pushes the unified branch and opens a pull request.
- **PR Description requirements:** Explicitly details per investigator:
  - What was incorporated (with links to relevant commits).
  - What was discarded and the rationale ("duplicate of C's...", "superseded by A's...").
- **Verification (`cmd__precheck`):** Before handing off to review, the merger runs standard prechecks
  (`make gate`, test suite, PR description formatting).
  - If prechecks fail, the run bounces back to the merger once with failure details to rectify.
- **Dashboard stage:** `merging: <merger>`.

### Phase 3 — The review

- **Trigger:** Successful completion of Phase 2 merge and passing prechecks.
- **Candidate eligibility:** Any investigator in the team *other than* the selected merger.
  Investigators whose Phase 1 run failed or timed out remain eligible to review, ensuring two-member
  teams can proceed even if one investigator encountered a Phase 1 failure.
- **Selection:** Chosen uniformly at random from eligible candidates (excluding merger), sorted and
  drawn via `AGENT_INVESTIGATION_SEED`.
- **Review protocol:** Executes `prompts/review.md` without modification. The review evaluates the
  PR, suggests improvements, and concludes with a definitive verdict.
- **Replacement on failure:** If the reviewer fails or finishes without a verdict, it is replaced
  once by another eligible team investigator.
- **Dashboard stage:** `review: <reviewer> (#<pr-number>)`.

### Phase 4 — Human decision and hand-off

- **Trigger:** Completion of Phase 3 review (or total failure in Phase 1 / exhausted retries).
- **Actions:**
  - Capacity leases on investigators are released so investigators are not held idle while the
    human decides.
  - The team enters an idle waiting state.
  - If a PR was opened, a comment mentioning the human is posted to the pull request.
  - A notification is sent to the human.
- **No auto-merge:** The human reviews the findings, PR, and review comments, and decides whether
  to merge, request further modifications, or close the PR.
- **Dashboard stage:** `waits for you: review ready (#<pr-number>)` or `waits for you: all phase 1 runs failed`.

### Resume lifecycle (`agent run --resume <team>`)

- **Open PR:** When resumed with an open PR, every investigator in the team (A, B, and C) performs
  an advisory review of the existing PR. Reviews provide constructive critique, GitHub suggestions
  (```` ```suggestion ````), or unified diffs (```` ```diff ````). No changes are automatically
  applied by the agents. The team releases investigator capacity leases and returns to Phase 4
  waiting for the human.
- **Merged or closed PR:** Resuming an investigation team whose PR is already merged or closed is
  refused with an explanatory message ("a new topic is a new run").

## Random selection and reproducibility

1. **Uniform selection:** Candidate lists are sorted lexicographically before random draws to ensure
   reproducible candidate index ordering across platforms.
2. **Determinism via seed:** When `AGENT_INVESTIGATION_SEED` is defined in the environment, selections
   use a deterministic pseudo-random generator seeded with this value, ensuring fully reproducible
   test runs.
3. **Entropy:** When `AGENT_INVESTIGATION_SEED` is unset, cryptographically secure pseudo-random
   entropy (`/dev/urandom` / `getrandom`) is used for unbiased selection.

## Capacity, concurrency, and queue policy

1. **Resource occupation:** While an investigation team phase is actively running, it occupies its
   assigned investigators. Capacity leases are released when entering Phase 4 (waiting for human)
   so that investigators are not locked while waiting for human input.
   Admission waits until all requested investigators can be reserved together, under the same
   dispatch lock used by the queue and review pairs; it does not hold a partial set while waiting.
   Ownership remains with the team between phases. Its own reservations do not make its candidates
   externally busy when starting merge or review runs.
2. **Queue interaction:** External review pairs waiting for free investigators observe occupied
   investigators as busy and wait in the queue per existing capacity mechanisms.
3. **Queue policy on candidate unavailability:** (Adopted via consultation C1 and user autopilot approval)
   If an otherwise eligible candidate for merger or reviewer is temporarily unavailable (in quota
   pause `QUOTA_UNTIL` or transient lock):
   - If other eligible candidates are currently free, an available candidate is selected immediately.
   - If *all* eligible candidates are temporarily unavailable, the team waits in the capacity queue
     for an investigator to become free.
   - Immediate escalation to human waiting occurs only if no untried eligible candidate exists or
     if a replacement has already failed.

## Failure handling and replacement matrix

| Scenario | Condition | System Action | Stage Display |
|---|---|---|---|
| Phase 1 partial failure | At least one investigator ends `DONE` | Other runs proceed normally; failed run excluded from merger selection | `phase 1: N investigators working (...)` |
| Phase 1 total failure | Zero investigators end `DONE` | Skip merge and PR; notify human and wait | `waits for you: all phase 1 runs failed` |
| Phase 2 precheck failure | Merger's branch fails `cmd__precheck` | Bounced back to merger once with failure log | `merging: <merger> (retry 1/1)` |
| Phase 2 merger abort | Merger crashes, fails, or fails second precheck | Replaced once by another eligible `DONE` investigator; if none remain, wait for human | `merging: <merger-2>` or `waits for you: merge failed` |
| Phase 3 review failure | Reviewer crashes or yields no verdict | Replaced once by another eligible team member (excluding merger); if none remain, wait for human | `review: <reviewer-2>` or `waits for you: review failed` |
| Candidate temporarily busy | Candidate in `QUOTA_UNTIL` / queue | Pick another free candidate; queue if all eligible candidates are temporarily paused | `waiting: capacity queue` |

## Task metadata and data model (Planned)

> [!NOTE]
> Metadata fields below are planned for subsequent runtime rounds (Rounds 2–4). No schema or
> state modifications are persisted in Round 1.

Planned attributes stored in the orchestration task's `meta.env` and bus state:
- `KIND=investigation_team`: Identifies the task as an investigation orchestration unit.
- `INVESTIGATION_TEAM`: Comma-separated list of member investigators (e.g. `inv-astra,inv-gemini-argon`).
- `INVESTIGATION_BASE_SHA`: Immutable base commit SHA from which all member branches branch.
- `task.md`: Original topic text, using the existing task file convention for multiline input.
- `INVESTIGATION_TOPIC_SLUG`: Sanitized topic string used for document paths.
- `INVESTIGATION_PHASE`: Numeric phase identifier (`1`, `2`, `3`, `4`).
- `INVESTIGATION_PHASE1_TASKS`: Mapping of investigator names to their Phase 1 task IDs, branches,
  published result commits, and statuses.
- `INVESTIGATION_MERGER`: Name of the chosen merger.
- `INVESTIGATION_REVIEWER`: Name of the chosen reviewer.
- Merge/review task and branch references: Keep later phase runs separate from the Phase 1 results.
- `INVESTIGATION_PR`: URL and number of the opened pull request.
- `INVESTIGATION_SEED`: Seed used for deterministic selections.
- `INVESTIGATION_MERGE_RETRIES`: Counter for merge precheck retries.
- `INVESTIGATION_MERGER_REPLACEMENTS`: Separate counter for replacing a failed merger.
- `INVESTIGATION_REVIEW_RETRIES`: Counter for reviewer replacement.
- Resume review results: Track the task and verdict for each original investigator's advisory review.
- Hand-off progress: Track PR comment and notification completion so restarts recover the same hand-off.
- `AUTO_STAGE`: Human-readable stage displayed on the dashboard.

These are logical fields; structured mappings need a versioned record rather than raw multiline values
in `meta.env`. The runtime round must define the encoding, preserve existing task and bus formats,
and add legacy-load fixtures whenever it extends a persisted type. No seed setting is read in Round 1.

## Reuse boundaries and system interfaces

When implementing future rounds, existing components have explicit boundaries:
1. **Selection (`pick_investigator`):** `pick_investigator` in `bin/agent` implements provider-biased
   round-robin rotation for review pairs. Investigation team selection must *not* reuse `pick_investigator`;
   it requires a dedicated uniform random selection helper.
2. **Availability checking (`free_investigator_list` vs `free_investigator`):** `free_investigator_list`
   inspects `KIND=investigation` statuses and `QUOTA_UNTIL`, but `free_investigator` omits quota checks.
   Investigation team capacity tracking must use a consistent reservation overlay visible to both normal
   queues and review-pair paths.
3. **Worktree and branch creation (`start_investigation`):** Existing `start_investigation` creates
   branches from the caller's current mutable branch and names them `inv<ID>-<timestamp>`. Investigation
   teams must freeze an immutable `INVESTIGATION_BASE_SHA` and name branches
   `agents/<investigator>/<timestamp>`.
4. **Review execution:** Reuses `prompts/review.md` directly without prompt changes.
5. **Checks and notifications:** Reuses `cmd__precheck`, `auto_stage`, and human notification helpers.

## Four-round implementation roadmap

1. **Round 1 (Current):**
   - Architectural design document (`docs/investigation-teams.md`).
   - `--investigation` CLI parsing and comprehensive refusal validations (`bin/agent`).
   - Explicit inactive execution stub ("Investigation teams are not available yet").
   - Help text and status entry in `README.md`.
2. **Round 2 (Phase 1):**
   - Concurrent launch of investigator runs on dedicated branches (`agents/<investigator>/<timestamp>`).
   - Prompt `prompts/inv-work.md` ensuring strict isolation and documentation conventions.
   - Capacity tracking and dashboard stage updates.
   - Phase 1 completion tracking and failure handling.
3. **Round 3 (Phases 2–4):**
   - Uniform seeded selection of merger from DONE members.
   - Integration prompt `prompts/inv-merge.md`.
   - `cmd__precheck` enforcement with single-bounce retry.
   - Pull request creation and PR description verification.
   - Uniform seeded selection of reviewer and execution of `prompts/review.md`.
   - Single replacement logic for failed mergers and reviewers.
   - Phase 4 human notification and transition to idle wait.
4. **Round 4 (Resume and Polish):**
   - Resume handler on open PR triggering all-investigator advisory reviews.
   - Refusal of resume on merged or closed PRs.
   - Comprehensive README updates and human verification checklist.
