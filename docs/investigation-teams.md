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
5. **Runtime execution (Phases 1–4 and Resume).** The complete investigation workflow and resume are implemented and active:
   `agent run --investigation --team "A, B" "topic"` runs the end-to-end pipeline (concurrent Phase 1 investigations on isolated branches, Phase 2 merger selection, integration
   prompt `prompts/inv-merge.md`, PR creation, automated prechecks with single retry bounce `cmd__precheck`, Phase 3 reviewer selection, advisory review `prompts/review.md`, and Phase 4
   human hand-off). Resuming an open PR (`agent run --resume <team>`) conducts Phase 5 advisory reviews across all team investigators proposing improvements without applying changes or auto-merging.
   During Phase 1, member runs execute concurrently on dedicated branches (`agents/<investigator>/<timestamp>`)
   created from the frozen base commit, with prompt `prompts/inv-work.md`, strict MCP isolation,
   and capacity overlay tracking.

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
- **Publication verification and retry:** When an investigator finishes `DONE`, the coordinator
  verifies that the result commit descends from the base, changes code, and is present on `origin`.
  Transient network or transport failures during verification (`git ls-remote`) do not permanently fail
  the task: the child retains its `done` status, the snapshot member status is marked `verifying`, the
  cohort lease is preserved, and verification is retried on subsequent dispatch cycles (logging
  `investigation_publication_check_failed`). Invalid local results or a confirmed missing or mismatched ref mark the
  member failed.
- **Dashboard stage:** `phase 1: N investigators working (A, B, C)` while runs are in progress,
  `phase 1: verifying published results (...)` during publication retries. For legacy phase-1 runs (without
  `INVESTIGATION_PIPELINE=1`), transitioning to `waits for you: phase 1 complete (merge not available yet)`
  or `waits for you: all phase 1 runs failed`. In the full pipeline (`INVESTIGATION_PIPELINE=1`), advancing
  directly to Phase 2 (or `waits for you: all phase 1 runs failed` if no phase 1 child completed `DONE`).
  Phase 1 completion sets `no-comment` on `auto_stage`, preventing inheritance of caller `PR_URL` or
  comments on unrelated PRs.

### Phase 2 — The merge

- **Trigger:** All Phase 1 investigator runs have terminated.
- **Candidate eligibility:** Only investigators whose Phase 1 run concluded with `STATUS: DONE`
  are eligible to serve as the merger. If no Phase 1 run ended `DONE`, merge is skipped, and the
  team transitions to Phase 4 terminal completion (status `done`) as a failure.
- **Selection:** One investigator is chosen uniformly at random among eligible candidates.
  Names of eligible candidates are sorted, and selection is made via an unbiased draw seeded by
  `AGENT_INVESTIGATION_SEED` or parent `INVESTIGATION_SEED` (falling back to `/dev/urandom` when unseeded).
- **Execution:** The selected merger creates a new integration branch from the frozen base commit.
  It inspects the branches of all Phase 1 runs:
  - Preserves unique findings and implementations.
  - De-duplicates redundant or overlapping solutions.
  - Resolves conflicts and contradictions.
- **Publication:** The merger pushes the unified branch and opens a pull request.
- **PR Description requirements:** Explicitly details per investigator:
  - What was incorporated (with links to relevant commits).
  - What was discarded and the rationale ("duplicate of C's...", "superseded by A's...").
- **Verification (`cmd__precheck`):** After the pull request is opened, automated prechecks run
  via `cmd__precheck` (`make gate`, test suite, PR description conventions).
  - If prechecks fail, the run bounces back to the merger once (`INVESTIGATION_MERGE_RETRIES=1`) with failure
    diagnostics appended to `task.md` and stage `merging: <merger> (fix checks)`.
  - A second precheck failure or a merger failure (crash, non-zero exit, or cannot start) marks that merger failed
    and triggers one replacement (`INVESTIGATION_MERGER_REPLACEMENTS=1`) by another eligible `DONE` investigator
    (supporting up to 3 total Phase 2 merger runs when an initial merger bounces and fails prechecks before replacement).
  - If the replacement merger fails, or if prechecks fail with replacements exhausted, the team hands off to the human
    (`waits for you: merge failed`).
