# Review of PR #101 against item 8.5 — `inv-opus` (task `20261114-184005-inv-opus`)

PR: [#101](https://github.com/mateusz-szczyrzyca/team-agents/pull/101), "Item 8.5: the history of a pull request, and the
story of a round". Reviewed head: [`8738b91`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8738b91d1152f252e36f9a2466d100d9beef8519),
the PR's `headRefOid`. Its code under `crates/` is the same as at
[`89fd5ad`](https://github.com/mateusz-szczyrzyca/team-agents/commit/89fd5adf354222d1b8a32ed7fae3379f2703efef); the later
commits only add review reports and merge `main`. I checked it against item 8.5 of
[`rounds/round-8.5.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/rounds/round-8.5.md),
the review rules (`prompts/investigator/code-review.md`), the developer rules and `AGENTS.md`.

This report replaces the earlier `inv-opus` report in this file, which reviewed
[`311120f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/311120fac52d5f7f0716961ea60eb9e502878180). The code has not
changed since then, so every finding of that report still stands. I checked each one again and folded it in below.

```text
VERDICT: REQUEST_CHANGES
SUMMARY: The ledger, the pure story, `agent log` and the daemon's recording sites are in place, the gate is green and both
         checks of the item pass. But a cut loss note makes a whole round unreadable, the team story and the `--json` order
         are wrong on real daemon data, several things the item names are not recorded, and four of my five mutations
         survived the suite.
SIZE: 3,743 changed lines outside tests and investigation reports (4,522 with them); limit 5,000 for this plan
GATE: fmt ok, clippy ok, test ok (2,426 passed, 0 failed, 0 ignored), doc ok — `GATE_WAIT=1 make gate` PASS at 8738b91, 79 s
```

## Checks

| What | Result |
|---|---|
| `GATE_WAIT=1 make gate` at `8738b91` | **PASS**: `1` step (`rust`) in `79` s; `2,426` tests passed, `0` failed, `0` ignored |
| PR-1 size command against `origin/main...HEAD` | `3,743` lines without `docs/investigations/`, `4,522` with them (the description's `4,084` predates the four review reports) |
| Strict clippy pass (`-W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing ...`) | `0` warnings |
| Mechanical sweep of the added production lines | `6` leads, none a finding (one scoped `#[expect]`, two specified fallbacks, an `Option` flatten, two `spawn`s of joined services) |
| Existing tests changed (`git diff origin/main...HEAD` over `*.rs`) | `1`: [`crates/agent-runtime/src/git/deadline_tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-runtime/src/git/deadline_tests.rs#L123-L138), by the `TEST-FIX` [`89fd5ad`](https://github.com/mateusz-szczyrzyca/team-agents/commit/89fd5adf354222d1b8a32ed7fae3379f2703efef) the human approved |
| Tests of this PR changed after their RED commit | only by the `5` listed `TEST-FIX` commits (judged below) |
| RED [`f7ef159`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f7ef1590ba71ab75030d8a16cbbd3049ab97df58) checked out in a temporary worktree | builds; `cargo test -p agent history_events_tests`: `0` passed, `5` failed, as its message says |
| Item check 1: a team round with one fix pass, read after the daemon stopped, with the real binary | **passes**: the item, `3` agents by role, every run with its prompt's first lines and steps, the files each changed, two review passes with their verdicts, `merged as a9a3f28` |
| Item check 2: `--prompts` prints the reviewers' second prompts whole | **passes**: all `4` reviewer prompts (passes `1` and `2`, `30,066` bytes each) are found whole between their markers |
| Item check 2: `--json \| wc -l` equals the round's records in the database | **passes**: `81` = `81` (SQL: links of the round plus the team's start) |

I ran the item's checks like this. I patched the golden test
`a_team_round_with_a_worker_one_fix_pass_and_a_second_review_pass_that_approves_is_told_whole` for a moment, so that it
copied `history.db` out after `rig.stop()`. Then I ran `target/debug/agent log` against a fresh `AGENTS_HOME` that held
only that file. The patch was reverted, not committed.

## Behaviours broken on purpose (review step 6)

Each mutation was reverted before the next one. M1 to M4 ran before the RED replay. For M5 I first rebuilt the target
directory, which the replay had shared.

| # | Mutation | Kind | Result |
|---|---|---|---|
| M1 | `PullPushed` records the old head (`before.head`) instead of the pushed `ready.tip` ([`publish.rs#L273`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/rounds/publish.rs#L268-L276)) | recorded | killed: `2` tests fail |
| M2 | The ledger links a gate to the task's *oldest* run (`ORDER BY at ASC, rowid ASC`) ([`ledger.rs#L151`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-runtime/src/history/ledger.rs#L146-L159)) | recorded | **survived**: `cargo test -p agent-runtime --test history` (`81`) and the daemon's history and gate tests stay green |
| M3 | The story breaks a tie of time by the reverse write order ([`story.rs#L71`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/story.rs#L71)) | story | killed: `3` core tests and `4` goldens fail |
| M4 | `rounds_of_tasks` takes the rounds of the task's *first* run, not its latest ([`owner/history.rs#L62`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/history.rs#L56-L69)). This links questions, answers, controls and notifications. | recorded | **survived**: `cargo test -p agent`, `709` + integration tests green |
| M5 | Both 8.5 calls of `record_lost()` removed: an unreadable questions file and a clock that cannot give a time are no longer counted ([`owner/history.rs#L25`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/history.rs#L17-L29), [`#L80`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/history.rs#L72-L84)) | recorded (loss) | **survived**: `cargo test -p agent` green |

I also ran two probes (scratch tests, reverted):

- A `lost` note whose `detail` is longer than `max_text_bytes = 64` is written, then `round_records` is called. It fails
  with `Err(Value { table: "history_records", column: "cut", key: "12", source: Cut(UnknownCutField { field: "detail" }) })`.
- A two-round team whose `TeamEnded` is linked to both rounds, as the daemon links it. `render(.., Selection::Team(..))`
  prints `team ended` `2` times.

## BLOCKERS

Items marked *(new)* were not in the earlier `inv-opus` report. Items marked *(also …)* were found by other reviewers too.

1. **[COR, recorded] A cut loss note makes the whole round and team unreadable.**
   [`sequenced.rs#L25`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-runtime/src/history/reader/sequenced.rs#L25)
   — `EVENT_CUTS` lacks `detail`, which
   [`event.rs#L351`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/history/event.rs#L351)
   cuts. One loss note over the limit then makes `agent log` exit `1` for that round and team (probe above). Fix: derive
   the list from `texts_mut` (one source of the names), and add a test that writes every kind with every text over the
   limit. *(new; also `inv-astra`, `inv-gemini-argon`)*
2. **[COR, recorded] Every `changes` record has the time its run started.** The run engine records the changes when
   the process exits
   ([`run.rs#L407`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-runtime/src/run.rs#L407)),
   before the owner records the end
   ([`owner/history.rs#L212-L225`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/history.rs#L212-L225)).
   So
   [`changed_at`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-runtime/src/history/ledger.rs#L161-L173)
   always falls back to `runs.at`. This puts a run's changes before its launch and steps in `--json`, against
   [`docs/history.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/docs/history.md)
   ("for `changes`, when the run ended"). Fix: give the changes record the time it was captured, and add a daemon test
   that its time is not before the run's last step.
3. **[COR, story] `agent log --team` prints `team ended` once per round.**
   [`teams.rs#L742-L745`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/teams.rs#L742-L745)
   links the end to every round, and
   [`text.rs#L88-L96`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/story/text.rs#L88-L96)
   prints each round's records under it. The test that pins "end once" passes only because its fixture links the end to
   no round
   ([`story_world.rs#L315-L319`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/tests/support/story_world.rs#L315-L319)).
   `docs/history.md` says the same ("none for the end of a team"); the daemon does not do that. No `--team` text is made
   by a daemon run. Fix: link the end to no round, or print a record linked to several rounds once, and pin a two-item
   team story.
4. **[COR, DOC-3, recorded] A partial allowance is told as "allowed by the human: the reviews go on" while the round goes back.**
   [`checks.rs#L185`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/rounds/checks.rs#L166-L192)
   records `CheckOutcome::Allowed` when *any* subject was allowed. Its doc
   ([`event.rs#L96`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/history/event.rs#L96))
   says "allowed every one of them: the reviews go on". When the human allows one issue and refuses another, the story
   says the reviews go on, then that the round was sent back. No test covers the mixed answer. Fix: record what was
   allowed and what was refused in one outcome, and test the mixed case. *(new)*
5. **[COR, item "every push"] A head pushed when the pull request is published again after the checks held it is not recorded.**
   [`publish.rs#L182-L185`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/rounds/publish.rs#L180-L210)
   — `open_pull_request` pushes, finds the draft it opened before and records neither `PullOpened` nor `PullPushed`. The
   reviews are then asked at a head the story never shows being pushed. Confirmed by reading. *(also `inv-astra`,
   `inv-gemini-argon`)*
6. **[COR, item "lost … each under its cause … a story says … why"] What this item cannot capture is not told, and some of it is not counted.**
   - The new captures (an unreadable questions file, a failed clock read) call `record_lost()`. It counts with no cause
     ([`handle.rs#L332-L336`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-runtime/src/history/handle.rs#L331-L336)),
     and it writes no `lost` note, although the database is writable then.
   - A gate that ran but whose post-command Git inspection, or the saving of its state record, fails is neither recorded
     nor counted
     ([`gate.rs#L315-L336`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/gate.rs#L313-L338)).

   The PR says best-effort tombstones were approved. That is a reason not to note what an unwritable database loses, and
   it does not cover these. *(also `inv-astra`, `inv-gemini-argon`)*
7. **[COR, DOC-3, item "merged (the merge commit)"] A merge the human makes is recorded without its commit.**
   [`merges.rs#L168-L169`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/rounds/merges.rs#L168-L169)
   says "GitHub's answer for the pull request does not say its commit". That is false: `GET /pulls/{n}` has
   `merge_commit_sha`. Every `--no-merge` round ends "merged (the merge commit was not recorded)", and the loss is not
   counted.
8. **[SEC-2] A gate's log path is not checked for secrets.**
   [`event.rs#L373`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/history/event.rs#L373)
   checks only the task and the head. 8.4 checks a launch's `working_dir`
   ([`redact.rs#L105`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-runtime/src/history/redact.rs#L99-L107)),
   so a credential in the home path reaches the database through `record.result.log`. Confirmed by reading. *(also
   `inv-astra`, `inv-gemini-argon`)*
9. **[COR, item story point 2] The reviewers are missing from "the agents by role and how each was chosen".**
   [`text.rs#L186-L228`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/story/text.rs#L186-L228)
   lists only the team's plan. How each reviewer was chosen is recorded nowhere: `review_asked` names the reviewer only.
10. **[TDD-2] Linking an event to the right round is not proven.** M2 and M4 survived. Every test has a task that works
    for one round, so "latest run" is never distinguished from "first run"
    ([`gate_rounds.rs#L33`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-runtime/tests/history/gate_rounds.rs#L33)).
    In a two-item team, the coordinator's gates, questions and controls of round 2 could go to round 1 with every test
    green. Fix: a daemon run of a two-item list, with each event asserted in its own round. *(M4 new)*
11. **[TDD-2, TDD-6] The losses this item adds are never provoked.** M5 survived: no test makes the questions file
    unreadable or the clock fail. The ground rule says "Every failure the item names is provoked". *(new)*
12. **[TDD-2, TDD-7] How long a gate took is not pinned.** The only assertion is `started_at <= gate.at`
    ([`history_events.rs#L391`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/rounds/tests/history_events.rs#L391)),
    and the goldens normalise durations. The rig's clock is injected, so the exact span can be asserted.
13. **[TDD-6] "Each new record written and read back equal" holds through the database for only `9` of the `19` event kinds.**
    The runtime tests write `Checked`, `Gate`, `Lost`, `Notified`, `PullOpened`, `PullPushed`, `PullReady`,
    `QuestionAsked` and `TeamEnded`. The other kinds are read back by the daemon tests field by field, never with long
    texts. A table of every kind, written and read back equal at and over the limit, would have caught blocker 1. *(new)*
14. **[TDD-8, PR-9] Real sleeps in this PR's own tests, with a false reason in the description.** `late:TEXT` orders the
    two reviewers with `sleep 3`
    ([`team-cli.sh#L180`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/tests/fixtures/team-cli.sh#L176-L182)),
    and the golden rig polls with `tokio::time::sleep`
    ([`history_stories.rs#L107`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/rounds/tests/history_stories.rs#L107)).
    On a host that stalls for 3 s the goldens flip order. Both came with
    [`e09d40e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e09d40eee563cb92ec21a0dab806b44b138167c0) of this PR,
    so "retained because the task explicitly requires preserving existing tests" does not hold. Fix: a file handshake, as
    `wait_for_gate` already does.
15. **[TYP-4] `HistoryEvent::Lost { what: String, cause: String }`**
    ([`event.rs#L260-L267`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/history/event.rs#L259-L267))
    — a kind and a cause stored as free text in a persisted format, which reading accepts whatever it says. Use keyword
    enums (`EventKind` / record kinds, a cause enum in `agent-core`).
16. **[TYP-4, TYP-3] `LogRequest::new(round, team, prompts: bool, no_steps: bool, json: bool)`**
    ([`log.rs#L75-L81`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/commands/log.rs#L75-L81))
    — three swappable `bool`s. `PromptView`, `StepView` and `Format` already exist.
17. **[PERF-3] Reading `n` steps of a run is `O(n²)`.**
    [`sequenced.rs#L266-L271`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-runtime/src/history/reader/sequenced.rs#L261-L273)
    searches the run's cached steps linearly for every step row. Index the cache by step number. *(also `inv-astra`,
    `inv-gemini-argon`)*

## MINOR

1. **[ERR-1]** [`notes.rs#L62`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-runtime/src/history/notes.rs#L60-L63)
   forgets every run's scope at `4096` runs, and
   [`#L80`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-runtime/src/history/notes.rs#L79-L81)
   drops the oldest pending note at `256`. Neither logs anything.
2. **[SEC-2]** `Noting` sits under the redactor
   ([`daemon/history.rs#L60`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/history.rs#L60)),
   so the `detail` of a loss note is never redacted. The error texts it holds carry no values today, but nothing keeps
   it so. *(new)*
3. **[SEC-1]** `agent log` writes stored texts raw
   ([`text.rs#L50-L57`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/story/text.rs#L49-L57)),
   so terminal control sequences in a PR body, a review report, a final message or a step reach the human's terminal.
   *(new)*
4. **[COR]** Two history-only clock reads can fail the operation itself: the gate's `started_at`
   ([`gate.rs#L306`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/gate.rs#L306))
   and the say's `said`
   ([`controls.rs#L355`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/controls.rs#L355)).
   The say's read comes after its messages were delivered. The module's own rule is that "the history never refuses or
   delays". *(new)*
5. **[COR]** `pull_closed` is recorded only for approved pull requests
   ([`merges.rs#L170-L183`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/rounds/merges.rs#L170-L184)).
   The daemon does not watch a pull request under review (8.1), so a close during review leaves nothing. This should
   be documented.
6. **[GEN-1]** [`json.rs#L19-L39`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/story/json.rs#L19-L39)
   re-declares the record keywords of `RecordKind`, and
   [`#L176-L181`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/story/json.rs#L176-L181)
   writes by hand the JSON that `Client`'s own `Serialize` gives. *(new)*
7. **[API-12]** `round_part(.., _options: StoryOptions)`
   ([`text.rs#L144`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/story/text.rs#L144))
   takes a parameter it never uses. *(new)*
8. **[story wording]**
   - An allowance is told as "checks before a review found 1 issue … `- Allow the round to change production code
     without a test of its own?`", which is a question, not an issue
     ([`story-checks.txt#L202-L204`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/tests/golden/story-checks.txt#L202-L204)).
   - The questions notification comes before "asked the human"
     ([`#L130-L135`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/tests/golden/story-checks.txt#L130-L135)).
   - The notification names its task twice.
9. **[story completeness]** A worker's first run is linked to no round, so the round story leaves it out (`10` of the
   team's `11` runs in the earlier measurement). `--team` prints it under "Team events", after `team ended`. The PR
   should say so.
10. **[TDD-1]** In RED [`1fe99b1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1fe99b17f3ab022b8e731125be4a2987c9d48b19),
    two tests fail on an index or an `unwrap`, not on their assertion (`story_records.rs` `lines[2]` and
    `find(..).unwrap()`).

## ACCEPTED DEVIATIONS

- **[ERR-3]** [`json.rs#L66-L72`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent-core/src/story/json.rs#L66-L72):
  the `expect` sits under a scoped `#[expect(clippy::expect_used, reason = ...)]`. The reason holds: an event comes from
  the ledger's JSON, and the writer refuses one that has no JSON form.
- **[TDD-14]** [`5d88ce7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5d88ce76ca6d06f85036acfb39cc274fe6cb338c):
  the table list and `user_version` `3`, which the human approved. It also changes `docs/history.schema.sql`, and the PR
  says so.
- **[TDD-14]** [`9cc791d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9cc791da471dbc2e62017c6056a8113d24e3875e):
  the test was wrong. It expected another team's records in one team's rounds.
- **[TDD-14]** [`372a960`](https://github.com/mateusz-szczyrzyca/team-agents/commit/372a960a1bcce99feeb0432acbb732e4451b9e2d):
  the fake CLI really leaves files, so the tree is not clean.
- **[TDD-14]** [`337d736`](https://github.com/mateusz-szczyrzyca/team-agents/commit/337d736f322cadb51b69ac56797fb051e9b1d916):
  adds the `lost` kind to the vocabulary tables. Every other expectation is unchanged.
- **[TDD-11]** [`89fd5ad`](https://github.com/mateusz-szczyrzyca/team-agents/commit/89fd5adf354222d1b8a32ed7fae3379f2703efef):
  an existing test changed with the human's approval, as the PR reports. The change keeps the observation that ended the
  poll, and the deadline and the termination criteria are unchanged.
- **[TDD-12]** [`gate.rs` `#[cfg(test)] run`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/gate.rs#L551-L555):
  a test-only wrapper that keeps the existing gate tests unchanged. It changes no production path.
- **[PR-1, PR-2]** `3,743` lines without the reports is within the plan's `5,000`. Splitting this late would be worse
  than finishing.

## REJECTED DEVIATIONS

- **[TDD-8]** See blocker 14. The real-time waits are this PR's own.
- **[TDD-1]** The `Test-First: not possible` reason of
  [`a70e9d1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a70e9d16cd550c257db1639aaa0528d1f44722dc) ("the new variant makes the frozen table
  out of date") does not hold. A RED commit could have added the variant as a stub together with the updated tables;
  instead, keys, texts and identities came before any test asked for them.
- **[TDD-1, TDD-3]** [`e09d40e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e09d40eee563cb92ec21a0dab806b44b138167c0)
  fixes the time of `pull_merged` ([`merges.rs#L99-L100`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8738b91d1152f252e36f9a2466d100d9beef8519/crates/agent/src/daemon/owner/rounds/merges.rs#L97-L104))
  in the same commit as the golden that showed the bug. The golden could have been committed first as a RED commit. The
  fix is covered at the head, so this asks for nothing in the code.

## NEEDS HUMAN

- The scope of the approved "best-effort loss tombstones": I cannot see the verdict. Blocker 6 assumes it covers what an
  unwritable database loses, and not what can be captured or noted while the database works.
- The change of an existing test in [`89fd5ad`](https://github.com/mateusz-szczyrzyca/team-agents/commit/89fd5adf354222d1b8a32ed7fae3379f2703efef):
  approved by the human according to the PR; I did not see the answer file.
- The two rejected `Test-First` reasons above: process only, since both behaviours are tested at the head. The human
  decides whether to accept them.
- The real sleeps of blocker 14: replace them, or waive them explicitly.

## What is good

- One ledger with a write sequence. The SQL `CHECK`s of kind and payload refuse bad rows when written, and the reader
  refuses them again. Migration `3` keeps 8.4 histories readable through temporary views, without changing the file.
- The story is a pure function (`selected` / `render` / `json_lines`). The text and `--json` share one selection, and
  seeded tests cover the order, a permutation of the input and two teams at once.
- Seven goldens come from real daemon runs. `check_story` compares every stored prompt with what the fake CLI got, and
  `--json` lines with an SQL count.
- `agent log` needs no daemon, and an unknown ID has its own exit code `3`.

## Not done

- No real model CLI and no suite of the old implementation was run.
- Blockers 5 and 8 and the PERF-3 finding were confirmed by reading only; the other reviewers' probes were not repeated.
- This branch changes no code; the report is its only change.
