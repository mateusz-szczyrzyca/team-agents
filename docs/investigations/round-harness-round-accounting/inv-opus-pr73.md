# Finishing PR #73 (round accounting) — inv-opus (claude/opus)

Round: harness/round-accounting

Task `20261110-072635-inv-opus`, branch `agents/inv-opus/20261110-072635`. Work on
[#73](https://github.com/mateusz-szczyrzyca/team-agents/pull/73): its head
[`aaadc20`](https://github.com/mateusz-szczyrzyca/team-agents/commit/aaadc20e3171d4ca196badb2464cc8e8adabf01f) plus
three commits, merged with the current `main`
([`e35df3b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e35df3b22fad1837ea5d77569343ccc32e3fe6b8)). The feature was not started anew
and nothing that works was rewritten.

## Result

- The gate passes on the PR merged with the current `main`: `agent gate --wait` at
  [`78848c8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/78848c8372bfce6ae07d8ced21c4f859d9950c56):
  `8/8` steps, `0` lint findings.
- Items 1 to 6 are done (below). No production code needed a fix: the lint gap is closed, the `task_field`
  isolation broke no caller, every test of the specification has a test that covers it, and a real team run (fake
  CLIs) is reported correctly. The investigation team's phases, the gap this feature was made for, are counted.
- **Item 7 (the PR description) is not done by me.** This run is Phase 1 of an investigation: it may not open or
  edit a PR, and it pushes to its own branch, not to #73's. The commits fast-forward from #73's head, so the merger
  can push them to `agents/inv-kimi/20261109-215048` as they are. The section "Proposed PR description" below is
  ready to paste.
- I found an older bug on `main`, not caused by this PR and not fixed here: an interrupted review run can make the
  script treat the team's coordinator as failed (see "Found outside the PR"). It needs its own task.

## Commits

- [`5be1d18`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5be1d180d75b4c93abdad4c8e973a363d58fbd15) —
  merge of `origin/main` (item 1)
- [`b6a0107`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b6a0107595c39735088261497d2d2f75fe926b95) —
  the three test files in `SHELL_SCRIPTS`, their ShellCheck warnings fixed (item 2)
- [`78848c8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/78848c8372bfce6ae07d8ced21c4f859d9950c56) —
  the specification's tests the PR did not check, and the real team run (items 4 and 5)
- this report

Size against `main`: `1,418` lines (`+1,362` / `−56`) in `12` files, under the `2,000`-line soft limit.
`agent round-size 20261110-072635-inv-opus` says `2,087` lines, because it counts from the task's base
[`45f1e7a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/45f1e7a29ebd2797d3112e39e129bfe938236604) and
so includes `main`'s own `24` commits that the merge brought in. Against the current `main`, which is what #73 is
measured against, it is `1,418`.

## 1. Bringing the PR up to date

`git merge origin/main` at #73's head: **no conflict**. The two sides touch different code:

- `main` (since the PR's base) changed the Go rewrite (`agent/gh_body_diagnostics*.go`, `agent/gh_review_body*.go`),
  `docs/agent-go-map.md`, `rewrite-agent-in-go.md`, `.agents/reports/` and the README's Go-rewrite section.
- The PR changed `bin/agent`, its three test files, `tests/agent-go-map.tsv`, the docs, and the README's token and
  round sections.

`README.md` merged automatically: both sides' text is kept, in different sections. Neither side's behaviour changed.

## 2. The lint gap

- `Makefile`: `SHELL_SCRIPTS += tests/unit/round_id.bats tests/unit/round_runs.bats tests/unit/tokens_round.bats`,
  next to the other test files (and later `tests/unit/round_team_run.bats`, the new file of item 5).
- `wait_status … done` → `wait_status … "done"`: `11` places (SC1010), `9` in
  [`round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/78848c8372bfce6ae07d8ced21c4f859d9950c56/tests/unit/round_id.bats)
  and `2` in
  [`round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/78848c8372bfce6ae07d8ced21c4f859d9950c56/tests/unit/round_runs.bats).
- `ls … | grep -- '-inv[0-9]*$' || true` (SC2010) in `round_id.bats` → a glob loop over `tasks/*-inv*` that keeps the
  name matching `-inv[0-9]*$`. The test still finds the same review task the same way.
- `shellcheck -S warning` on the four files: `0` findings, no suppression. `make lint-shell` passes.

## 3. The change to `task_field`

`task_field task key` now runs `unset "$2"` before it sources `meta.env`
([`bin/agent#L799`](https://github.com/mateusz-szczyrzyca/team-agents/blob/78848c8372bfce6ae07d8ced21c4f859d9950c56/bin/agent#L799)).
Before, a key missing from the task's file came from the caller's shell: an exported variable, a variable the runner
loaded from its own `meta.env` (`meta_load`), or a `local` of any function up the call stack (Bash scope is dynamic).

**Verdict: no caller relied on the fallback. Nothing needed a fix.** How I checked:

### 3a. Static: the keys read and where they are written

Every key read with `task_field` (or `investigation_field`, which wraps it), as a literal or through a variable, is
written into `meta.env` by `meta_set`: either when a task is made (the `for kv in "KEY=…"` lists of `create_task`,
`resume_run`, `start_helper`, `start_investigation`, `cmd__investigation_team`, `investigation_*_admit`,
`investigation_phase_start`, and the `meta_set` lines of `remote_member_task`) or later by an explicit
`meta_set "$id" KEY …`.

**Keys read with `task_field` that are never written into a task's `meta.env`: none.**

The `85` literal keys read, all written somewhere:

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

Keys read through a variable:

- `cmd_debug_agent`: `$field` is `AGENT` or `MODEL`; `$c` is one of
  `AGENT KIND MODEL TYPE STATUS NOTE CREATED FINISHED RUN HELPER_ID PARENT_TASK TEAM`.
- `investigation_field` adds `INVESTIGATION_COMMENTED`, `INVESTIGATION_HANDOFF_*`, `INVESTIGATION_MERGER`,
  `INVESTIGATION_MERGER_TASK`, `INVESTIGATION_MERGER_REPLACEMENTS`, `INVESTIGATION_MERGE_RETRIES`,
  `INVESTIGATION_NOTIFIED`, `INVESTIGATION_PIPELINE`, `INVESTIGATION_PRECHECK*`, `INVESTIGATION_RESUME_*`,
  `INVESTIGATION_REVIEWER`, `INVESTIGATION_REVIEWER_TASK`, `INVESTIGATION_REVIEW_RETRIES`, `INVESTIGATION_SEED`,
  `INVESTIGATION_TARGET_BRANCH`, `INVESTIGATION_TRIED_MERGERS`, `INVESTIGATION_TRIED_REVIEWERS`. They are written as
  literals or through `meta_set "$parent" "INVESTIGATION_${key}"`, `"INVESTIGATION_${role}_TASK"`,
  `"INVESTIGATION_TRIED_${role}S"` and `"$key"` (`investigation_phase_start`, `investigation_phase_failed`).

What the environment of a run exports (the systemd `--setenv`s `AGENTS_HOME`, `PATH`, `XDG_RUNTIME_DIR`; the runner's
`LC_ALL`, `MCP_TOOL_TIMEOUT`, `AGENT_RUN_LONG_AT`, `GIT_*`, `AGENT_REAL_GH`, `AGENT_GH_*`, `AGENT_GITHUB_*`,
`AGENTBUS_BIN`, `AGENT_REVIEW_BYLINE`; `secrets.env`; and the CLI's `AGENT_HANDBACK_LEAD`, `AGENT_FIX_PASS`,
`AGENT_PRECHECK_PASS`) shares no name with these keys. So the list the task asks for (keys never written, set in a
run's environment) is empty.

### 3b. Dynamic: where old and new actually answer differently

A key is missing from one task's file even though other tasks have it (optional fields, a task still being written
field by field, a fixture). So I also measured where the change makes a difference:

- I put a temporary `task_field` into `bin/agent`, never committed. It logs every call where the key is missing from
  the task's `meta.env` but has a non-empty value in the calling shell, which is exactly where the old accessor
  returned something else.
- I ran the whole bats suite with it twice (`GATE_WAIT=1 make unit-bats`): `760` and `763` tests, all passing,
  `632` and `659` such calls.
- With an extra probe on `meta_load` I traced the one group I could not explain from the log.

Every call falls into one of these groups. In none did the code rely on the old value:

| Where | Key | Ambient value came from | Old answer | Verdict |
|---|---|---|---|---|
| `task_stop_requested`, `task_held` (the parent walk) | `PARENT_TASK` | the running child's own `PARENT_TASK` (its runner's `meta_load`) | an ancestor without `PARENT_TASK` "pointed" to the child's parent: a self-loop ended by the `visited` guard or the 3-step cap; when one investigation team's runner admitted another, it walked into the **other** team's parent | same result in the self-loop case; a wrong one in the cross-team case (a stop or pause of team A could hold team B). New is right |
| `agent_running_task`, `free_investigator_list`, `investigation_teams_*` (scans over all tasks) | `AGENT`, `STATUS`, `RUN`, `KIND`, `QUOTA_UNTIL` | the runner's own task | a task whose file lacked the field (half-written, a fixture) took the runner's own agent, status, run or quota | wrong before (a half-made task could look running or out of quota); new reads it as not running. The quota test still passes: the out-of-quota task has `QUOTA_UNTIL` in its own file |
| `member_out` from `team_after_run` | `TEAM` | the lead's metadata, loaded into a review's runner by `notify` | see "Found outside the PR" | not a reliance; the old answer made that bug worse |
| `gh_coordinator` with no `RUNNER_ID` | `TEAM` | the test's exported `TEAM=T1` (`github_name_registry.bats`) | the test's team | only outside a runner; the tests pass either way |
| `node_heartbeat_json` | `AGENT` | the exported `AGENT=1` of the test "M4: ambient runner environment variables do not leak into task metadata" | the leak that test is about | the new answer is what that test wants |
| `cmd_tokens`, `tokens_table` | `ROUND_ID` | the exported `ROUND_ID='?'` of the PR's own test | the leak the PR fixed | intended |

I also checked by hand the case that looked most likely to rely on the fallback: `do_fix_pass` sets
`PR_NUMBER=$(task_field "$parent" PR_NUMBER)` and can run inside a review's runner, whose own `PR_NUMBER` is in the
shell. Its parent always has `PR_NUMBER`: `request_review` writes it before it queues the review (`--verify`,
`--autopilot`), and the script is the only one that queues reviews (`inv-add --kind review`). Its normal path is a
separate unit (`agent _fixpass`), which never had the review's values anyway. So it did not rely on the fallback.

Hardening, not changed: `unset "$2"` without `-v` would remove a **function** of that name when no variable has it.
No function in `bin/agent` has an upper-case name (`0`), so nothing breaks. `unset -v` would make that certain.

## 4. The specification, test by test

The specification is the earlier task text of round accounting (the task given to the Phase 1 investigators of #73, summarised in its reports in this directory and in #73's description).
✅ = covered by the PR. ➕ = test added or extended in
[`78848c8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/78848c8372bfce6ae07d8ced21c4f859d9950c56). None of
the added tests found a gap: each passed on its first run (the commit says `Test-First: not possible` with that
reason).

### Part 1

| Specification test | Test | |
|---|---|---|
| `Round:` none → `?`; one → its ID, blanks trimmed | `round_id.bats`: "round_id_of: none gives ?, one line its ID with blanks trimmed, two lines or a broken ID are refused" | ✅ |
| two `Round:` lines → `agent run` refuses | same; and "agent run refuses a text with two Round: lines, or a broken ID, before anything starts" (solo) | ✅ |
| `R12//A` → refused | `round_id_of` test (`R12//A`, `/R12`, `R12/`, `R12 A`, `R#12`, empty); and `agent run --team lead,member $'Round: R12//A\nwork'` in "agent run refuses …", which also checks that no task and no branch were made | ✅ |
| `agent run` records the ID from the text (team and solo) | "agent run records the round of its text in every task it makes; a text without one gives ?" | ✅ |
| `agent run --investigation` records it | "agent run --investigation records the round in the investigation team and in each of its phases"; "… refuses a broken Round: line before anything starts" | ✅ |
| `--resume` inherits it, or takes the resume text's own line | "agent run --resume keeps the previous round, takes the resume text's own Round: line, refuses a broken one"; "a hand-back's text … never decides the round" | ✅ |
| a worker gets the requester's ID and keeps it when the requester's changes | "a worker gets its requester's round when it is made, and keeps it when the requester's changes" | ✅ |
| an investigation team and its phases get it and keep it | "agent run --investigation records the round …": the phases had it; **now also kept** after the investigation team's `ROUND_ID` changes | ➕ |
| a review gets it and keeps it | "an investigation and a review asked for by a task get that task's round": the review had it; **now also kept** after the requester's changes | ➕ |
| a single investigation (not a review) gets it and keeps it | **new**: "an investigation (not a review) asked for by a task gets that task's round, and keeps it when the task's changes". The old test's title named an investigation, but it queued only a review | ➕ |
| every `ROUND=0` site writes `ROUND_ID` | "every place that makes a task's metadata (ROUND=0) records its ROUND_ID with it" (source test, at least `6` sites) | ✅ |
| `agent tokens <team>`: a member's worker (one level) | `tokens_round.bats`: "agent tokens <team>: every task reachable from the team's, at any depth, each with its role and round" (`H-worker`) | ✅ |
| an investigation team's phases (two levels), role `investigation`; a review, role `review` | same (`I-team`, `I-phase`, `I-review`); and on tasks the script made: **new** `round_team_run.bats` (item 5) | ✅ ➕ |
| an investigation asked for by a worker (three levels) | same (`W-inv`) | ✅ |
| a cycle does not loop | same (`Cyc-a` ↔ `Cyc-b`, under `timeout 30`, one row each) | ✅ |
| another team's investigation is not included | same (`X-inv`, and `X-linked`, a task of another team whose `PARENT_TASK` points into this one) | ✅ |
| `--round R12`: includes R12, R12/A, R12/A/#74 | "agent tokens --round R12: R12 and the rounds inside it, never R120 or R1; sums per role and model, the total" | ✅ |
| `--round R12` leaves out R120 and R1 | same: no `r120` row, no `r1` row, no `70000…` usage of either | ✅ |
| sums per role and per model match the rows; the total matches the sum | same (fixed numbers per role, per model and total); and **new** `round_team_run.bats`: the role, model and sub-round sums each add up to the total | ✅ ➕ |
| `tests/unit/tokens.bats` passes unchanged | unchanged against `main` (`git diff origin/main...HEAD -- tests/unit/tokens.bats` is empty); `5/5` pass in the gate | ✅ |

### Part 2

| Specification test | Test | |
|---|---|---|
| a run's start line carries the round and the PR, or `pr -` | `round_runs.bats`: "a run's start line carries the round and the PR, or pr - before the task has one"; "a task without a Round: line marks its runs with round ?" | ✅ |
| a member's runs over two PRs: split between `R12/A/#74` and `R12/A/#75`; the task's row keeps its total | "agent tokens --round: a member's usage split by its runs' PRs; its row keeps the task's total" | ✅ |
| a worker made while its requester was on #74 counts under #74 after the requester moved to #75 | "a worker made while its requester was on PR #74 counts under #74 after the requester moved to #75" (real worker, `REQUESTER_PR=74`, marker `pr #74`, report after the move: `R12/A/#74`, no `#75`); **new**: "a worker's later run still goes into the PR its requester was on …", because the first test's only run starts before the move: `run_pr` of the worker is `74` with the requester on `#75`, and empty for a worker made before its requester had a PR | ✅ ➕ |
| runs without a marker counted under `<round>/?`, the total unchanged | "agent tokens --round: runs without a marker count under <round>/?, never dropped; the total unchanged" (`R12/A/?` = `367`, total `87367` = the sum of all rows) | ✅ |
| `docs/node-improvements.md` exists and `docs/multi-instance.md` links it | "docs/node-improvements.md lists the nodes' future work …": it only grepped the name; **now also** checks for a Markdown link `](node-improvements.md)` | ✅ ➕ |

## 5. Against real data

[`round_team_run.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/78848c8372bfce6ae07d8ced21c4f859d9950c56/tests/unit/round_team_run.bats)
makes everything through the script, on the fake CLIs. The only artificial data are the usage lines, because the
fake CLIs report none. The test appends them after each task's last run line, so they also count under that run's
marker. What it makes:

- an investigation team `agent run --investigation --team inv $'…\nRound: R20/inv'` with its three phases;
- then a team `agent run --team lead,member $'Build it.\nRound: R20/A'`;
- the team's PR `#74` on the coordinator;
- a worker the member asks for (`agentbus delegate`);
- a review of `#74` the coordinator asks for (`agentbus inv-add --kind review`).

The three reports, from the test's run:

```text
TASK                                     AGENT                    KIND                    IN       CACHED        OUT       COST ROLE          ROUND
20261110-075502-lead                     lead                     worker              100000        10000       1000     0.0000 coordinator   R20/A
20261110-075502-member                   member                   worker               20000         2000        200     0.0000 member        R20/A
20261110-075505-inv1                     inv                      investigation          400           40          4     0.0000 review        R20/A
20261110-075505-worker-h1                worker                   helper                3000          300         30     0.0000 worker        R20/A
total                                                                                 123400        12340       1234     0.0000
```

```text
TASK                                     AGENT                    KIND                    IN       CACHED        OUT       COST ROLE          ROUND
20261110-075454-investigation-team       investigation-team       investigation_team no usage reported                               investigation R20/inv
20261110-075455-inv                      inv                      investigation           50            5          0     0.0000 investigation R20/inv
20261110-075457-inv                      inv                      investigation           60            6          0     0.0000 investigation R20/inv
20261110-075500-inv                      inv                      investigation           70            7          0     0.0000 review        R20/inv
total                                                                                    180           18          0     0.0000
```

`agent tokens --round R20` lists all `8` tasks (each once), then:

```text
role coordinator                                                                      100000        10000       1000     0.0000
role investigation                                                                       110           11          0     0.0000
role member                                                                            20000         2000        200     0.0000
role review                                                                              470           47          4     0.0000
role worker                                                                             3000          300         30     0.0000
model claude                                                                          123580        12358       1234     0.0000
sub-round R20/A/#74                                                                     3400          340         34     0.0000
sub-round R20/A/(no PR)                                                               120000        12000       1200     0.0000
sub-round R20/inv/#73                                                                     70            7          0     0.0000
sub-round R20/inv/(no PR)                                                               110           11          0     0.0000
total                                                                                 123580        12358       1234     0.0000
```

All of it is right, and the test asserts it:

- the team's report has its `4` tasks with their roles and `R20/A`, and none of the investigation team's;
- the investigation team's report has its parent and all `3` phases (the reviewer as `review`);
- the round has both, every sum adds up to the total;
- the coordinator's and the member's runs started before the PR (`(no PR)`); the worker's and the review's went into
  `#74`; the investigation's reviewer went into the PR its merger opened (`#73`, from the fake `gh`).

**The gap is closed.** An investigation team's phases are its tasks through `INVESTIGATION_TEAM` / `PARENT_TASK` at
the second level. `agent tokens <investigation team>` now lists them; before the PR it stopped with "no task of team",
as the earlier report says. `agent tokens --round` counts them with the team they belong to.

Two things the real run shows that a reader may not expect (both documented, neither a defect):

- An investigation team started by `agent run --investigation` is never linked to a team: the human starts it, and no
  task is its parent. So `agent tokens <team>` does not include it; `agent tokens --round` does.
- `model claude`: the fake agents have no `MODEL`, so the model sum falls back to the type (`task_model`).

## 6. The gate

```bash
agent gate --wait   # at 78848c8, a committed clean tree, origin/main e35df3b merged in
```

```text
ok    check-generated     0s
ok    lint-shell         19s
ok    lint-go             4s
ok    lint-docs           0s
ok    unit-go            11s
ok    unit-bats         218s
ok    harness            20s
ok    unit-agent-bats     4s
gate: all 8 steps passed
gate: PASS at 78848c8 — recorded
```

- `unit-bats`: `763/763`, `0` skipped. The round-accounting files: `round_id.bats` `11`, `round_runs.bats` `7`,
  `tokens_round.bats` `5`, `round_team_run.bats` `1`.
- `harness`: `60/60`; `unit-agent-bats`: `9/9`; `unit-go`: `agentbus`, `agentlive`, `agent` pass.
- `tools/check-agent-test-map.sh`: OK, `763` tests, every new test has its row in `tests/agent-go-map.tsv` (round
  `R53`, as the PR's other rows).
- No failure, so nothing to compare with `main`. The `node_two_homes.bats` failure that #73's precheck once reported
  did not come back in any of my `3` full runs.

## Found outside the PR

**An interrupted review run can make the script take the team's coordinator for failed.** This is on `main`, and the
PR neither causes nor changes it. My probe saw it in `review_resilience.bats` ("--autopilot: a review that ends
without a verdict while the other is ready to merge is done again by another investigator"):

1. A review's runner exits interrupted. `runner_exit` → `review_without_verdict` → `review_pair_done` →
   `notify "$parent" failed …`
   ([`bin/agent#L3201`](https://github.com/mateusz-szczyrzyca/team-agents/blob/78848c8372bfce6ae07d8ced21c4f859d9950c56/bin/agent#L3201)).
2. `notify` runs `meta_load "$id"` in the caller's shell
   ([`bin/agent#L4667`](https://github.com/mateusz-szczyrzyca/team-agents/blob/78848c8372bfce6ae07d8ced21c4f859d9950c56/bin/agent#L4667)):
   the review's runner now holds the **coordinator's** `TEAM`, `AGENT`, `KIND`, ….
3. `runner_exit` then sees `TEAM` set and calls `team_after_run "$RUNNER_ID" failed`
   ([`bin/agent#L6171`](https://github.com/mateusz-szczyrzyca/team-agents/blob/78848c8372bfce6ae07d8ced21c4f859d9950c56/bin/agent#L6171)).
   With `AGENT` equal to the lead it goes into the coordinator-failed branch: `coordinator_takeover`, then
   `member_out`, then a push saying "The coordinator … failed".
   ([`bin/agent#L3554`](https://github.com/mateusz-szczyrzyca/team-agents/blob/78848c8372bfce6ae07d8ced21c4f859d9950c56/bin/agent#L3554)).

With the old `task_field`, `member_out` also took the lead's `TEAM` for the review and told the team that the review's
agent was out. With the new one it stops there, but `coordinator_takeover` still runs. A fix (`notify` in a subshell,
or `notify` reading fields with `task_field` instead of `meta_load`) changes notification behaviour, so it is not
part of this round. **Recommended:** a separate task, test first: an interrupted review of an `--autopilot` pair
leaves the team's coordinator in place.

## Proposed PR description (item 7)

The merger can use this as #73's description. It keeps the existing sections and adds what this pass found. Replace
the commit links once the commits are on #73's branch (the SHAs stay the same if they are pushed as they are).

````markdown
## Summary

Round accounting, both parts of the specification in one PR (the human accepted one PR instead of two):

- **Part 1 — the round ID, a team's tasks, the report by round.** `round_id_of` reads the task text's one `Round:`
  line (`?` without one); `agent run` (solo, `--team`, `--investigation`, `--resume`, `--need`/`--on`) refuses two such
  lines or a broken ID before anything starts. Every task records `ROUND_ID` when it is made — from its text, from the
  resume text or the previous team, or from the task it is made for (workers, investigations, reviews, investigation
  teams' phases, failover replacements, remote members' proxies). `agent tokens <team>` follows `PARENT_TASK` and
  `INVESTIGATION_TEAM` at any depth, once per task, never into another team; rows gain `ROLE` and `ROUND` at the end.
  `agent tokens --round <id>` reports `<id>` and every `<id>/…`, with sums per role and per model, then the total.
- **Part 2 — sub-rounds per run, the nodes' future work.** Every run's start line carries its round and PR
  (`▶ run 7 (resume) round R12/A pr #74`, `pr -` before a PR); a worker, an investigation or a review records its
  requester's PR when it is made (`REQUESTER_PR`). `agent tokens --round` adds sums per sub-round; a run without a
  marker counts under `<round>/?`. `docs/node-improvements.md` (linked from `docs/multi-instance.md`) lists the nodes'
  future work, first the usage that nodes do not synchronise.
- `task_field` no longer takes a field missing from a task's `meta.env` from the caller's shell.

## Merge with main

`origin/main` (`e35df3b`) merged with no conflict: main changed the Go rewrite (`agent/`), its ledger documents and the
README's Go-rewrite section; this PR changed `bin/agent`, its tests and other README sections. Both sides are kept.

## Checks

- `agent gate --wait` at `78848c8` (main merged in): `8/8` steps, `0` lint findings; `unit-bats` `763/763`, `harness`
  `60/60`, `unit-agent-bats` `9/9`.
- Round accounting: `24` tests in `round_id.bats`, `round_runs.bats`, `tokens_round.bats`, `round_team_run.bats`.
- `tests/unit/tokens.bats`: unchanged, `5/5`.
- The three test files of the PR, and the new `round_team_run.bats`, are now in `SHELL_SCRIPTS`: ShellCheck found
  `11` × SC1010 and `1` × SC2010, fixed without changing what the tests check.

## The specification, test by test

Every test of Part 1 and Part 2 has a test (table in `docs/investigations/round-harness-round-accounting/inv-opus-pr73.md`).
Added in this pass, none of which found a gap: the phases' and the review's round kept after the requester's changes; a
single investigation's round; a worker's later run in its requester's old PR; the Markdown link to
`docs/node-improvements.md`; a real team run with a worker, a review and an investigation team, reported by
`agent tokens <team>`, `<investigation team>` and `--round` with every role, round and sum right.

## task_field isolation

Checked every key read with `task_field`: none is missing from every task's `meta.env` while set in a run's
environment, and an instrumented run of the whole suite (`659` reads where old and new differ) showed no caller that
relied on the old fallback — where they differed, the old answer was another task's value. No call needed a fix.

## Compatibility

- Token-table columns and their order unchanged; `ROLE` and `ROUND` appended.
- `meta.env` gains optional `ROUND_ID`, `REQUESTER_PR`, `INVESTIGATION_RESUME_ROUND_ID`; old tasks read as before
  (`-`, `<round>/?`).
- Run start lines gain a suffix; old lines are still parsed.
- `agent run` refuses a text with two `Round:` lines or a broken ID (README).
- New log event `job_round_unreadable`.

## Lint suppressions

None.

## Skipped tests

None.

## Not done

- Panel round display, usage synchronisation from nodes, a round's life cycle, labelling old tasks: out of scope.
- Found on main, not changed here: an interrupted review run loads its coordinator's metadata into its own shell
  (`notify` → `meta_load`) and then runs the coordinator-failed path. A separate task.
````

## Lint suppressions

None added.

## Skipped tests

None.

## Not done

- #73's description is not updated and #73's branch has no new commits from me: Phase 1 rules (own branch, no PR).
  The merger pushes [`5be1d18`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5be1d180d75b4c93abdad4c8e973a363d58fbd15),
  [`b6a0107`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b6a0107595c39735088261497d2d2f75fe926b95),
  [`78848c8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/78848c8372bfce6ae07d8ced21c4f859d9950c56)
  (and this report) to `agents/inv-kimi/20261109-215048`, a fast-forward from
  [`aaadc20`](https://github.com/mateusz-szczyrzyca/team-agents/commit/aaadc20e3171d4ca196badb2464cc8e8adabf01f), and
  pastes the description above.
- The `notify` / `meta_load` bug above: reported, not fixed.

## Suggestions for AGENTS.md (not changed: rule 2)

- Under "Bash": a function must not `meta_load` (`source` a `meta.env`) another task's metadata into a shell that runs
  a task — read single fields with `task_field`, or load in a subshell. The `notify` bug above is that.
- Under "Tests" / bats: a new bats file goes into the `Makefile`'s `SHELL_SCRIPTS` and every new test into
  `tests/agent-go-map.tsv` in the same commit; the first is not checked by anything, which is how this PR's files
  escaped ShellCheck.

There are no agentbus tools in this run, so nothing went to the repository notebook.