- **Dashboard stage:** `merging: <merger>` during merge execution, `merging: <merger> (fix checks)` during
  precheck fix retries.

### Phase 3 — The review

- **Trigger:** Passing prechecks on the opened pull request.
- **Publication revalidation:** Before launching the reviewer (especially after a capacity queue wait),
  the merger's published integration branch is re-validated against `PRECHECK_HEAD`. A transient transport
  or lookup error retries; a confirmed changed or unavailable branch triggers Phase 2 precheck failure
  handling following the same global bounce and replacement rules.
- **Candidate eligibility:** Any investigator in the team *other than* the selected merger.
  Investigators whose Phase 1 run failed or timed out remain eligible to review, ensuring two-member
  teams can proceed even if one investigator encountered a Phase 1 failure.
- **Selection:** Chosen uniformly at random from eligible candidates (excluding merger), sorted and
  drawn via `AGENT_INVESTIGATION_SEED` or parent `INVESTIGATION_SEED`.
- **Review protocol:** Executes `prompts/review.md` without modification. The review evaluates the
  PR, suggests improvements, and concludes with an explicit verdict (`ready-to-merge`, `needs-fixes`, `incomplete`).
- **Replacement on failure:** If the reviewer crashes, exits non-zero, or yields no explicit verdict, it is replaced
  once (`INVESTIGATION_REVIEW_RETRIES=1`) by another eligible team investigator (excluding the merger).
  If the replacement reviewer also fails, the team hands off to the human (`waits for you: review failed`).
- **Dashboard stage:** `review: <reviewer> (#<pr-number>)`.

### Phase 4 — Terminal completion

- **Trigger:** Completion of Phase 3 review (or total failure in Phase 1 / exhausted retries).
- **Actions:**
  - Releases investigator capacity leases (`INVESTIGATION_LEASE=""`).
  - Sets `INVESTIGATION_HANDOFF_PENDING=1` to track pending external hand-off actions.
  - Caller notification metadata is explicitly isolated (`REVIEW=0`, `WORK_ROUND=`, `WORK_ROUNDS=`,
    `JOB_ID=`, `JOB_LEAD=`, `LEAD_URL=`, `PR_URL=`) to prevent inheritance from the dispatching runner.
  - External actions use existing best-effort helpers (`human_on_pr`, `notify`) executed at least once:
    - If a PR was opened, `human_on_pr` comments on the PR and records `INVESTIGATION_COMMENTED=1`.
    - Human notification is sent via `notify` and records `INVESTIGATION_NOTIFIED=1`.
    - Note that flags record completed local invocation attempts rather than guaranteeing external network delivery;
      a crash or process restart before flag persistence may re-execute an external action.
  - Sets `INVESTIGATION_HANDOFF_PENDING=""` once external actions complete.
  - If interrupted during hand-off, subsequent supervisor dispatch resumes hand-off and completes
    pending external actions (guaranteeing recovery even after capacity leases have been cleared).
  - The team enters status `done` (terminal completion) after external notification attempts and flags are recorded. No code is ever merged automatically; the PR is available for independent human review and decision.
- **Terminal completion stages:**
  - `advisory reviews complete (#<pr-number>)` (on successful advisory review pass)
  - `all phase 1 runs failed` (if no phase 1 child completed `DONE`)
  - `merge failed` (if merge replacements or precheck retries exhausted)
  - `review failed` (if reviewer replacement failed)

### Resume lifecycle (`agent run --resume <team>`)

- **Open PR:** When resumed with an open PR, every investigator in the team (A, B, and C) performs
  an advisory review of the existing PR. Reviews provide constructive critique, GitHub suggestions
  (```` ```suggestion ````), or unified diffs (```` ```diff ````). No changes are automatically
  applied by the agents. The team releases investigator capacity leases and reaches Phase 4
  terminal completion (status `done`).
