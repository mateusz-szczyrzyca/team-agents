# Round-accounting integration — `inv-astra`

Task `20261110-083221-inv-astra`, branch `agents/inv-astra/20261110-083221`.
Round: harness/round-accounting

## Integration and publication

Both authoritative Phase 1 results were fetched from published `origin` branches. No peer worktree was read and no investigator was contacted.

At the start of this merge, [#73](https://github.com/mateusz-szczyrzyca/team-agents/pull/73) had already advanced from [`aaadc20`](https://github.com/mateusz-szczyrzyca/team-agents/commit/aaadc20e3171d4ca196badb2464cc8e8adabf01f) to [`05789f0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/05789f0ae37c437a0b499fcb737c207277951fb2). Its head already integrated the two manifest results. I fast-forwarded the assigned branch to that published head, preserving all original RED/GREEN progressions and the earlier merger's report. No implementation was restarted or rewritten.

The phase's explicit publication rule permits pushing only `agents/inv-astra/20261110-083221`. The topic permits one superseding PR when the workflow cannot push to the original branch. This task therefore publishes one **Supersedes #73** PR from the assigned branch. It neither pushes to the original branch nor merges or closes either PR.

### Per-investigator decisions

- `inv-opus`: took [`5be1d18`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5be1d180d75b4c93abdad4c8e973a363d58fbd15) (merge with current `main`).
- `inv-opus`: took [`b6a0107`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b6a0107595c39735088261497d2d2f75fe926b95) (ShellCheck registration and warnings).
- `inv-opus`: took [`78848c8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/78848c8372bfce6ae07d8ced21c4f859d9950c56) (additional specification assertions and fake-CLI run).
- `inv-opus`: took the authoritative [`f9db27a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f9db27ad67d82f960a34c71661974bcfc59de4ea) and its [`docs/investigations/round-harness-round-accounting/inv-opus-pr73.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/docs/investigations/round-harness-round-accounting/inv-opus-pr73.md), including the dynamic metadata audit. Nothing was dropped.
- `inv-gemini-argon`: retained the independent audit, coverage table and fake-CLI evidence from authoritative [`ab4d4d5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ab4d4d51d402c1b3124136b3919d54da69ac881a) in [`docs/investigations/round-harness-round-accounting/inv-gemini-argon.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/docs/investigations/round-harness-round-accounting/inv-gemini-argon.md). Its file is byte-for-byte identical to the published result.
- `inv-gemini-argon`: dropped the duplicate merge [`1db0f4b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1db0f4b7e85526f43fbb98d026211b3919a53d37), superseded by Opus's merge.
- `inv-gemini-argon`: dropped duplicate lint changes [`977f540`](https://github.com/mateusz-szczyrzyca/team-agents/commit/977f5400c170b1a187335ed490687be435226f5b), superseded by Opus's lint fix plus broader test coverage. No unique production implementation was dropped.
- Absent or failed results: **none**. Both manifest entries are `done`, both recorded commits exist on `origin`, and both findings files are available. Opus's finishing report uses the `inv-opus-pr73` filename; its original `inv-opus` report is also present.

Gemini's original statement that all requirements were already covered is narrowed by Opus's findings: explicit non-review investigation coverage, retention after requester changes, later worker PR attribution, and the actual documentation link were missing or weaker. The integrated tests retain Opus's stronger assertions. The earlier two-PR proposals in historical reports are superseded by the human's acceptance of both parts in one PR.

## Merge and lint checks

Current `origin/main` is [`e35df3b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e35df3b22fad1837ea5d77569343ccc32e3fe6b8); it is already an ancestor of the integrated head through Opus's merge. There were `0` conflicts. [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/README.md) retains the round-accounting documentation and `main`'s native GitHub/parity notes. No conflict resolution removes either side's behaviour.

[`Makefile`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/Makefile) includes all four round-accounting Bats files in `SHELL_SCRIPTS`. Opus fixed `11` SC1010 warnings with quoted `"done"` and the SC2010 warning with a glob loop. Existing test intent is preserved. No suppression was introduced.

## Metadata fallback audit

I independently enumerated the literal `task_field` keys in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/bin/agent) and checked for explicit `meta_set` or constructor assignment writers, then reviewed the variable-key forms and runner/CLI exports. All `85` literal keys have writers. The requested intersection — keys never persisted but supplied by the run environment — is **empty**. No supported caller relied on the fallback, so no call-site fix or new behaviour test is needed.

Checked literal keys:

- `AGENT`
- `AGENTS_MD`
- `AUTOPILOT`
- `AUTOPILOT_OFF`
- `AUTOPILOT_QBOUNCED`
- `AUTO_STAGE`
- `BASE`
- `BASE_SHA`
- `BRANCH`
- `CREATED`
- `DBOUNCE_PENDING`
- `DONE_BOUNCED`
- `FAILOVER_UNTIL`
- `FINISHED`
- `FIX_PASS`
- `FORMAT_BOUNCED`
- `FROM_AGENT`
- `HANDBACK_PENDING`
- `HANDBACK_SINCE`
- `HELPER_ID`
- `HOLD_START`
- `IDLE_RUNS`
- `INVESTIGATION_BASE_SHA`
- `INVESTIGATION_LEASE`
- `INVESTIGATION_PHASE`
- `INVESTIGATION_PHASE1_TASKS`
- `INVESTIGATION_TEAM`
- `INVESTIGATION_TOPIC_SLUG`
- `INV_ID`
- `JOB_ASSIGNMENT`
- `JOB_ID`
- `JOB_LEAD`
- `KIND`
- `LEAD_KICK_RUN`
- `LONG_RUN`
- `MODEL`
- `NETWORK_PAUSES`
- `NETWORK_RUN`
- `NOSTATUS_BOUNCED`
- `NOTE`
- `OVERLOAD_TIMES`
- `PARENT_TASK`
- `PAUSE_REASON`
- `PRECHECK_HEAD`
- `PRECHECK_PASS`
- `PR_MERGE_OWN`
- `PR_NUMBER`
- `PR_URL`
- `QUESTIONS_BOUNCED`
- `QUOTA_UNTIL`
- `REMOTE_AGENT`
- `REMOTE_NODE`
- `REMOTE_TEAM`
- `REPO_NAME`
- `REPO_ROOT`
- `REQUESTER_PR`
- `RESTARTS`
- `RESUMED_FROM`
- `REVIEW`
- `REVIEW_PAIR`
- `REVIEW_PAIR_SINCE`
- `REVIEW_SLOT_PENDING`
- `REVIEW_WAIT_TOLD`
- `ROUND`
- `ROUND_ID`
- `ROUND_PR`
- `ROUND_PR_BRANCH`
- `ROUND_TESTS`
- `RUN`
- `RUN_BOOT`
- `RUN_STARTS`
- `SOLO_CUT`
- `START_NOTIFY`
- `STATUS`
- `STATUS_BOUNCED`
- `TEAM`
- `TEAM_STOPPED`
- `TIER`
- `TYPE`
- `VERIFY`
- `WAKE_AT`
- `WAKE_DEFERRED`
- `WORKTREE`
- `WORK_ROUND`
- `WORK_ROUNDS`

Variable-key calls:

- `cmd_debug_agent` / `$field`: `AGENT` or `MODEL`, both persisted.
- `cmd_debug_agent` / `$c`: `AGENT KIND MODEL TYPE STATUS NOTE CREATED FINISHED RUN HELPER_ID PARENT_TASK TEAM`, all persisted.
- `investigation_field` / `$2`: dynamically selected `INVESTIGATION_*` and ordinary metadata fields. Explicit writers and `INVESTIGATION_${key}`, `INVESTIGATION_${role}_TASK`, `INVESTIGATION_TRIED_${role}S` writers cover these; this wrapper already isolated the field before the PR.

Environment candidates checked: systemd's `AGENTS_HOME`, `PATH`, `XDG_RUNTIME_DIR`; runner locale/timeouts, `AGENT_RUN_LONG_AT`, Git identity/configuration, `AGENTBUS_BIN`, `AGENT_REAL_GH`, GitHub wrapper exports and `AGENT_REVIEW_BYLINE`; CLI hand-back/fix/precheck variables. None is a never-persisted lookup key. Arbitrary user secrets are not an intended metadata interface.

The inherited reports cover optional fields that can be absent from an individual task: `SOLO_CUT` is persisted before the next run reads it; `REMOTE_NODE` reads target persisted proxy tasks; `do_fix_pass` reads a parent PR recorded by `request_review`. Opus's instrumented suite observed `659` old/new differences in `PARENT_TASK`, `TEAM`, `AGENT`, `STATUS`, `RUN`, `KIND`, `QUOTA_UNTIL` and `ROUND_ID`. These are cross-task or ambient leaks, not supported fallbacks. I reviewed that evidence; I did not repeat or commit the temporary instrumentation.

## Specification coverage

I read the assertions, the published specification summaries and the finishing task's required cases. The table names all `24` round-accounting tests; a row may cover several specification clauses. The full gate executes these alongside the unchanged token compatibility tests.

| Part | Specification coverage | Test |
|---|---|---|
| 1 | Missing/trimmed declaration; duplicate/malformed IDs including `R12//A`; line diagnostics | [`round_id_of: none gives ?, one line its ID with blanks trimmed, two lines or a broken ID are refused`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_id.bats#L16) |
| 1 | Invalid solo/team input creates no task or branch | [`agent run refuses a text with two Round: lines, or a broken ID, before anything starts`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_id.bats#L35) |
| 1 | Solo/team creation records the round or `?` | [`agent run records the round of its text in every task it makes; a text without one gives ?`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_id.bats#L46) |
| 1 | Investigation parent and phases inherit and retain the round | [`agent run --investigation records the round in the investigation team and in each of its phases`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_id.bats#L63) |
| 1 | Invalid investigation input is refused before creation | [`agent run --investigation refuses a broken Round: line before anything starts`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_id.bats#L82) |
| 1 | Resume inherits or overrides and rejects invalid input | [`agent run --resume keeps the previous round, takes the resume text's own Round: line, refuses a broken one`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_id.bats#L90) |
| 1 | System hand-back inherits; its quoted text does not override | [`a hand-back's text (the system's, not the human's) never decides the round: the previous one is kept`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_id.bats#L111) |
| 1 | Worker inherits and retains requester round | [`a worker gets its requester's round when it is made, and keeps it when the requester's changes`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_id.bats#L135) |
| 1 | Review inherits and retains requester round | [`an investigation and a review asked for by a task get that task's round`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_id.bats#L154) |
| 1 | Non-review investigation inherits and retains requester round | [`an investigation (not a review) asked for by a task gets that task's round, and keeps it when the task's changes`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_id.bats#L182) |
| 1 | Every `ROUND=0` metadata constructor writes `ROUND_ID` | [`every place that makes a task's metadata (ROUND=0) records its ROUND_ID with it`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_id.bats#L209) |
| 2 | Run markers carry round/PR or `pr -`; old run parser works | [`a run's start line carries the round and the PR, or pr - before the task has one`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_runs.bats#L17) |
| 2 | Undeclared round marks runs with `?` | [`a task without a Round: line marks its runs with round ?`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_runs.bats#L35) |
| 2 | Worker requested on fake PR `#74` remains there after requester moves to `#75` | [`a worker made while its requester was on PR #74 counts under #74 after the requester moved to #75`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_runs.bats#L43) |
| 2 | Member usage split across PRs preserves its task total | [`agent tokens --round: a member's usage split by its runs' PRs; its row keeps the task's total`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_runs.bats#L94) |
| 2 | Unmarked runs count under `<round>/?`; total unchanged | [`agent tokens --round: runs without a marker count under <round>/?, never dropped; the total unchanged`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_runs.bats#L106) |
| 2 | Node follow-up exists and has a real Markdown link | [`docs/node-improvements.md lists the nodes' future work, first the usage of remote members; multi-instance links it`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_runs.bats#L114) |
| 2 | Later worker runs preserve requester snapshot, including no-PR snapshot | [`a worker's later run still goes into the PR its requester was on when the worker was made, not the requester's new one`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_runs.bats#L123) |
| 1 | Nested workers/investigations/phases/reviews; cycles terminate; foreign teams excluded; exact sum | [`agent tokens <team>: every task reachable from the team's, at any depth, each with its role and round`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/tokens_round.bats#L53) |
| 1 | Single task columns; legacy metadata uses `-` | [`agent tokens <task> ends its row with the role and the round; a task made before rounds shows -`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/tokens_round.bats#L75) |
| 1 | Ambient `ROUND_ID` never labels legacy metadata | [`an old task without ROUND_ID does not inherit the caller's ambient value`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/tokens_round.bats#L88) |
| 1 | Hierarchical selection includes descendants, excludes `R120`/`R1`; exact role/model/total sums | [`agent tokens --round R12: R12 and the rounds inside it, never R120 or R1; sums per role and model, the total`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/tokens_round.bats#L121) |
| 1 | Missing argument and unmatched round fail cleanly | [`agent tokens --round without a task of that round says so`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/tokens_round.bats#L147) |
| 1+2 | Script-created teams, worker, review and all investigation phases; exact rows and reconciled sums | [`a team run and an investigation team of round R20: every task with its role, its round and the right sums`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_team_run.bats#L28) |

The `5` existing tests in [`tests/unit/tokens.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/tokens.bats) remain unchanged against `main`. No test already on `main`, helper, stub, workflow or instruction file was edited by this integration. The newly added run-marker test is explicitly editable under this task. The test ledger only gains entries for new tests.

No further coverage gap required a production change. Opus's additional assertions passed against existing behaviour; its commit records `Test-First: not possible` for that reason. Original feature and metadata isolation RED/GREEN histories remain intact.

## Fake-CLI data

The full gate runs [`tests/unit/round_team_run.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/tests/unit/round_team_run.bats) in an isolated temporary home with fake CLIs. The script creates a normal team, delegated worker, requested review and investigation team with all three phases. Only usage lines are injected, because the fake CLIs report no usage.

- Normal team: exactly `4` tasks; `123400 / 12340 / 1234` input/cached/output.
- Investigation team: parent and all phases; `180 / 18 / 0` input/cached/output, with its final phase classified as `review`.
- `agent tokens --round R20`: all `8` tasks once; `123580 / 12358 / 1234` input/cached/output.
- Role, model and sub-round sums each reconcile to the round total.
- Normal-team sub-rounds: `R20/A/#74` is `3400 / 340 / 34`; `R20/A/(no PR)` is `120000 / 12000 / 1200`.

The investigation phase omission is closed. Gemini independently verified a linked normal/investigation team with different usage amounts; the synthetic nested-closure test covers that relation on every gate run.

## Run-marker test race found by the merger

The first full `agent gate --wait` at [`05789f0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/05789f0ae37c437a0b499fcb737c207277951fb2) failed the new test `a run's start line carries the round and the PR, or pr - before the task has one`: after `_wake`, the task remained `paused` instead of becoming `done`. The other `7` gate steps passed, including all linters.

The test appended `PR_NUMBER` and used `sed -i` to expire `WAKE_AT` immediately after observing `STATUS=paused`. Code inspection shows the runner still writes `FINISHED` after publishing that status. Its locked read/replace update can overwrite the test's unlocked edits, retaining the original future wake time. This is consistent with the observed pause; the original temporary task was cleaned up by the test harness.

I changed only this new test's two metadata writes to call the existing `meta_set` under its lock, in a sourced subshell. All assertions, the quota/resume scenario and the explicit `_wake` invocation remain unchanged. No timeout was increased and no production behaviour changed. The task explicitly allows editing this PR's newly added tests. The commit uses the documented test-race `Test-First: not possible` exception; the initial failing gate is retained as evidence.

## Checks

- Initial clean-tree `agent gate --wait`: the new run-marker race described above failed; the other `7` steps passed.
- After the fix, `GATE_WAIT=1 make unit-bats BATS_FILES='tests/unit/round_runs.bats'`: `7/7` passed.
- After the fix, `GATE_WAIT=1 make gate`: all `8/8` steps passed, including all `763/763` unit Bats tests, Go tests, harness and native CLI checks; `0` lint findings.
- `tools/check-agent-test-map.sh`: passed; `183` files, `763` tests.
- `git diff --check`: passed.
- Current `origin/main` remains the merged commit cited above. The final committed-tree `agent gate --wait` result is recorded in the PR's Checks section before publication.
- The combined diff remains below the `3,000`-line hard limit; the PR records its final size. Retained investigation reports account for most of the excess above the `2,000`-line soft target.

## Compatibility

Both specification parts remain in one PR. Existing token columns keep their order with `ROLE`/`ROUND` appended. Optional metadata and suffixed run markers leave old data readable; old unmarked usage remains in `<round>/?`. No migration or dependency is needed. Duplicate/malformed task-text declarations are newly rejected as documented in [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/README.md).

## Lint suppressions

None added.

## Skipped tests

None added.

## Not done

- Serena was unavailable in this session; code discovery used the permitted shell fallback.
- No real agent CLIs, integration tests, installation commands, peer worktrees or agentbus coordination tools were used.
- Remote usage synchronization, panel changes, round lifecycle management and retroactive labelling remain outside scope; follow-up is in [`docs/node-improvements.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/05789f0ae37c437a0b499fcb737c207277951fb2/docs/node-improvements.md).
- Opus reported an unrelated pre-existing interrupted-review `notify` / `meta_load` bug. It remains a separate test-first task; this merger did not reproduce or alter it.
