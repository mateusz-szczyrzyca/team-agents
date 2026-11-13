# Item 7.1 — the red gate of PR #94, and what its re-review needs (inv-opus)

Report of the investigator `inv-opus` for the round `rust-agent/7.1`, an investigation on the open PR
[#94](https://github.com/mateusz-szczyrzyca/team-agents/pull/94). Branch `agents/inv-opus/20261112-232419`, started from
the PR's head [`108b9bb`](https://github.com/mateusz-szczyrzyca/team-agents/commit/108b9bbccb2e077b7b11e792e7d815e099888361).
No pull request: the merger integrates the independent results.

## Summary

- The only review of PR #94 (`inv-kimi`) stopped at a red gate: `8` tests of the legacy Bats suite time out. **PR #94 does
  not cause them.** They fail on `main` too, the same `8`, since
  [`3655000`](https://github.com/mateusz-szczyrzyca/team-agents/commit/36550002f0c3472c15f74537da7d3210eda6f424)
  (Nov 11), which changed how a team of **one** investigator runs and did not run these tests.
- The fix is one commit on this branch,
  [`5110cde`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5110cde014b8b28fca68ee56d854aa483f2e7ee0): a
  team of one runs its merge phase again, as the `8` tests specify. No test or stub is changed. With it the whole
  legacy suite passes (`822/822` on `main`), and so does the full gate of this branch (see *Checks run*).
- The Rust work of PR #94 was not reviewed on its merits yet. I checked its size and the item's checks. Two things
  will stop the next review unless the merger fixes them in the PR description (not in the code):
  1. The PR description has none of the sections the rules require. Its deviations are recorded only in the report
     file, and an unrecorded deviation is a defect. A draft is under *A description for PR #94*.
  2. Check 3's "next assignment" cannot happen in a real 7.1 run. The ground rules require its reason in the PR
     description (*Check 3* below).
- **An incident I caused, already repaired:** my first `git bisect run` re-initialized the shared repository as bare
  (`core.bare=true`). For about 20 minutes, `/home/dev/projects/team-agents` refused every command that needs a work tree.
  Details and what I checked are under *Incident*.

## The red gate

### What fails

`GATE_WAIT=1 make unit-bats` on `main` ([`e938575`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e9385750e69cdd88970dacd8cf54433d4feed85e)):
exit `2`, `814` ok, `8` not ok — exactly the `8` of the review:

- `agent status and show name independent investigators and the PR comment names --investigation` (`investigation_independent_labels.bats`)
- `single investigator is accepted and investigation-team task is created` (`investigation_single_investigator.bats`)
- `single investigator with whitespace is trimmed and accepted` (`investigation_single_investigator.bats`)
- `sole investigator completes merge review and notification without a decision wait` (`investigation_single_pipeline.bats`)
- `sole investigator review without a verdict still fails the completed flow` (`investigation_single_pipeline.bats`)
- `--team with a single investigator starts the requested investigation cohort` (`investigation_team_cli.bats`)
- `agent run --investigation records the round in the investigation team and in each of its phases` (`round_id.bats`)
- `a team run and an investigation team of round R20: every task with its role, its round and the right sums` (`round_team_run.bats`)

All of them run an investigation team of one investigator and wait for its parent task to be `done`. Each stays
`running`, in phase `2`. It fails the same way serially (`BATS_JOBS=1`) and at PR #94's head: a deterministic
failure, not load.

### When it started

I bisected `bats -f whitespace tests/unit/investigation_single_investigator.bats` (one test, about `35` s):

- good: [`03da11a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/03da11a50a427e153d9d637db62321985fd1e41e) (Nov 2, where the test was made to pass)
- good: [`21f208d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/21f208d000c0c580edec7b93af4018f228056a0a) (the RED commit of the fix loop)
- **first bad:** [`3655000`](https://github.com/mateusz-szczyrzyca/team-agents/commit/36550002f0c3472c15f74537da7d3210eda6f424) "An investigation fixes what its review asks for, on its one PR (green)"

### Why

Besides the fix loop, `3655000` made a team of one skip its merge run. Phase 2 called `investigation_solo_owner`, which
recorded the phase-one task as the merger, set a stage and returned. The PR was then to be opened by a later pass of
`investigation_pipeline_tick` (`investigation_solo_pr`, "the next pass finds it"). Such a pass comes only from a
dispatch:

- In production, the `_supervise` timer dispatches, so the flow waits for the timer instead of going on.
- In the Bats suite there is no timer, so the flow never goes on.

The tests also specify the old flow itself. The fake pipeline CLI (`tests/unit/stubs/fake-investigation-pipeline-cli`)
creates PR `#73` in its **phase-2 merge run**, and the stub `gh pr create` records no PR. So even a pipeline that kept
going would loop on opening the PR. The commit's message lists the Bats files it ran; these are not among them.

### The fix (commit [`5110cde`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5110cde014b8b28fca68ee56d854aa483f2e7ee0))

Rule 1 of `AGENTS.md` says the code is wrong until proven otherwise, and no test or human decision supports the
change for a team of one. So I restored what the tests specify:

- `bin/agent`: the phase-2 line that called `investigation_solo_owner` is removed, so a team of one gets its merge run
  (`investigation_phase_choose … 2`) as before.
- `bin/agent`: `investigation_solo_owner`, which only that line called, is removed. Its log event
  `investigation_solo_owner` existed only since Nov 11 and is named nowhere else (README, docs, tests).
- `README.md`: two sentences said a team of one has no merge phase. They now say its one investigator is the merger.
- Unchanged:
  - the fix loop of `3655000`: the merger, here the one investigator, is sent back with the findings;
  - `investigation_solo`, and the `investigation_solo_pr` fallback (the script opens the PR when a team of one's merge
    run published but opened none).

Results:

- the `8` tests: fail on `main`, pass with the fix (`36/36` in their `7` files, `investigation_fix_loop.bats` included);
- the whole legacy suite on `main` with the same one-line change: `822/822`, exit `0`, in `3:31` min;
- shellcheck on `bin/agent`: `0` findings.

**For the human — the alternative:** if a team of one should really have no merge run (one model run fewer), the `8`
tests and the fake pipeline CLI must change to that design. Only you may allow that (rule 1). The pipeline also needs
two fixes:

- go on in the same pass after `investigation_solo_owner`;
- take the PR's URL from the output of `gh pr create` instead of waiting for "the next pass".

Then `5110cde` is to be dropped.

**For the merger:** `5110cde` is independent of item 7.1 and touches only `bin/agent` and `README.md`. It can stay in
PR #94 (its gate needs it), or go to `main` on its own and PR #94 be rebased. That is the review's own suggestion, and
the cleaner one by PR-7 (one purpose per PR).

## Incident: my bisect changed the shared repository's configuration

`git bisect run` exports `GIT_DIR` (and `GIT_PREFIX`) to the script it runs. I checked this afterwards:
`GIT_DIR=/home/dev/projects/team-agents/.git/worktrees/inv-opus-main`. The Bats setup's own git commands then
worked on the real repository instead of the temporary one.

- `git init -q --bare "$HOME/remote.git"` re-initialized the shared repository and set **`core.bare=true`** in
  `/home/dev/projects/team-agents/.git/config`. The main checkout then answered `fatal: this operation must be run in
  a work tree`, from about `23:30` to `23:51`.
- `git commit --allow-empty -m init` made commits on the detached `HEAD` of my scratch worktree only. They are not on
  any branch.
- `git remote add origin` failed (`origin` exists), so the setup stopped there: no push, no `insteadOf` rule and no
  `set-url` ran.

What I checked and did:

- Before the change: `main@{0}` is a `git am` at `23:20:16`, which needs a work tree. The config was written at
  `23:49:35`, during the bisect. I set `core.bare = false` again; the diff of the config against its copy from
  before the repair is that one line.
- `git -C /home/dev/projects/team-agents status` works again. Its four modified `rounds/*.md` files are from
  `15:38`, before this task.
- Not changed:
  - the URL of `origin`, and no `insteadOf` rule added;
  - local `main` and `origin/main` (both `e938575`);
  - no `agents/{inv,solo,lead,member,worker}/…` branch, locally or on GitHub;
  - `info/exclude` (from Nov 5).
- The bisect was repeated with a script that unsets `GIT_DIR` and `GIT_PREFIX` and stops if the shared config's
  checksum changes. It did not change again.
- The first bisect steps ran `bats` outside the machine's test lock. From then on every step took the lock
  (`tools/test-lock.sh`, `TEST_LOCK_WAIT=1`).

## Review of the Rust work of PR #94

Not a full review, which is the reviewers' job. I looked at what decides whether the next review can get past its
first steps.

### Size

The PR-1 command at the PR's head, `8f9f3c7...108b9bb`, the investigation report left out: `3,439` changed lines
outside tests. With `5110cde`: `3,460`. Limit `5,000`.

### The item's checks, and where they are shown

| Check | Tests (all run by `make rust`, in the gate) |
|---|---|
| `agent run a "text"` starts a solo task | `team_process` (the real binary) and `daemon::owner::teams::a_solo_task_is_started_runs_to_done_and_its_team_is_closed_and_told_to_the_human` |
| `agent run a b -f list.md` starts a team whose coordinator follows the rules | `team_process`; `daemon::owner::teams::an_investigator_in_the_team_coordinates_and_an_explicit_coordinator_wins`; `daemon::team::form` tests |
| a worker, or too many investigators, is refused before anything starts | `a_run_with_too_many_investigators_for_two_outside_reviewers_is_refused_before_anything_starts`, `a_run_with_a_worker_as_coordinator_or_only_workers_is_refused_before_anything_starts` (both check that no team, worktree or branch is left) |
| a member's DONE → integration and the next assignment, shown by `agent status` | `a_member_that_is_done_tells_the_coordinator_which_wakes_to_integrate_its_branch`; the next assignment only from a seeded state — see below |

### Check 3: the next assignment cannot follow a member's DONE in 7.1

`decide::assign::next_assignments` (item 3.1) counts a member as busy while its item is `InProgress` or `Fixing`.
After a member's `DONE` its item stays `InProgress`. The next phase is `InReview { pr, … }`, and only 8.1 opens a pull
request. So in a real 7.1 run, a member that finished gets no next item: it waits until 8.1 moves its item on. The
tests show the next assignment from a state seeded with the item `InReview`.

The first report says this under *Not done*, but it is not in the PR description. The ground rules: "a check that
cannot pass yet needs its reason in the PR description". This is a limit of the state model of 2.2 (no phase
"integrated, waiting for its PR"), not a defect of 7.1. It needs to be said, not coded around.

### The PR description

PR #94 was opened by the script's one-investigator path (`investigation_solo_pr`). Its description is the task block and
one line ("A team of one investigator has nothing to merge…"). It has none of §17's sections. The deviations are listed
only in [`docs/investigations/round-rust-agent-7-1/inv-claude-sonnet.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/108b9bbccb2e077b7b11e792e7d815e099888361/docs/investigations/round-rust-agent-7-1/inv-claude-sonnet.md):

- `run` outside `Command`;
- a catalog of its own for the team texts;
- every task starting at once;
- bus failures as specified fallbacks;
- reason strings in `.rs` files;
- three tests written after their code.

The rules say "an unrecorded deviation is a defect". The script's "Round size" line also says `0 lines (+0 -0) in 0
files`, which is wrong (`3,439` by PR-1). That is a defect of the script's counting on a PR opened this way; I left it
alone (*Out of scope*).

### A description for PR #94

For the merger to put under the task block (the script keeps the task block):

```markdown
## What and why

Item 7.1: `agent run` and the team's life — starting a solo task or a team (plan, agentbus team, worktrees and
branches, first assignments), and keeping it going: a member's DONE tells and wakes the coordinator, the next items
go to free members, wake-ups from the bus, `delegate` requests served by the first free worker in the configured
order with the result returned to the requester, and a finished team closed with the human notified.

Also: commit 5110cde restores the merge run of a one-investigator team in `bin/agent`. `main` fails 8 legacy Bats tests
since 3655000, which made this PR's gate red. It can move to its own PR on `main` instead.

## Size

3,460 changed lines outside tests by the PR-1 command (relaxed limit 5,000).

## Tests

See `docs/investigations/round-rust-agent-7-1/inv-claude-sonnet.md` ("Item checks"): unit tests of every decision,
owner tests with fake CLIs, the real bus and git, and process tests of the real binary (`crates/agent/tests/team_process.rs`).
Check 3's "next assignment" is shown from a state whose item is `in_review`: in 7.1 an item stays `in_progress` after
its member's DONE until 8.1 opens its pull request, and only then is the member free again (3.1's rule).
No existing test was modified or removed.

## Gate

`GATE_WAIT=1 make gate` at the head of this branch: PASS (see the inv-opus report).

## Deviations

1. `run` is not a variant of `Command`: `crates/agent/src/tests/commands.rs` matches `Command` exhaustively, so a new
   variant needs a change to that test (rule 1). `run` is added when the command line is parsed (`Parsed::Run`).
2. The team texts are a catalog of their own (`TeamPrompt`, `TeamPrompts`): the catalog test of `prompts/harness/run/`
   compares the whole `HarnessPrompt` catalog with that directory.
3. Every task of a team starts at once: the bus wakes a member that never ran because of its unread `team_created`.
4. Bus failures are specified fallbacks: what the bus does not take stays in an outbox and is tried again (tested);
   a refusal for good is logged at `error` and dropped.
5. Short reason strings ("the worker ended with STATUS: FAILED") are values of a template variable in `.rs` files.
6. Three tests were written after their code (`is_transient`, the item whose branch cannot start, `item_in_flight`).
No lint suppression, no new dependency, no skipped test, no change of a persisted format or of the protocol.

## Out of scope

- No wake-loop guard; a request being served when the daemon stops is not returned; `helper-next` is not per team.
- The script's "Round size" line of this PR counts 0 lines.
```

## Checks run

All in this worktree or in a scratch worktree of `origin/main` under `/tmp` (removed at the end), every test run under
the machine's test lock except the first bisect steps (see *Incident*):

```bash
GATE_WAIT=1 BATS_JOBS=1 BATS_FILES=tests/unit/investigation_single_investigator.bats make unit-bats  # PR head: 2 not ok; main: 2 not ok
git bisect start 3655000 03da11a && git bisect run /tmp/inv-opus-one.sh  # first bad: 3655000
GATE_WAIT=1 make unit-bats            # main e938575: exit 2, 814 ok, 8 not ok
GATE_WAIT=1 make unit-bats            # main + the one-line fix: exit 0, 822 ok
GATE_WAIT=1 BATS_FILES="<the 7 files>" make unit-bats   # this branch with the fix: exit 0, 36 ok
shellcheck -S warning bin/agent       # 0 findings
GATE_WAIT=1 make gate                 # this branch at 5110cde: PASS, 9/9 steps
```

Gate at [`5110cde`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5110cde014b8b28fca68ee56d854aa483f2e7ee0),
`GATE_WAIT=1 make gate`: **PASS**, all `9` steps, recorded by `agent gate`:

- `check-generated`, `lint-shell`, `lint-go`, `lint-docs`: ok, `0` findings
- `unit-go`: ok
- `unit-bats`: ok, `822` ok, `0` not ok
- `harness`, `unit-agent-bats`: ok
- `rust`: ok — fmt, clippy `-D warnings`, `1,383` tests passed, `0` failed, docs with warnings denied

## Not done

- No full review of the Rust code on its merits: the reviewers do that once the gate is green.
- `5110cde` is not on `main`: where it goes is the merger's (or the human's) decision, above.

## Suggestions for `AGENTS.md` (not changed)

- Under "Workflow and checks": **never run the Bats suite from `git bisect run`, or with `GIT_DIR` set** — the setup's
  `git init --bare` then re-initializes the real repository as bare. A bisect script must `unset GIT_DIR GIT_PREFIX`
  first. (`tests/unit/helpers.bash` could unset them itself; that is a change of a test helper, the human's to allow.)
- A change to the investigation pipeline should run every `investigation_*.bats` file, plus `round_id.bats` and
  `round_team_run.bats`, before it is committed. `3655000` listed the files it ran, and the failing ones were not among
  them.

## For the notebook

(`notebook_append` is not available in this phase.)

- `git bisect run` exports `GIT_DIR`, so Bats tests run from it touch the real repository.
- The investigation pipeline goes on only in a dispatch: at a run's end, an `agent` command, or the `_supervise`
  timer. A step that starts nothing and returns waits for the timer in production, and forever in the Bats suite.
- Item 7.1 and the state model: a member is free again only when its item is `in_review` (a PR exists), so "the next
  assignment after a member's DONE" depends on 8.1.

## Review pass 1: the findings of inv-claude-sonnet, fixed

The [review](https://github.com/mateusz-szczyrzyca/team-agents/pull/94#pullrequestreview-5883884204) asked for
changes: `3` major and `7` minor findings. What was done with each:

### Major

1. **Requests of other teams taken by `helper-next`.** Fixed: a look at a team holds another team's request, reads
   the queue on for its own, and puts the held ones back on the bus before the next team is looked at. A put-back the
   bus refuses goes to the outbox, as before. I did not take the review's way (`helper-list`, then
   `helper-set --status running`): `helper-list` prints text for people, not JSON, and `helper-set` cannot set the
   worker, which the MCP tools show to agents; `agentbus` stays as it is.
   $\color{red}{\textbf{RED}}$ [`af74bea`](https://github.com/mateusz-szczyrzyca/team-agents/commit/af74bea70db536c772c5c4129c46831ccc3eb59e) (two teams on one bus; the
   later team's request first: timed out after `60` s) → $\color{green}{\textbf{GREEN}}$
   [`bb2b8fd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/bb2b8fd8e67ed1ca54cfc54d9c654b7e4048e4a2).