- **Merged or closed PR:** Resuming an investigation team whose PR is already merged or closed is
  refused with an explanatory message ("the PR is merged — a new topic is a new run" or
  "the PR is closed — a new topic is a new run") before creating any background tasks or worktrees.
- **Refusal checks:** Resume requires a completed human hand-off (`waiting`) with no active lease or pending hand-off.
  Incompatible flags (`--same-round`, `--verify`) and teams without an open PR are rejected fail-fast.
- **All-investigator cohort admission:** Admission reserves all team investigators together under
  `dispatch.lock`. If any team investigator is temporarily unavailable (running in another task or in `QUOTA_UNTIL` pause),
  the parent task queues in the capacity queue (`waiting: capacity queue`) until all members are free.
  No partial starts are performed.
- **Manifest and tracking:** Prior to acquiring `INVESTIGATION_LEASE 1`, the complete manifest skeleton
  `resume.json` (version 1) is written to `$TASKS/<parent>/resume.json` with pre-allocated child task IDs
  (`INVESTIGATION_RESUME_TASKS`), the current PR commit SHA (`INVESTIGATION_RESUME_HEAD`), and an incremented
  resume pass counter (`INVESTIGATION_RESUME_COUNT`).
- **Phase 5 execution:** Admission sets `INVESTIGATION_PHASE=5`, `INVESTIGATION_LEASE=1`, and clears prior Phase 4
  terminal hand-off markers (`INVESTIGATION_HANDOFF_PENDING`, `INVESTIGATION_COMMENTED`, `INVESTIGATION_NOTIFIED`,
  `INVESTIGATION_HANDOFF_STAGE`, `INVESTIGATION_HANDOFF_RESULT`). Each team member (including the original Phase 2
  merger and Phase 3 reviewer) receives a dedicated child advisory review task (`KIND=investigation`, `REVIEW=1`,
  `INVESTIGATION_PHASE=5`) running `prompts/review.md` against the PR branch.
  While reviews are underway, the stage displays `review: N investigators reviewing (#<pr-number>)`.
- **Fault tolerance and no replacements:** Unlike Phase 2 and Phase 3, there are no replacements in the advisory
  cohort because all team investigators participate simultaneously. Any child crash, non-zero exit, or missing
  verdict is recorded in `resume.json`. During supervisor recovery, missing or unstarted child slots (status `new`
  without an active runner unit) fail while live running child slots retain their capacity lease.
- **Phase 4 hand-off:** When all advisory reviews reach terminal states, `INVESTIGATION_LEASE` is cleared and the
  team returns to Phase 4 human hand-off. The stage transitions to `waits for you: advisory reviews ready (#<pr-number>)`
  (or `waits for you: advisory reviews ready (#<pr-number>, N failed)` if any reviewer failed or omitted an explicit
  verdict). The PR receives a comment via `human_on_pr`, the human is notified via `notify`, and the parent returns
  to `waiting`. Repeated resumes can be run on subsequent updates to the PR, tracking each pass with `INVESTIGATION_RESUME_COUNT`.

## Random selection and reproducibility

1. **Uniform selection:** Candidate lists are sorted lexicographically before random draws to ensure
   reproducible candidate index ordering across platforms.
2. **Determinism via seed:** When `AGENT_INVESTIGATION_SEED` (environment/config) or parent task
   `INVESTIGATION_SEED` is defined, selections use a deterministic pseudo-random generator seeded with
   this value, ensuring fully reproducible test runs. The seed must be a non-negative decimal integer
   matching `^[0-9]+$`. If non-decimal characters are present, a warning `investigation_seed_invalid` is
   logged; if PRNG derivation fails, `investigation_seed_failed` is logged. A seed validation or selection
   failure triggers an immediate failure handoff to the human (`waits for you: merge failed` or
   `waits for you: review failed`), releasing capacity leases rather than misclassifying the team into the capacity queue.
