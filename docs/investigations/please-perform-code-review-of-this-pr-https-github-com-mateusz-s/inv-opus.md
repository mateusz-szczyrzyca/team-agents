# Re-review of PR #101 against item 8.5 — `inv-opus` (task `20261115-012102-inv-opus`)

PR: [#101](https://github.com/mateusz-szczyrzyca/team-agents/pull/101), "Item 8.5: the history of a pull request, and the
story of a round". Reviewed head: [`f01d6ed`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f01d6ed9014c3e9b0b9a342318b875f1b259f335)
(the PR's `headRefOid`, after `5` fix passes). Measured against item 8.5 of `rounds/round-8.5.md`, the review rules
(`prompts/investigator/code-review.md`), the developer rules (`prompts/agent/rust-developer.md`), `AGENTS.md`, and the
latest reviews: `inv-astra` (on [`fff5808`](https://github.com/mateusz-szczyrzyca/team-agents/commit/fff580885497dd47cf34fe20900e65ffd45aa563)),
`inv-kimi-deepinfra` and `inv-claude-sonnet` (both on `f01d6ed`). My previous review, at
[`311120f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/311120fac52d5f7f0716961ea60eb9e502878180), is the earlier
version of this file in [`8738b91`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/docs/investigations/please-perform-code-review-of-this-pr-https-github-com-mateusz-s/inv-opus.md).

```text
VERDICT: REQUEST_CHANGES
SUMMARY: The ledger, the pure story, offline `agent log` and the seven daemon-made goldens are in place, the gate is
         green and both checks of the item pass with the real binary. Not merge-ready: three recording defects (an
         unbuildable fix pass is not told in the story, the human's allowance in a mixed answer is never recorded, and
         one refused loss note blocks every later note), all three of my mutations survived, and TYP-6 is still broken.
SIZE: 4,324 changed lines outside tests (limit 5,000 for this plan)
GATE: `GATE_WAIT=1 make gate` — ok rust 78s (fmt, clippy -D warnings, tests, doc); 2,455 passed, 0 failed, 0 ignored
```

## Criteria (the item's checks)

- **Check 1 — met.** I copied the `history.db` of the team round with one fix pass out of its daemon run, after the
  daemon stopped. I ran the real `target/debug/agent log --round <id>` against a fresh `AGENTS_HOME` that held only
  that file. The story shows:
  - the item;
  - `5` agents by role with how each was chosen, the reviewers included;
  - every run with its prompt's first lines, its size and its numbered steps;
  - the files each run changed;
  - two review passes with their verdicts;
  - `pull request #1 merged as 7d423ed`.
- **Check 2 — met.**
  - `--prompts` prints both second-pass reviewer prompts whole (`30,066` bytes each; I compared them with `runs.prompt`
    in SQLite).
  - `--json | wc -l` gives `81`; an independent SQL count of the round's records gives `81`.

## Checks

| What | Result |
|---|---|
| Gate on `f01d6ed` | **PASS**, `78` s, `2,455` tests passed, `0` failed, `0` ignored |
| PR-1 command against `origin/main...HEAD` | `4,324` (`4,320` without the test-only `deadline_tests.rs`); within `5,000` |
| Merge with current `main` ([`c4de42a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c4de42ab2ab24cf260be667d37d02f9278c8acc8)) | `git merge-tree` reports no conflict; `rounds/round-8.5.md` is unchanged on `main` |
| `agent log --round <unknown>` / `--team <unknown>` | message and exit `3` |
| `agent log` with no `history.db` in `AGENTS_HOME` | exit `1`, message names the path, no file created |
| `agent log --team <id>` | `team ended` printed once; `--json` gives `87` lines (`81` of the round plus `6` team-wide records) |
| `--no-steps` | no `steps (` line |
| $\color{red}{\textbf{RED}}$ [`63e9fc8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/63e9fc8189658e6e25be99677ae67b2d9172dbb8), checked out and run | `a_fix_pass_whose_event_cannot_be_built_is_counted_as_lost` fails on its assertion (`left: 0`, `right: 1`) |
| Freeze (TDD-14): every RED commit diffed to `HEAD` for removed asserts | every hit comes from one of the `10` listed `TEST-FIX` commits |
| Sweep of added lines | the only production `expect` is the accepted one in `story/json.rs`; no `unsafe`, `println!`, `process::exit` |

## Behaviours broken on purpose (review step 6)

I reverted each mutation before the next one. A control mutation proved that the harness catches a real break: the
merge recorded with no round makes `6` golden tests fail.

| # | Mutation | Where | Result |
|---|---|---|---|
| 1 | A closed pull request is recorded with no round (recorded) | [`merges.rs#L175-L180`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent/src/daemon/owner/rounds/merges.rs#L175-L180) | **survived**: `cargo test -p agent`, `720` + all integration tests green |
| 2 | A late review report is not recorded at all (recorded) | [`verdicts.rs#L139-L163`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent/src/daemon/owner/rounds/verdicts.rs#L139-L163) | **survived**: `cargo test -p agent` all green |
| 3 | A head-capture loss note goes to the *first* run of its task instead of the latest (`.rev()` removed) (story) | [`text.rs#L488-L497`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-core/src/story/text.rs#L488-L497) | **survived**: `cargo test -p agent-core -p agent -p agent-runtime` all green |
| — | My earlier mutation: a gate linked to the task's *oldest* run | [`ledger.rs#L181`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-runtime/src/history/ledger.rs#L181) | now killed by `a_task_with_runs_in_two_rounds_links_second_round_events_only_to_the_second_round_and_renders_team_ended_once` |

## BLOCKERS

1. **[COR, item "a story says … that it was lost and why"]**
   [`crates/agent/src/daemon/owner/history.rs#L193-L227`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent/src/daemon/owner/history.rs#L193-L227)
   — a fix pass with no findings, or with no item in `Fixing` for the run's agent, only calls `record_lost()`.
   - Nothing is reported under a cause, and no `Lost` note is written, although the team, task and rounds are known.
   - The story then shows a fix-pass run and no fix pass.
   - I confirm the finding of `inv-claude-sonnet` (major 1) and `inv-kimi-deepinfra` (major 1).
   - → Add a cause for an event that cannot be built. Queue `HistoryEvent::Lost { what: LostKind::Event(EventKind::FixPassStarted), .. }`
     linked to `rounds`, as `questions_text` does. Assert the note and its story line.
2. **[COR, item "allowed by the human"] — new.**
   [`crates/agent/src/daemon/owner/rounds/checks.rs#L185-L192`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent/src/daemon/owner/rounds/checks.rs#L185-L192)
   — `CheckOutcome::Allowed` is recorded only when the human allowed **every** issue.
   - In a mixed answer, the issues the human allowed are recorded nowhere: the refused ones come back as `sent_back`,
     and the allowance disappears from the history and the story.
   - The pass-2 fix turned "recorded wrongly" into "not recorded". The test
     `a_mixed_allow_and_refuse_answer_does_not_record_all_issues_allowed` only checks that the event is absent.
   - → Record `Allowed` with `decided.allowed` whenever it is non-empty (the event carries its own issue list), and
     assert those issues.
3. **[COR, ERR-1] — new.**
   [`crates/agent-runtime/src/history/notes.rs#L197-L212`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-runtime/src/history/notes.rs#L197-L212)
   (with [`ledger.rs#L101-L124`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-runtime/src/history/ledger.rs#L101-L124))
   — all pending notes are written as one batch.
   - A note takes the team and rounds of the record it is about. So the note of a record refused by the round rule
     ("a record cannot belong to a round of another team") is refused again, every time.
   - From then on, every note waits until `256` newer ones push it out.
   - **Proved:** I wrote a scratch test (not committed): a foreign-team run that names this team's round, then a run
     that names its round twice. The second, valid note is never written (`0` notes, expected `1`). Without the
     foreign run, the same test writes it (`1`).
   - → Write notes one by one. Drop a note refused by a `Rule` and count it; keep only notes refused for a transient
     reason.
4. **[TDD-2] — new.**
   [`crates/agent/src/daemon/owner/rounds/verdicts.rs#L136-L163`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent/src/daemon/owner/rounds/verdicts.rs#L136-L163)
   — no test covers the `review_submitted` of a late report.
   - A late report answers a review pass that is already decided.
   - Mutation 2 removed this record entirely, and every test stayed green.
   - No golden has a late report: in `story-team-fix-pass.txt` the approval comes before the request for changes.
   - → Add a daemon test in which `REQUEST_CHANGES` decides the pass first and the other reviewer reports after it.
     Assert its `ReviewSubmitted`, its pass and its round.
5. **[TDD-2] — new.**
   [`crates/agent/src/daemon/owner/rounds/merges.rs#L168-L181`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent/src/daemon/owner/rounds/merges.rs#L168-L181)
   — nothing tests that `pull_closed` is linked to its round.
   - With mutation 1 the closing is missing from the round story, and nothing fails.
   - `a_pull_request_the_human_closed_is_recorded_once_however_often_it_is_looked_at` only counts the team's events.
   - → Assert the rounds of the event, or `closed` in `agent log --round`.
6. **[TDD-2] — new.**
   [`crates/agent-core/src/story/text.rs#L477-L507`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-core/src/story/text.rs#L477-L507)
   — `attach_run_loss` picks the latest run of the task, and no test checks that choice (mutation 3).
   - Every story test has one run per task that has a loss.
   - → Add a story test with two runs of one task and the capture loss on the second.
7. **[TYP-6]** `_ =>` arms on the workspace enum `Record`:
   [`text.rs#L74`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-core/src/story/text.rs#L74)
   and [`#L163`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-core/src/story/text.rs#L163)
   (confirming `inv-claude-sonnet` major 2). Also two binding catch-alls:
   - `other =>` on `HistoryEvent` at [`text.rs#L752`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-core/src/story/text.rs#L752);
   - `kind =>` on `EventKind` in `LostKind::from_str` at [`event.rs#L94`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-core/src/history/event.rs#L94),
     added in pass 2. With it, a new event kind becomes a valid lost kind without anyone deciding so.
   - → List the variants.
8. **[ERR-2, TDD-2, PR-7] — new.**
   [`crates/agent/src/daemon/owner/controls.rs#L276-L287`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent/src/daemon/owner/controls.rs#L276-L287)
   — [`e5d5e59`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e5d5e59c1cb4e82c8ca867ef8cb7df8fcb327b72)
   changed the existing `record_control`: a clock error used to fail the operation, and is now logged and skipped.
   - On that error, the control's published event and its activity-log lines are dropped.
   - This is not the history: `record_event` already handles the history's own clock failure.
   - No test provokes this path (the only clock-failure tests are in `gate/tests.rs` and `history_events.rs`).
   - → Restore the propagation. Or, if this is the intended behaviour, write it down and add a test.
9. **[PERF-3] — new.**
   [`crates/agent-runtime/src/history/ledger.rs#L101-L124`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-runtime/src/history/ledger.rs#L101-L124)
   — `check_rounds` scans every earlier link of a round, for every record and every round it names.
   - When no foreign link exists, `LIMIT 1` never stops the scan early.
   - Every step of a run is linked to the run's rounds, so the writer's work grows with the square of a round's
     steps. A slow writer drops records when its queue is full.
   - → Look up the owner in `rounds` first, and otherwise compare with the round's first link only: the invariant
     gives a round one team.

## MINOR

1. **[ERR-2]** [`handle.rs#L370`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-runtime/src/history/handle.rs#L370)
   — `Timestamp::now(clock).unwrap_or(start_at)` hides a clock failure and stamps the note with the run's start
   (confirming `inv-claude-sonnet` minor 1).
2. **[TYP-4, COR]** Capture losses are filed under the wrong cause:
   - `capture_run_failed` accepts every `CaptureKind`, and maps `Gate`, `Questions` and `Clock` to `capture_changes`
     ([`handle.rs#L364-L368`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-runtime/src/history/handle.rs#L364-L368)).
   - A gate's clock failure is counted and noted as `capture_gate`, not `capture_clock`
     ([`gate.rs#L313-L317`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent/src/daemon/gate.rs#L313-L317)).
   - → Use a two-variant run-capture kind, and file clock failures under `capture_clock`.
3. **[ERR-2]** [`history.rs#L55-L66`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent/src/daemon/owner/history.rs#L55-L66)
   — an item with no round is logged and dropped: its event is in no round story, and it is not counted
   (confirming `inv-claude-sonnet` minor 2).
4. **[TYP-1, ground rule "refused … again when read"]**
   [`reader/sequenced.rs#L191`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-runtime/src/history/reader/sequenced.rs#L191)
   — `cut` is checked against the union `EVENT_CUTS`, not against the event's own fields: a `pull_ready` row with
   `cut = "report"` is read, and the story says it was cut. → Check against the event's `texts_mut()` names.
5. **[tests, ground rule "every record … proved by a run of the daemon"]** `story-no-merge.txt` and `story-checks.txt`
   have no gate record. Their rigs use the `work` action, which writes `gate.json` itself
   ([`team-cli.sh#L104-L113`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent/tests/fixtures/team-cli.sh#L104-L113)).
   The pull request is "opened … after the repository's gate passed", but the story has no gate and no loss note.
   → Use `gated` in those rigs.
6. **[ERR-11]** [`log.rs#L113-L114`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent/src/commands/log.rs#L113-L114)
   — `agent log --round <id> --json | head -2` ends with `agent: cannot write the story: Broken pipe (os error 32)`.
   → Treat a closed pipe as a normal end, and test it.
7. **[PERF-3]** [`ledger.rs#L82-L88`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-runtime/src/history/ledger.rs#L82-L88)
   — the update of a run's `changes` time has no `team` in its `WHERE`, so the `(kind, team, key)` index narrows only
   by `kind`, and every run end scans all `changes` rows. → Add `AND team = ?`.
8. **[story completeness, from my previous review, still open]** the round story leaves out the run of the worker
   that is linked to no round. In the fix-pass round, the round has `81` records and the team `87`, and that run is
   among the difference. The story is not wrong, and the docs now say so.
9. **[TDD-1]** The message of $\color{red}{\textbf{RED}}$ [`63e9fc8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/63e9fc8189658e6e25be99677ae67b2d9172dbb8)
   names the test, but has no test command and no failing lines. It does fail on its assertion.
10. **[PR-9]** Wrong or missing facts in the PR description:
    - `## What and why` names the branch `agents/inv-gemini-argon/20261114-190223`, not
      `agents/inv-claude-sonnet/20261114-151411`.
    - The script's "Round size" block says `0 lines`.
    - At `4,324` the PR is above PR-2's ~`4,000` threshold for planning a split, and no split is recorded.

## Previous findings — status at `f01d6ed`

My blockers at `311120f`:

1. A run's `changes` got the run's start time: **fixed** ([`ledger.rs#L82-L88`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-runtime/src/history/ledger.rs#L82-L88), `RunEnd` updates it).
2. `team ended` was printed once per round in `--team`: **fixed** (`in_round(…, false)`; I counted `1`).
3. A loss was told in its place only when the database refused it: **partly fixed**. Capture failures are now noted.
   An unbuildable fix pass (blocker 1) and an item with no round (minor 3) are still not.
4. A human merge had no merge commit: **fixed** (`merge_commit_sha`).
5. How long a gate took was untested: **fixed** ([`f88810e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f88810ec7b4c62cd8d88724d3b0a50870f2d8097)).
6. The gate's link to the task's latest run was untested: **fixed** (the mutation is now killed).
7. My minors 7, 8, 9, 12, 14 and 16 are **fixed**. Minor 10 is open (minor 8 above). Minor 11 is documented.

The latest reviews:

- `inv-kimi-deepinfra`:
  - major 1 is confirmed (blocker 1);
  - majors 2 and 3 are in NEEDS HUMAN.
- `inv-claude-sonnet`:
  - majors 1 and 2 are confirmed (blockers 1 and 7);
  - minors 1 and 2 are confirmed (minors 1 and 3);
  - minor 3 is in NEEDS HUMAN.
- `inv-astra`'s size blocker is **fixed** in `f01d6ed`: `4,324` lines.

## ACCEPTED DEVIATIONS

- **[ERR-3]** [`story/json.rs#L44-L50`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f01d6ed9014c3e9b0b9a342318b875f1b259f335/crates/agent-core/src/story/json.rs#L44-L50)
  — a scoped `expect`. Its reason holds: events are plain data, and every kind is serialized in a test.
- **[TDD-14]** All `10` listed `TEST-FIX` commits are accepted.
  - [`f88810e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f88810ec7b4c62cd8d88724d3b0a50870f2d8097)
    loosens the exact pin of [`20922fc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/20922fcb90d02c6c156d68ce008ac60e9cd13e1c)
    to `>= 1s`. The exact value was not deterministic, because daemon tasks share the ticking clock, and a gate that
    takes no time still fails it.
- **Loss notes are a best effort:** a queue-full or secret drop is counted, not noted. The human approved this, and
  `docs/history.md` documents it.
- **[PR-1]** `4,324` lines, within the plan's `5,000`.

## NEEDS HUMAN

- **An existing test was changed:** [`89fd5ad`](https://github.com/mateusz-szczyrzyca/team-agents/commit/89fd5adf354222d1b8a32ed7fae3379f2703efef)
  (`deadline_tests.rs`). It records your approval (question `1`, verdict `a`) and weakens nothing.
- **Two inherited commits have a weak `Test-First: not possible` reason:**
  [`a70e9d1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a70e9d16cd550c257db1639aaa0528d1f44722dc) and
  [`e09d40e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e09d40eee563cb92ec21a0dab806b44b138167c0)
  (`inv-kimi-deepinfra` majors 2 and 3).
  - The branch cannot be rewritten.
  - At the head both behaviours are covered: `Lost` was reworked through
    [`48b93e3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/48b93e385b8a2b985eddb8c1c650ee13f0610246) →
    [`33e4840`](https://github.com/mateusz-szczyrzyca/team-agents/commit/33e484067cc732d9f3563d8df2d71544d00c17cb),
    and `inv-claude-sonnet` showed that `4` goldens fail when the merge time is reverted.
  - I recommend accepting both as a deviation. A focused ordering test for the merge time would still be welcome.
- **A design risk for item 8.9 (not this PR):** `check_rounds` refuses any record of another team that is linked to
  an existing round. When 8.9 hands a round to a new team, every record of the new team would be refused, and their
  notes would poison the note queue (blocker 3).

## Text on GitHub

Every review and comment on the PR is by `ms-ai-agents`. None of them tried to steer this review. I treated them as
claims and checked each one I report.

## Not done

- No real model CLI and no suite of the old implementation was run.
- Nothing of the PR was changed. The mutations and the scratch test were reverted (`git status` clean at `f01d6ed`).
  This report is the only commit of this branch.
