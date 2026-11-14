# Review of PR #101 against item 8.5 — `inv-opus`

PR: [#101](https://github.com/mateusz-szczyrzyca/team-agents/pull/101), "Item 8.5: the history of a pull request, and the
story of a round". Reviewed head: [`311120f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/311120fac52d5f7f0716961ea60eb9e502878180)
(the PR's `headRefOid`, also the base of this branch). Measured against item 8.5 of `rounds/round-8.5.md`, the developer
rules (`prompts/agent/rust-developer.md`) and `AGENTS.md`.

**Verdict: REQUEST_CHANGES.** Most of the item is there, and done carefully. The gate is green and the item's two checks
pass when I run them by hand. But two recording/story defects show up in real daemon data: a run's changes get the wrong
time, and a team's end is printed once per round. The loss rule of the item is only partly met. Two of the three
behaviours I broke on purpose went unnoticed by the tests.

## Checks

| What | Result |
|---|---|
| `GATE_WAIT=1 make gate` on `311120f` | **PASS**, `72` s (`rust` step: fmt, clippy `-D warnings`, tests, docs) |
| PR-1 size command against `origin/main...HEAD` | `4,084` lines (limit `5,000`; matches the PR description) |
| Item check 1: a team round with one fix pass, daemon stopped, `agent log --round <id>` with the real binary | passes: item, agents by role, every run with its prompt's first lines and steps, files per run, two review passes with verdicts, the merge |
| Item check 2: `--prompts` prints the reviewers' second prompts whole | passes (`30,066`-byte prompts printed between markers) |
| Item check 2: `--json \| wc -l` equals the round's records in the database | passes: `81` = `81` (independent SQL count) |
| Unknown round | `agent: the history has no round …`, exit `3` |
| No history file in `AGENTS_HOME` | exit `1`, message names the path, no file is created |
| Golden-story tests (`history_stories`), run `3` times | `7/7` passed each time |
| RED commit [`1fe99b1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1fe99b17f3ab022b8e731125be4a2987c9d48b19) checked out and run | `story_records`: `1` passed, `8` failed, as its message says |
| TDD-14 freeze: each RED commit's test files diffed to `HEAD` | only `use` lines and the listed `TEST-FIX` commits change them |

How I ran item check 1: I temporarily patched
`a_team_round_with_a_worker_one_fix_pass_and_a_second_review_pass_that_approves_is_told_whole` (not committed) to copy
its `history.db` out after `rig.stop()`. Then I ran `target/debug/agent log` with `AGENTS_HOME` set to a fresh
directory that held only that file.

## Behaviours broken on purpose (review step 6)

Each mutation was reverted with `git checkout` before the next one.

| # | Mutation | Where | Result |
|---|---|---|---|
| 1 | The gate event records `started_at: record.result.finished_at`, so every gate takes no time (recorded) | [`crates/agent/src/daemon/gate.rs#L447-L458`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/gate.rs#L447-L458) | **survived**: `cargo test -p agent` all green (`709` + the integration tests) |
| 2 | The story orders a tie by the reverse write order (`Reverse(record.sequence)`) | [`crates/agent-core/src/story.rs#L55-L75`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-core/src/story.rs#L55-L75) | killed: `3` tests of `story_records` fail |
| 3 | The ledger links a gate to the rounds of the task's *oldest* run (`ORDER BY at ASC, rowid ASC`) instead of the latest (recorded) | [`crates/agent-runtime/src/history/ledger.rs#L147-L159`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/history/ledger.rs#L147-L159) | **survived**: `cargo test -p agent-runtime -p agent` all green |

## Findings

### Major

1. **[correctness, recorded] Every `changes` record gets the time its run started, not the time it ended.** The run
   engine records `Changes` when the process exits
   ([`crates/agent-runtime/src/run.rs#L498-L503`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/run.rs#L498-L503)).
   The owner records `RunEnd` after that
   ([`crates/agent/src/daemon/owner/history.rs#L219`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/history.rs#L219)).
   The ledger's
   [`changed_at`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/history/ledger.rs#L162-L174)
   takes the run end's time if one is recorded, and the run's start time otherwise. Because the end is never recorded
   yet, it always falls back to the start. In the database of the team-fix-pass round, all `11` `changes` rows have
   `at` equal to `runs.at` and a sequence below their `run_end`. This breaks the documented contract
   ([`docs/history.md#L195`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/docs/history.md#L195):
   "for `changes`, when the run ended"). It also puts every run's changes *before* its launch and steps in `--json`,
   whose order is time order. Story point 4 asks for "after each run its changes". The text story hides this because
   it groups records by run. Fix: give the changes record its own time, or record the end first, and add a daemon
   test that asserts the `changes` time equals the `run_end` time.

2. **[correctness, story] `agent log --team` prints "team ended" once per round of the team.** The daemon links
   `TeamEnded` to every round of the team
   ([`crates/agent/src/daemon/owner/teams.rs#L742-L745`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/teams.rs#L742-L745)).
   The team story prints each round's linked records under that round
   ([`crates/agent-core/src/story/text.rs#L88-L96`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-core/src/story/text.rs#L88-L96)).
   The same happens to any other record linked to two rounds. The test
   `the_story_of_a_team_tells_its_rounds_in_order_and_the_team_start_and_end_once` passes only because its fixture
   links `TeamEnded` to no round
   ([`crates/agent-core/tests/support/story_world.rs#L315-L319`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-core/tests/support/story_world.rs#L315-L319)),
   which is not what the daemon does. `docs/history.md` also says the end of a team has no rounds
   ([`#L340`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/docs/history.md#L340)).
   I reproduced it with a scratch test (not committed): two rounds, `TeamEnded` linked to both as the daemon links
   it. `text.matches("team ended\n").count()` was `2`. No golden covers a team with two items, so nothing catches it.

3. **[item gap] A lost record is said in its place only when the database refused it.** The item says: "a story says,
   in the place of what is missing, that it was lost and why". Only records that SQLite refuses when written alone
   get a `lost` note
   ([`crates/agent-runtime/src/history/notes.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/history/notes.rs)).
   Two kinds of loss leave the story silent:
   - The new capture failures of this item: an unreadable questions file, or a clock that cannot give a time. They
     call `record_lost()`
     ([`crates/agent/src/daemon/owner/history.rs#L17-L28`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/history.rs#L17-L28),
     [`#L71-L84`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/history.rs#L71-L84)).
     That bumps the total but reports **no cause**
     ([`crates/agent-runtime/src/history/handle.rs#L331-L336`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/history/handle.rs#L331-L336)),
     although the item asks for "each under its cause".
   - Queue-full and secret-identity drops: they are counted, but nothing appears in the story.

   The PR says the human approved "best-effort loss tombstones". The contract proposed to the human was broader. The
   Phase 1 report
   ([`docs/investigations/round-rust-agent-8-5/inv-kimi-deepinfra.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/docs/investigations/round-rust-agent-8-5/inv-kimi-deepinfra.md),
   points 2 and 3 of "Loss") recommended notes "for queue overflow, individual write failures, secret-identity omission
   and failed captures". It also recommended that an absence with no note says "missing; no stored record / cause
   unknown". Unless the human's verdict narrowed this, the captures and queue overflow still need a note. At minimum,
   the new 8.5 captures should report a cause.

### Medium

4. **[item gap, recorded] A merge the human makes is recorded without its merge commit.**
   [`crates/agent/src/daemon/owner/rounds/merges.rs#L168-L169`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/rounds/merges.rs#L168-L169)
   records `Merged(None)`. Its comment says "GitHub's answer for the pull request does not say its commit". That is not
   true of GitHub: `GET /repos/{owner}/{repo}/pulls/{n}` returns `merge_commit_sha`. The client simply does not decode
   it ([`crates/agent-runtime/src/github.rs#L1720`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/github.rs#L1720)).
   So every `--no-merge` round ends with "merged (the merge commit was not recorded)". The golden pins that text
   ([`crates/agent/tests/golden/story-no-merge.txt#L142`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/tests/golden/story-no-merge.txt#L142)).
   The missing commit is not counted as lost and its cause is not told. The item asks for "merged (the merge commit)".

5. **[tests, TDD-2 / TDD-7] How long a gate took is not proven** (mutation 1). The only assertion is
   `assert!(*started_at <= gate.at)`
   ([`crates/agent/src/daemon/owner/rounds/tests/history_events.rs#L391`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/rounds/tests/history_events.rs#L391)),
   and the goldens normalise durations. The rig's clock is injected and ticks, so an exact or strict assertion is
   possible.

6. **[tests, TDD-2] Linking a gate to the task's *latest* run is not proven** (mutation 3).
   `a_gate_with_no_rounds_belongs_to_the_rounds_of_the_latest_run_of_its_task`
   ([`crates/agent-runtime/tests/history/gate_rounds.rs#L33`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/tests/history/gate_rounds.rs#L33))
   builds a world with one run per task, so "latest" and "oldest" are the same run. In a team with two items, a
   regression would put the coordinator's second gate in the first round.

### Minor

7. **[TYP-4] `HistoryEvent::Lost { what: String, cause: String }`**
   ([`crates/agent-core/src/history/event.rs#L260-L267`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-core/src/history/event.rs#L260-L267)).
   A kind and a cause are stored as free strings, in a persisted format that accepts any value on read. `RecordKind` /
   `EventKind` and the writer's cause enum already exist.
8. **[TYP-4] Three `bool` parameters in `LogRequest::new(round, team, prompts, no_steps, json)`**
   ([`crates/agent/src/commands/log.rs#L75-L81`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/commands/log.rs#L75-L81)).
   `PromptView`, `StepView` and `Format` already exist and could be passed in instead.
9. **[item, story point 2] The reviewers are not among "the agents by role and how each was chosen".** The "Agents"
   part lists only the team's plan
   ([`crates/agent-core/src/story/text.rs#L186-L228`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-core/src/story/text.rs#L186-L228)).
   The reviewers appear only in "asked of …" and in their runs, and how they were chosen is not recorded. This should
   be recorded, or the PR should say why not.
10. **[story completeness] The round story leaves out a run that the team ran during the round.** The worker's first
    run is linked to no round (round linking from 8.4). The team-fix-pass round therefore shows `10` of the team's `11`
    runs (checked in its database). `--team` prints that run in a trailing "Team events" part, after "team ended",
    out of time order. The story is not wrong, but the PR should state it.
11. **[item] "closed" is noticed only for approved pull requests.** `PullClosed` is recorded only in `try_merge`
    ([`merges.rs#L170-L183`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/src/daemon/owner/rounds/merges.rs#L170-L184)).
    A pull request the human closes during review leaves no record. This limit is not documented.
12. **[TDD-8, deviation's reason] The real-time waits were added by this PR.** They are `late:` (`sleep 3`) and
    `wait_for_gate` (`sleep 0.1` polling)
    ([`crates/agent/tests/fixtures/team-cli.sh#L118-L125`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/tests/fixtures/team-cli.sh#L116-L125),
    [`#L178-L182`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent/tests/fixtures/team-cli.sh#L178-L182)).
    They came with [`e09d40e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e09d40eee563cb92ec21a0dab806b44b138167c0).
    The PR says they were "retained because the task explicitly requires preserving existing tests", which does not
    apply to tests the PR itself added. The order of the reviewers' verdicts in the goldens depends on the 3-second
    margin. It was stable in my `3` runs.
13. **[TDD-1]** In RED [`1fe99b1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1fe99b17f3ab022b8e731125be4a2987c9d48b19), two tests pass
    against the stub (the commit says so). Two others fail on an index or `unwrap`, not on their assertion:
    `story_records.rs:230` (`lines[2]`) and `:260` (`find(...).unwrap()`).
14. **[notes.rs, ERR-1]** Notes are dropped without a log line in two places
    ([`notes.rs#L60-L63`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/history/notes.rs#L60-L63),
    [`#L79-L81`](https://github.com/mateusz-szczyrzyca/team-agents/blob/311120fac52d5f7f0716961ea60eb9e502878180/crates/agent-runtime/src/history/notes.rs#L79-L81)):
    - At `4096` runs, the store forgets where *every* run belongs (`scopes.clear()`), so a later loss of a run still
      going cannot be placed.
    - At `256` pending notes, the oldest is dropped (`pending.remove(0)`).
15. **[TDD-14 / PR-2, both disclosed]**
    - `TEST-FIX` [`5d88ce7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5d88ce76ca6d06f85036acfb39cc274fe6cb338c)
      also changes `docs/history.schema.sql`.
    - At `4,084` lines the PR is above the ~`4,000`-line split threshold, and no split was planned before coding.
16. **[cosmetic]**
    - The questions notification reads "claude-1 has questions (`<task-4>`) (task `<task-4>`)": the task is named twice.
    - In `story-question-answered.txt` the notification (line `44`) comes before "asked the human" (line `49`), because
      the question is timed after the instructions are added to the file.

## What is good

- One ledger with a write sequence. Events are stored as JSON with SQL checks of kind and payload, refused on write and
  again on read. A migration to schema `3` keeps the 8.4 histories readable through temporary views.
- The story is a pure function in `agent-core` (`selected` / `render` / `json_lines`). The text and the JSON share one
  selection, and seeded property tests cover order, permutation and disjoint teams.
- Seven goldens come from real daemon runs with fake CLIs and a fake GitHub. `check_story` compares every run's
  stored prompt with what the fake CLI received, and counts the `--json` lines against independent SQL.
- `agent log` works with no daemon. It has its own exit code (`3`) for an unknown ID, and a test proves that a
  database written by 8.4 alone is read and left unchanged.
- A secret in a review report and in fix-pass findings is searched for in the SQLite and WAL bytes.

## Not done

- I did not run real model CLIs or any suite of the old implementation.
- I could not read the human's verdict on the loss contract (finding 3). I judged it from the PR text and the
  Phase 1 report.
- Nothing in this branch changes the PR's code. This report is the only commit.