3. **Entropy:** When unset, cryptographically secure pseudo-random entropy (`/dev/urandom`) is used
   for unbiased selection. If reading random bytes fails, `investigation_random_failed` is logged.

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
4. **Cancellation and stop lifecycle:** When an investigation team is stopped (`agent stop <id>` or `agent stop --running`), stop handling waits for any in-flight cohort admission under `dispatch.lock` before stopping children, cancelling pending handoffs and prechecks, and releasing its capacity lease. If stop is interrupted before cleanup finishes, subsequent supervisor ticks detect the retained lease and trigger dispatch cleanup outside the lock without advancing the stopped team.

An interrupted merger or reviewer admission marks its unstarted child `failed` before replacement,
so the child cannot retain investigator capacity after the parent releases its lease.

## Failure handling and replacement matrix

| Scenario | Condition | System Action | Stage Display |
|---|---|---|---|
| Phase 1 partial failure | At least one investigator ends `DONE` | Other runs proceed normally; failed run excluded from merger selection | `phase 1: N investigators working (...)` |
| Phase 1 total failure | Zero investigators end `DONE` | Skip merge and PR; notify human and wait | `waits for you: all phase 1 runs failed` |
| Phase 2 precheck failure | Merger's PR fails `cmd__precheck` | Bounced back to merger once (`INVESTIGATION_MERGE_RETRIES=1`) with failure log appended to `task.md` | `merging: <merger> (fix checks)` |
| Phase 2 merger abort | Merger crashes, fails, cannot start, or opens a PR against the wrong base | Replaced once (`INVESTIGATION_MERGER_REPLACEMENTS=1`) by another eligible `DONE` investigator; if none remain, wait for human | `merging: <merger-2>` or `waits for you: merge failed` |
| Phase 2 precheck retry failure | Merger fails second precheck | Marks merger failed; triggers replacement if replacements remain (`INVESTIGATION_MERGER_REPLACEMENTS=1`); hands off to human if replacements exhausted | `merging: <merger-2>` or `waits for you: merge failed` |
| Phase 3 review failure | Reviewer crashes, fails, or yields no explicit verdict | Replaced once (`INVESTIGATION_REVIEW_RETRIES=1`) by another eligible team member (excluding merger); if none remain, wait for human | `review: <reviewer-2> (#<pr-number>)` or `waits for you: review failed` |
| Candidate temporarily busy | Candidate in `QUOTA_UNTIL` / queue | Pick another free candidate; queue if all eligible candidates are temporarily paused | `waiting: capacity queue` |
| Resume on merged/closed PR | Target PR is merged or closed on GitHub | Refuse command immediately before creating tasks or taking capacity | Exit 1 with explanatory message (`the PR is merged — a new topic is a new run` or `the PR is closed — a new topic is a new run`) |
| Resume capacity contention | Any team investigator busy or in `QUOTA_UNTIL` | Queue entire cohort under `dispatch.lock`; no partial starts | `waiting: capacity queue` |
| Resume advisory review failure | One or more advisory reviewers crash, fail, or omit explicit verdict | No replacements in advisory cohort; failed/missing verdict slots recorded in `resume.json`; handoff still transfers to human with failure count | `waits for you: advisory reviews ready (#<pr-number>, N failed)` |

## Task metadata and data model

> [!NOTE]
> Round 4 finalizes the full investigation lifecycle, adding Phase 5 resume metadata, advisory review tracking, and manual verification procedures.
> Task attributes are additive; existing task files, metadata, and bus state remain backwards-compatible. Old Phase 4 tasks lacking Phase 5 fields load normally.

Task attributes stored in the orchestration task's `meta.env` and task files (no bus state changes):

### Phase 1 metadata
- `KIND=investigation_team`: Identifies the task as an investigation orchestration unit.
- `AGENT=investigation-team`: Coordinator identity for the orchestration unit.
- `INVESTIGATION_TEAM`: Comma-separated list of member investigators (e.g. `inv-astra,inv-gemini-argon`).
- `INVESTIGATION_BASE_SHA`: Immutable base commit SHA from which all member branches branch.
- `task.md`: Original topic text, using the existing task file convention for multiline input.
- `INVESTIGATION_TOPIC_SLUG`: Sanitized topic string used for document paths and prompt placeholder `{{INVESTIGATION_TOPIC_SLUG}}`.
- `INVESTIGATION_PHASE`: Numeric phase identifier (`1` during Phase 1).
- `INVESTIGATION_PHASE1_TASKS`: Comma-separated list of child Phase 1 task IDs.
- `INVESTIGATION_LEASE`: `1` while the whole cohort owns its investigators; cleared upon entering Phase 4.
  Admission writes the initial `phase1.json` manifest skeleton with allocated task IDs before setting
  `INVESTIGATION_LEASE 1` to prevent orphaned lease leaks if admission is interrupted.