2. **A run that cannot start leaves the team stuck.** Fixed: `fail_to_start` ends with `after_run`, as `failed` does.
   $\color{red}{\textbf{RED}}$ [`785bc03`](https://github.com/mateusz-szczyrzyca/team-agents/commit/785bc03aad47073345d49a59727329cb629b841c) (a worker and a member whose
   worktree is gone: both timed out) → $\color{green}{\textbf{GREEN}}$
   [`9de6e00`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9de6e00f29c3996c52e5f856ffd26690d291f83b).
3. **`HOW_*` sentences in `react.rs`.** Fixed: `RequestOutcome::Failed` carries a `WorkerEnd` and what the worker
   said; each `WorkerEnd` has its sentence in `prompts/harness/team/worker-*.md` (five new `TeamPrompt` entries), and
   `failure_reason` renders it. `a_worker_that_failed_returns_how_and_what_it_said`, a test of this PR (not on `main`),
   now compares the whole outcome.
   $\color{red}{\textbf{RED}}$ [`07ccd85`](https://github.com/mateusz-szczyrzyca/team-agents/commit/07ccd85691823b82588ccc7cd5cfa70c7d36aec1) →
   $\color{green}{\textbf{GREEN}}$ [`ffa5274`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ffa52740faea3110c576aa8497a02f8cce798fec).

### Minor

1. **PR description.** Rewritten below the task block: deviations, the `bin/agent` commit, check 3, files outside the
   item's list.
2. **`run` outside `enum Command`.** Not changed: it needs the human's permission to change one line of
   `crates/agent/src/tests/commands.rs` (rule 1). Listed in the PR as a question for the human.
3. **The rule of `context.md` without a test.** Pinned:
   [`6ca9f35`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6ca9f357c71c6e31b17350f9ed26ecc4f4d50298). Without the line the test fails.
4. **Untested error variants.**
   - `FormError::IdCollision`: tested with a generator that always gives the same number,
     [`9f8100a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9f8100af5e4da1cf5e20a64902ac3f0521d467f5). With the check turned off the test fails.
   - `StartProblem::RepoGone`: tested by a restart without the repository in `agents.toml`,
     [`0b2fb17`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0b2fb174084252d1a72cd6014361e5531e58c1ed). The daemon's log shows that variant.
   - `DaemonError::HomePath`: removed, because nothing can reach it. `Home` keeps the `AbsolutePath` it is made from
     and hands it to the bus, [`e8e5114`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e8e51148b001ccbe9f03075f9ab779c8ec5eac74).
   - `AssignmentError::UnknownMember`, `FormError::NoSuchItem`, `FormError::Advance`: not tested. They come up only
     if agent-core's `next_assignments` names a member or an item it was not given. No input reaches them through the
     public API, and testing them would need a seam only for tests (TDD-12).
5. **The `origin` URL in an error.** Fixed: the user and password of `scheme://user:password@host/` are left out.
   $\color{red}{\textbf{RED}}$ [`3d27d8f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3d27d8fca38729e53be724f5d992e44d27ff7862) →
   $\color{green}{\textbf{GREEN}}$ [`e852ceb`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e852ceb351187f356edb938525d1bab02e252485).
6. **The owner's loop waits for `agentbus` and `git`.** Not changed in this round; written down as a known limit in
   `docs/architecture.md`, [`c4634b7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c4634b7430591e091f1d0a9a82039eecddbb3798).
7. **State kept only in memory.** Not changed; the known limits now also say that why a queued run starts is lost at a
   stop (same commit). Rebuilding the `member_finished` notice at start is left for a later item.

### Checks run in this pass

```bash
GATE_WAIT=1 make gate     # at c4634b7: PASS, all 9 steps
cargo test -p agent --bin agent -- the_requests_of_two_teams      # RED: FAILED twice (60 s time-out); GREEN: ok 5 times out of 5
cargo test -p agent --bin agent -- whose_run_cannot_start         # RED: 2 failed; GREEN: ok
cargo test -p agent --bin agent -- repository_left_the_configuration   # ok 3 times out of 3
cargo clippy --workspace --all-targets --locked -- -D warnings    # 0 findings
```

Every test run went through `tools/test-lock.sh`. Size by the PR-1 command (tests and `docs/investigations` left out):
`3,585` changed lines against `main`, `293` of them from this pass (limit `5,000`).

## Review pass 2: the findings of inv-kimi-openrouter, fixed

The [review](https://github.com/mateusz-szczyrzyca/team-agents/pull/94#pullrequestreview-5884313167) asked for changes:
`7` major findings, `0` minor. All `7` are fixed.

1. **Pending items were not tried again after a branch failure.**
   - Fix: every look at a team now gives out items, the clock's looks too. A failed branch switch is a warning, tried again at the next look.
   - The kind of look (`Look::Event` / `Look::Poll`) no longer changes anything, so it is gone.
   - Commits: $\color{red}{\textbf{RED}}$ [`b25392a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b25392ac33645602229a589a88f64e3c54fcf0da) → $\color{green}{\textbf{GREEN}}$ [`f515c04`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f515c0472c1ed17adf4ad016cab9ed85055927d9). The RED test was checked to fail at its own commit (time-out after `60` s).
2. **A team that cannot be recorded left its worktrees, branches and bus team behind, and the daemon ended.**
   - Fix: the team goes into a copy of the state before anything is made. The copy is saved after the worktrees and the bus team. When the save fails, the worktrees and branches are removed, the bus team is marked done, and the run is refused; the daemon goes on.
   - Commits: $\color{red}{\textbf{RED}}$ [`dc98312`](https://github.com/mateusz-szczyrzyca/team-agents/commit/dc98312ec8b1be10588655a693fead14d3920c1b) (a read-only home: the daemon ended) → $\color{green}{\textbf{GREEN}}$ [`0c0ee3f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0c0ee3f2f5506c98a71a89f719132dec42b015e4). The test fails with the clean-up left out.
   - Not tested: the refusal of a team the state refuses, because it needs a task ID that is already in use, and the daemon draws its IDs from the OS. It makes nothing that would have to be removed.
3. **Questions that cannot be used were reported as a hang.**
   - Fix: `request_outcome` first decides whether the worker goes on. For a task that failed after all, unusable questions are `NoResult`, a limit that cannot be waited for is `NoStatus`, and only the watchdog's end is `Hung`.
   - The fake CLI gets the action `questions`.
   - Commits: $\color{red}{\textbf{RED}}$ [`ce0d1ef`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ce0d1ef4adaffbdbe9f7363c52503140fb377eb0) → $\color{green}{\textbf{GREEN}}$ [`10f136b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/10f136b1c53902f60ead5a34574af9c7c204f0d6).
4. **`agent` without a command did not list `run`.**
   - Fix: one function builds the command line, used by the parser and by the help.
   - The test found a second fault: `run`'s description was the doc comment of `RunArgs`.
   - Commits: $\color{red}{\textbf{RED}}$ [`991e3cc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/991e3ccf306c323a10885fd2ab44539c394cf7c7) → $\color{green}{\textbf{GREEN}}$ [`e8265c7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e8265c7d3b1ff9ad564af1c91f4a66791841c1e3).
5. **Fixed sleeps in tests.** Three tests of this PR now wait for something they can see [`99b8295`](https://github.com/mateusz-szczyrzyca/team-agents/commit/99b829520d19aca10d3bcfc150cc3f8fa14d445b):
   - two of them wait for the answer to a status request, which the owner gives only after its first look at every team;
   - the bus-down test waits until the stand-in bus has refused the notice.
   - With `remember_closed_teams` left out, the restart test still fails `3` times out of `3`.
6. **`is_transient` widened for a test:** private again, [`0ec8c53`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0ec8c538d8313516aab1972e3c4f5cda2667c46e).
7. **The drop of a job the bus refuses for good was untested.**
   - A test now shows the refused notice is dropped and the next one still wakes the coordinator: [`2f531e2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2f531e299f118a6674872173bedb54c59aca279a).
   - The stand-in bus gets a mode that refuses one call with exit status `3`.
   - Checked: the test fails when the job is kept instead.

The tests changed in this pass are tests of this PR, not on `main`. The fixtures `team-cli.sh` and `flaky-agentbus.sh` (also not on `main`) only gained new actions.

### Checks run in this pass

```bash
GATE_WAIT=1 make gate    # at 0c0ee3f: PASS, all 9 steps; Rust: 1,401 passed, 0 failed
cargo clippy --workspace --all-targets --locked -- -D warnings    # 0 findings
cargo test -p agent --bin agent -- could_not_be_started_is_given_out   # RED at b25392a: FAILED (60 s); GREEN: ok 8 times out of 8
cargo test -p agent --bin agent -- daemon::owner::teams    # ok 3 times out of 3 after the barriers
```

Every test run went through `tools/test-lock.sh`. Size by the PR-1 command: `3,618` changed lines against `main`, `171` of them from this pass (limit `5,000`).
