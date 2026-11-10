# Finishing PR #73 (round accounting) — inv-kimi (opencode), Phase 2 merge

Round: `harness/round-accounting`

Task `20261110-081508-inv-kimi`. I merged the two authoritative Phase 1 results onto
[#73](https://github.com/mateusz-szczyrzyca/team-agents/pull/73), starting at its head
[`aaadc20`](https://github.com/mateusz-szczyrzyca/team-agents/commit/aaadc20e3171d4ca196badb2464cc8e8adabf01f).
The feature was not restarted, no existing test or stub was changed, and no production-code change was needed in this
merge.

## What was taken and dropped

### `inv-opus`

Taken from authoritative result [`f9db27a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f9db27ad67d82f960a34c71661974bcfc59de4ea):

- [`5be1d18`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5be1d180d75b4c93abdad4c8e973a363d58fbd15) — merge of current `origin/main` into #73.
- [`b6a0107`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b6a0107595c39735088261497d2d2f75fe926b95) — ShellCheck registration and fixes.
- [`78848c8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/78848c8372bfce6ae07d8ced21c4f859d9950c56) — the additional specification checks and the fake-CLI team run.
- [`inv-opus-pr73.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f9db27ad67d82f960a34c71661974bcfc59de4ea/docs/investigations/round-harness-round-accounting/inv-opus-pr73.md) — the static and dynamic `task_field` audit, full coverage analysis, and run evidence.

Nothing from this result was dropped. Its additional tests are a superset of the other result's checks.

### `inv-gemini-argon`

Taken from authoritative result [`ab4d4d5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ab4d4d51d402c1b3124136b3919d54da69ac881a):

- [`inv-gemini-argon.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ab4d4d51d402c1b3124136b3919d54da69ac881a/docs/investigations/round-harness-round-accounting/inv-gemini-argon.md) — the independent `589`-call audit, specification table, fake-CLI verification, and gate evidence.

Dropped as redundant:

- [`1db0f4b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1db0f4b7e85526f43fbb98d026211b3919a53d37) — duplicate merge of the same `origin/main`; superseded by `5be1d18`.
- [`977f540`](https://github.com/mateusz-szczyrzyca/team-agents/commit/977f5400c170b1a187335ed490687be435226f5b) — duplicate ShellCheck registration and fixes; superseded by `b6a0107`, whose glob keeps the original `-inv[0-9]*$` name selection and whose branch also contains the additional coverage.

Both manifest entries were `done`; neither findings file was absent.

## 1. Merge with current `main`

`origin/main` was [`e35df3b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e35df3b22fad1837ea5d77569343ccc32e3fe6b8).
The retained merge `5be1d18` had no conflict. `main` brought the Go rewrite, its reports and ledger documents, and the
README's Go-rewrite section; #73 brought `bin/agent`, round-accounting tests and documentation, and other README
sections. The automatic README resolution kept both sides' text and behaviour. No conflict resolution discarded a
change.

## 2. ShellCheck gap

[`b6a0107`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b6a0107595c39735088261497d2d2f75fe926b95) added
`round_id.bats`, `round_runs.bats`, and `tokens_round.bats` to `SHELL_SCRIPTS`; `78848c8` added `round_team_run.bats`
to the same list. It quoted `"done"` in `11` calls and replaced `ls | grep || true` with a glob loop. My direct
`shellcheck -S warning` check of all four files returned `0` findings. No suppression was added.

## 3. `task_field` fallback audit

**Result: no caller relied on the old environment fallback, so no caller needed a fix and the isolation remains.**

I independently extracted the literal accesses from `bin/agent`: `85` literal keys, each with a `meta_set` or
creation-list writer; the set of literal keys never written into `meta.env` is empty. This agrees with Gemini's
independent census of `589` calls. Opus additionally instrumented the accessor and ran the complete Bats suite,
examining `659` reads where the old and new answers differed; those were cross-task or ambient leaks, not intentional
fallbacks.

Literal keys checked (all have metadata writers):

```text
AGENT AGENTS_MD AUTOPILOT AUTOPILOT_OFF AUTOPILOT_QBOUNCED AUTO_STAGE BASE BASE_SHA BRANCH CREATED DBOUNCE_PENDING
DONE_BOUNCED FAILOVER_UNTIL FINISHED FIX_PASS FORMAT_BOUNCED FROM_AGENT HANDBACK_PENDING HANDBACK_SINCE HELPER_ID
HOLD_START IDLE_RUNS INVESTIGATION_BASE_SHA INVESTIGATION_LEASE INVESTIGATION_PHASE INVESTIGATION_PHASE1_TASKS
INVESTIGATION_TEAM INVESTIGATION_TOPIC_SLUG INV_ID JOB_ASSIGNMENT JOB_ID JOB_LEAD KIND LEAD_KICK_RUN LONG_RUN MODEL
NETWORK_PAUSES NETWORK_RUN NOSTATUS_BOUNCED NOTE OVERLOAD_TIMES PARENT_TASK PAUSE_REASON PRECHECK_HEAD PRECHECK_PASS
PR_MERGE_OWN PR_NUMBER PR_URL QUESTIONS_BOUNCED QUOTA_UNTIL REMOTE_AGENT REMOTE_NODE REMOTE_TEAM REPO_NAME REPO_ROOT
REQUESTER_PR RESTARTS RESUMED_FROM REVIEW REVIEW_PAIR REVIEW_PAIR_SINCE REVIEW_SLOT_PENDING REVIEW_WAIT_TOLD ROUND
ROUND_ID ROUND_PR ROUND_PR_BRANCH ROUND_TESTS RUN RUN_BOOT RUN_STARTS SOLO_CUT START_NOTIFY STATUS STATUS_BOUNCED
TEAM TEAM_STOPPED TIER TYPE VERIFY WAKE_AT WAKE_DEFERRED WORKTREE WORK_ROUND WORK_ROUNDS
```

Variable-key calls checked:

- `investigation_field`: `$2`; its dynamically formed `INVESTIGATION_*` keys and writers are enumerated in Opus's report.
- `cmd_debug_agent`: `$field`, restricted to `AGENT` or `MODEL`.
- `cmd_debug_agent`: `$c`, restricted to `AGENT KIND MODEL TYPE STATUS NOTE CREATED FINISHED RUN HELPER_ID PARENT_TASK TEAM`.

The runner/CLI exports and configuration names do not overlap with a key that lacks a metadata writer. Optional fields
can be absent from an individual task, but the old value then came from another task or the caller, not from a
supported fallback contract. The dynamic cases (`PARENT_TASK`, `TEAM`, `AGENT`, `STATUS`, `RUN`, `KIND`,
`QUOTA_UNTIL`, `ROUND_ID`) are tabulated in `inv-opus-pr73.md`; none relied on the old answer.

## 4. Specification coverage, test by test

“Added” means added or strengthened by `78848c8`; all other rows were already covered by #73. Every row passed in my
focused run.

| Part | Specification check | Covering test |
|---|---|---|
| 1 | No `Round:` gives `?`; one valid line gives its ID; whitespace/CR is trimmed | `round_id.bats`: `round_id_of: none gives ?, one line its ID with blanks trimmed, two lines or a broken ID are refused` |
| 1 | Duplicate lines and malformed IDs, including `R12//A`, are refused with the line number | Same `round_id_of` test |
| 1 | `agent run` refuses duplicates and `R12//A` before creating tasks or branches | `agent run refuses a text with two Round: lines, or a broken ID, before anything starts` |
| 1 | Solo and team tasks record the text's round; no line records `?` | `agent run records the round of its text in every task it makes; a text without one gives ?` |
| 1 | Investigation parent and all three phases record and keep the round | `agent run --investigation records the round in the investigation team and in each of its phases` — **added** keep-after-change assertions |
| 1 | Investigation creation rejects a broken line before starting | `agent run --investigation refuses a broken Round: line before anything starts` |
| 1 | Resume inherits or overrides the round and rejects a broken override | `agent run --resume keeps the previous round, takes the resume text's own Round: line, refuses a broken one` |
| 1 | System hand-back text cannot change the round | `a hand-back's text (the system's, not the human's) never decides the round: the previous one is kept` |
| 1 | A worker snapshots its requester's round | `a worker gets its requester's round when it is made, and keeps it when the requester's changes` |
| 1 | A requested review snapshots its requester's round | `an investigation and a review asked for by a task get that task's round` — **added** keep-after-change assertion |
| 1 | A requested non-review investigation snapshots the round | `an investigation (not a review) asked for by a task gets that task's round, and keeps it when the task's changes` — **added** |
| 1 | Every `ROUND=0` metadata constructor also writes `ROUND_ID` | `every place that makes a task's metadata (ROUND=0) records its ROUND_ID with it` |
| 1 | Team closure includes nested workers, investigations and all investigation phases; roles and rounds are shown | `tokens_round.bats`: `agent tokens <team>: every task reachable from the team's, at any depth, each with its role and round` |
| 1 | Closure terminates on cycles and excludes another team, even when cross-linked | Same closure test, including `Cyc-a`, `Cyc-b`, `X-inv`, and `X-linked` |
| 1 | Single-task rows append role and round; pre-round metadata displays `-` | `agent tokens <task> ends its row with the role and the round; a task made before rounds shows -` |
| 1 | Old metadata cannot inherit ambient `ROUND_ID` or enter `--round '?'` | `an old task without ROUND_ID does not inherit the caller's ambient value` |
| 1 | `--round R12` includes `R12` and descendants but excludes `R120`, `R1`, and unlabelled tasks; role/model/total sums are exact | `agent tokens --round R12: R12 and the rounds inside it, never R120 or R1; sums per role and model, the total` |
| 1 | Missing round argument and a round with no tasks fail cleanly | `agent tokens --round without a task of that round says so` |
| 1 | Existing `tokens.bats` remains unchanged and passes | `tokens.bats`: `5/5` in my focused run |
| 2 | Run markers carry round and PR, including `pr -`; existing run parsing still works | `round_runs.bats`: `a run's start line carries the round and the PR, or pr - before the task has one` |
| 2 | A task without a `Round:` line uses `round ?` | `a task without a Round: line marks its runs with round ?` |
| 2 | A worker requested on PR #74 stays under #74 after its requester moves to #75 | `a worker made while its requester was on PR #74 counts under #74 after the requester moved to #75` |
| 2 | The worker's later runs also retain #74; a pre-PR worker does not acquire a later requester PR | `a worker's later run still goes into the PR its requester was on when the worker was made, not the requester's new one` — **added** |
| 2 | Usage is split across #74, #75, and `(no PR)` while the task remains one row with its total | `agent tokens --round: a member's usage split by its runs' PRs; its row keeps the task's total` |
| 2 | Unmarked runs and usage before a run marker count under `<round>/?` without changing the total | `agent tokens --round: runs without a marker count under <round>/?, never dropped; the total unchanged` |
| 2 | Node follow-up exists and `docs/multi-instance.md` contains a real Markdown link to it | `docs/node-improvements.md lists the nodes' future work, first the usage of remote members; multi-instance links it` — **added** link assertion |
| 1+2 | Real script-created team, worker, review and investigation phases have exact rows and sums | `round_team_run.bats`: `a team run and an investigation team of round R20: every task with its role, its round and the right sums` — **added** |

No added test exposed a production gap. `78848c8` therefore records `Test-First: not possible` because each new test
passed against behaviour already present; the original RED/GREEN sequences remain intact.

## 5. Fake-CLI run data

I executed `round_team_run.bats`, not only static fixtures. It created a team with a coordinator, member, delegated
worker and review, plus an investigation team with its parent and three phases, all on fake CLIs. Usage lines are the
test's only artificial run data because the fake CLIs report none.

Observed in my run:

- `agent tokens <team>` listed exactly the coordinator, member, worker and review, all as `R20/A`, totalling `123400` input, `12340` cached and `1234` output tokens.
- `agent tokens <investigation-team>` listed its parent and every phase, with the final phase as `review`, totalling `180` input, `18` cached and `0` output tokens.
- `agent tokens --round R20` listed all `8` tasks once and totalled `123580` input, `12358` cached and `1234` output tokens.
- Role, model and sub-round sums each added up to that total.
- Sub-rounds were `R20/A/#74` = `3400/340/34`, `R20/A/(no PR)` = `120000/12000/1200`, `R20/inv/#73` = `70/7/0`, and `R20/inv/(no PR)` = `110/11/0` (input/cached/output).

The original gap is closed: investigation phases are reachable through `INVESTIGATION_TEAM` / `PARENT_TASK`, appear in
both team and round reports, and contribute to every sum.

## Checks

- Focused round-accounting and compatibility run: `29/29` Bats tests passed (`24` round-accounting tests plus `5` unchanged `tokens.bats` tests).
- Direct ShellCheck on all four round-accounting Bats files: `0` findings, `0` suppressions.
- `tools/check-agent-test-map.sh`: passed, `183` files and `763` tests.
- `GATE_WAIT=1 make gate` on the merged tree: all `8/8` steps passed, including `763/763` Bats tests, with `0` lint findings and `0` skipped tests.

## Found outside this PR

Opus reported an older `main` bug in which an interrupted review can load its coordinator's metadata through
`notify` → `meta_load` and then enter the coordinator-failed path. It is not caused by round accounting and was not
changed here; it needs a separate test-first task.

## Lint suppressions

None.

## Skipped tests

None.

## Not done

- Panel round display, node usage synchronisation, round lifecycle management and retroactive task labelling remain out of scope.
- The separate `notify` / `meta_load` bug above remains unfixed.