- `phase1.json` (version 1): Structured JSON record storing Phase 1 member outcomes:
  `{"version":1,"members":[{"agent":"...","task":"...","branch":"...","status":"...","commit":"..."}]}`.
  During recovery or periodic supervision, unstarted children (status `new` without an active runner unit)
  fail rather than hanging, while live running children hold capacity. If a legacy leased parent is
  encountered without a manifest, `investigation_manifest_recovered` logs reconstruction from parent-listed
  task IDs and child metadata, clearing orphaned leases once no children are running or awaiting publication verification.
- Child investigator tasks: `KIND=investigation`, `INVESTIGATION_TEAM=<coordinator-task-id>` (no bus `TEAM`),
  `BASE=<frozen-base-sha>`, prompt `prompts/inv-work.md`, no agentbus MCP tools.
- `AUTO_STAGE`: Human-readable stage displayed on the dashboard (e.g. `phase 1: N investigators working (...)`).

### Phases 2–4 orchestration metadata (Round 3)
Parent orchestration task fields:
- `INVESTIGATION_PIPELINE`: `1` for the full investigation pipeline (Round 3 onwards), advancing across phases 1–4; unset or `0` for legacy phase-1 runs without pipeline.
- `INVESTIGATION_TARGET_BRANCH`: Target base branch for merge and review (`main` by default, or the configured target branch).
- `INVESTIGATION_PHASE`: Current numeric phase identifier (`1`, `2`, `3`, `4`).
- `INVESTIGATION_MERGER`: Name of the chosen merger investigator.
- `INVESTIGATION_MERGER_TASK`: Child task ID of the merger run.
- `INVESTIGATION_MERGER_BRANCH`: Integration branch created by the merger (`agents/<investigator>/<timestamp>`).
- `INVESTIGATION_TRIED_MERGERS`: Comma-separated list of previously attempted merger investigators.
- `INVESTIGATION_MERGER_REPLACEMENTS`: Replacement counter for failed mergers (`1` after first replacement).
- `INVESTIGATION_MERGE_RETRIES`: Retry counter for precheck failure bounces (`1` after first precheck bounce).
- `PRECHECK_HEAD`: Commit SHA of the merger branch when prechecks run.
- `INVESTIGATION_PRECHECK`: Precheck runner status (`running`, `passed`, `failed`).
- `INVESTIGATION_PRECHECK_RUN`: Numeric run counter for precheck runner invocations (`1`, `2`, ...).
- `INVESTIGATION_PRECHECK_UNIT`: Transient systemd unit name for the precheck runner (`agent-<parent>-precheck-<run>`).
- `INVESTIGATION_PRECHECK_REASON`: Failure diagnostics from `cmd__precheck` to fix.
- `INVESTIGATION_PR`: Opened pull request URL.
- `PR_URL`: Standard pull request URL.
- `PR_NUMBER`: Standard pull request number.
- `BRANCH`: Merger pull request branch.
- `BASE`: Target base branch for the pull request (e.g. `main`), recorded on the parent task upon PR discovery to match the child merger's `BASE`. (The immutable starting commit SHA is preserved separately in `INVESTIGATION_BASE_SHA`).
- `INVESTIGATION_REVIEWER`: Name of the chosen reviewer investigator.
- `INVESTIGATION_REVIEWER_TASK`: Child task ID of the reviewer run.
- `INVESTIGATION_TRIED_REVIEWERS`: Comma-separated list of previously attempted reviewer investigators.
- `INVESTIGATION_REVIEW_RETRIES`: Replacement counter for failed reviewers (`1` after first replacement).
- `INVESTIGATION_VERDICT`: Explicit verdict returned by reviewer (`ready-to-merge`, `needs-fixes`, `incomplete`).
- `INVESTIGATION_HANDOFF_STAGE`: Final stage text recorded at handoff.
- `INVESTIGATION_HANDOFF_RESULT`: Terminal result string (`done` or `failed`) for notification.
- `INVESTIGATION_HANDOFF_PENDING`: Set to `1` while hand-off external actions are underway; cleared once both finish.
- `INVESTIGATION_COMMENTED`: Set to `1` once the PR comment has been posted to prevent duplicate comments.
- `INVESTIGATION_NOTIFIED`: Set to `1` once human notification has been dispatched to prevent duplicate notifications.
- `INVESTIGATION_SEED`: Optional non-negative decimal integer seed overriding `AGENT_INVESTIGATION_SEED`.

Child merger/reviewer task fields:
- `INVESTIGATION_TARGET_BRANCH`: Target base branch for merge or review (`main` or merger branch).
- `INVESTIGATION_PHASE1_MANIFEST`: Absolute path to parent's `$TASKS/<parent>/phase1.json`.
- `REVIEW`: Set to `1` for the Phase 3 reviewer child task.

### Phase 5 resume metadata (Round 4)
Parent orchestration task fields:
- `INVESTIGATION_PHASE`: Set to `5` while advisory reviews run, returning to `4` at human hand-off.
- `INVESTIGATION_RESUME_TASKS`: Comma-separated list of child advisory review task IDs for the current resume pass.
- `INVESTIGATION_RESUME_HEAD`: Commit SHA of the PR branch when the resume pass was initiated.
- `INVESTIGATION_RESUME_COUNT`: Numeric counter tracking the number of resume passes performed on the team (e.g. `1`, `2`, ...).
- `resume.json` (version 1): Structured JSON record storing Phase 5 member outcomes:
  `{"version":1,"members":[{"agent":"...","task":"...","status":"...","verdict":"..."}]}`
  (where `verdict` is a string: `ready-to-merge`, `needs-fixes`, `incomplete`, or empty if missing/failed).
- Admission & Recovery: Admission writes the complete `resume.json` manifest skeleton with allocated task IDs and sets `INVESTIGATION_RESUME_TASKS` before setting `INVESTIGATION_LEASE 1`. All-investigator admission occurs under `dispatch.lock`. During supervisor recovery, missing or unstarted child slots (status `new` without an active runner unit) fail while live running child slots retain the capacity lease. No replacements are attempted in the advisory cohort. Old Phase 4 tasks lacking these fields load normally without error.

Child advisory review task fields:
- `KIND=investigation`: Identifies child as an investigation task.
- `INVESTIGATION_TEAM`: Parent orchestration task ID.
- `INVESTIGATION_PHASE=5`: Marks task as a Phase 5 advisory reviewer.
- `REVIEW=1`: Directs runner to render `prompts/review.md` in advisory mode.
- `PR_URL`: Standard pull request URL.
- `PR_NUMBER`: Standard pull request number.
- `BASE`: PR branch or target commit being reviewed.

### Newly logged events
- `investigation_precheck_interrupted`: Precheck runner unit died or was interrupted before recording a result.
- `investigation_pr_lookup_failed`: GitHub CLI query (`trusted_prs`) failed while locating the open PR for the merger branch.
- `precheck_unit_failed`: Systemd could not spawn the transient precheck runner service unit.
- `investigation_seed_invalid`: `AGENT_INVESTIGATION_SEED` or `INVESTIGATION_SEED` contains non-digit characters (must match `^[0-9]+$`).
- `investigation_seed_failed`: Checksum/state derivation from the decimal seed failed.
- `investigation_random_failed`: Unseeded `/dev/urandom` byte read failed during candidate selection.

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

1. **Round 1 (Completed):**
   - Architectural design document (`docs/investigation-teams.md`).
   - `--investigation` CLI parsing and comprehensive refusal validations (`bin/agent`).
   - Explicit inactive execution stub ("Investigation teams are not available yet").
   - Help text and status entry in `README.md`.
2. **Round 2 (Completed):**
   - Concurrent launch of investigator runs on dedicated branches (`agents/<investigator>/<timestamp>`).
   - Prompt `prompts/inv-work.md` ensuring strict isolation and documentation conventions.
   - Internal runner `_investigation-team` with public CLI entry remaining inactive.
   - Capacity tracking and dashboard stage updates.
   - Phase 1 completion tracking, versioned recording (`phase1.json`), and failure handling.
3. **Round 3 (Completed — Phases 2–4):**
   - Public CLI entry point `agent run --investigation` is active for initial pipeline runs (phases 1–4).
   - Uniform seeded selection of merger from DONE members.
   - Integration prompt `prompts/inv-merge.md`.
   - `cmd__precheck` enforcement with single-bounce retry.
   - Pull request creation and PR description verification.
   - Uniform seeded selection of reviewer and execution of `prompts/review.md`.
   - Single replacement logic for failed mergers and reviewers.
   - Phase 4 human notification and transition to idle wait.
4. **Round 4 (Completed — Resume and Polish):**
   - Resume handler on open PR (`agent run --resume <team>`) triggering all-investigator Phase 5 advisory reviews without auto-merging.
   - Immediate refusal of resume on merged or closed PRs ("the PR is merged — a new topic is a new run" or "the PR is closed — a new topic is a new run").
   - Dedicated `resume.json` version 1 manifest and pass tracking metadata (`INVESTIGATION_RESUME_TASKS`, `INVESTIGATION_RESUME_HEAD`, `INVESTIGATION_RESUME_COUNT`).
   - All-investigator admission and recovery under `dispatch.lock`, with live slot lease retention and no replacements in advisory cohorts.
   - Return to Phase 4 human hand-off (`waits for you: advisory reviews ready (#<pr-number>)` or failure count suffix).
   - Comprehensive README updates, compatibility guarantees for legacy Phase 4 tasks, and human verification checklist.

## Manual checklist (human)

> [!IMPORTANT]
> This checklist is designed for human operators to execute manually when evaluating live investigation team behavior. Agents must not claim this checklist as completed during automated test runs.

Follow this checklist to verify an investigation team run and resume manually:

1. **Configuration**:
   Verify that `agents.conf` defines at least one investigator under `INVESTIGATORS_TYPE`.
2. **Initial investigation start (single investigator)**:
   Run `agent run --investigation --team "inv1" "Topic text"`.
   Confirm that:
   - The investigator starts in an isolated worktree (`phase 1: 1 investigator working`).
   - The same investigator then merges the results (`merging: <merger>`), opens a PR, and prechecks run.
   - The merger cannot review their own results (review is skipped with single investigator).
   - Final stage transitions to `advisory reviews complete` (if resumed) and status becomes `done`.
   - No human decision wait is required; the PR is ready for independent review and merge.
   
   With multiple investigators:
   Run `agent run --investigation --team "inv1, inv2" "Topic text"`.
   Confirm that:
   - Both investigators start concurrently in isolated worktrees (`phase 1: 2 investigators working`).
   - One investigator is chosen to merge (`merging: <merger>`), opens a PR, and prechecks run.
   - The other investigator reviews the PR (`review: <reviewer> (#<pr>)`).
   - Final stage transitions to terminal completion and status becomes `done`.
3. **Advisory resume on open PR**:
   With the PR open, run `agent run --resume <team-id> "Additional check"`.
   Confirm that:
   - All team investigators review the open PR concurrently (`review: 2 investigators reviewing (#<pr>)`).
   - Reviews are posted to GitHub as comments/suggestions without altering code or merging.
   - Stage transitions to `waits for you: advisory reviews ready (#<pr>)` and status returns to `waiting`.
4. **Refusal on merged or closed PR**:
   Merge or close the PR (`gh pr merge` or `gh pr close`).
   Run `agent run --resume <team-id>`.
   Confirm immediate refusal (`the PR is merged — a new topic is a new run` or `the PR is closed — a new topic is a new run`) without creating tasks or worktrees.
