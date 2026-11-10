# Finish PR [#73](https://github.com/mateusz-szczyrzyca/team-agents/pull/73) (round accounting) — `inv-gemini-argon`

- Investigator: `inv-gemini-argon` (Gemini 4 Argon High)
- Task: `20261110-072635-inv-gemini-argon`
- Round: `harness/round-accounting`
- Branch: `agents/inv-gemini-argon/20261110-072635`
- Starting PR head: [`aaadc20`](https://github.com/mateusz-szczyrzyca/team-agents/commit/aaadc20e3171d4ca196badb2464cc8e8adabf01f) on [#73](https://github.com/mateusz-szczyrzyca/team-agents/pull/73) (base [`45f1e7a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/45f1e7a29ebd2797d3112e39e129bfe938236604))
- Merged `origin/main` head: [`e35df3b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e35df3b22fad1837ea5d77569343ccc32e3fe6b8)

## Summary of what was checked, found, and changed

1. **Item `1` — Merge `origin/main` into the PR branch:**
   - Fast-forwarded `agents/inv-gemini-argon/20261110-072635` from [`45f1e7a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/45f1e7a29ebd2797d3112e39e129bfe938236604) to [`aaadc20`](https://github.com/mateusz-szczyrzyca/team-agents/commit/aaadc20e3171d4ca196badb2464cc8e8adabf01f), then merged `origin/main` ([`e35df3b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e35df3b22fad1837ea5d77569343ccc32e3fe6b8)) in [`1db0f4b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1db0f4b7e85526f43fbb98d026211b3919a53d37).
   - [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/README.md) was the only file modified on both sides and auto-merged with `0` conflicts, keeping both the `Round accounting` section ([`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/README.md#L553-L584)) and the `R12A`–`R12G` native GitHub write-formatting and parity-ledger notes from `main` ([`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/README.md#L1702-L1733)).
2. **Item `2` — Close the ShellCheck lint gap:**
   - Added [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats), [`tests/unit/round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_runs.bats), and [`tests/unit/tokens_round.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/tokens_round.bats) to `SHELL_SCRIPTS` in [`Makefile`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/Makefile#L50).
   - Quoted literal `"done"` in `11` `wait_status` calls (`9` in [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L51-L116) and `2` in [`tests/unit/round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_runs.bats#L26-L39)) to resolve `SC1010`, and replaced `ls "$AGENTS_HOME/tasks" | grep -- '-inv[0-9]*$' || true` in [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L159-L163) with a glob loop over `"$AGENTS_HOME/tasks"/*-inv[0-9]*` to resolve `SC2010`, committed in [`977f540`](https://github.com/mateusz-szczyrzyca/team-agents/commit/977f5400c170b1a187335ed490687be435226f5b) with `0` suppressions.
3. **Item `3` — Audit `task_field` environment isolation:**
   - Audited all `589` `task_field` calls (`85` literal keys and `3` variable arguments) in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent).
   - Found `0` keys read with `task_field` that are never written into `meta.env`, and `0` callers that relied on falling back to the runner's or CLI's process environment when a field is absent from `meta.env`.
4. **Item `4` — Check the PR against the specification, test by test:**
   - Every requirement in Part `1` and Part `2` is covered by the `21` tests in [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats), [`tests/unit/tokens_round.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/tokens_round.bats), and [`tests/unit/round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_runs.bats); no specification gap was found.
5. **Item `5` — Verify against real fake-CLI run data:**
   - Executed an end-to-end fake-CLI team run with `Round: R12/live`, a coordinator (`lead`), a member (`member`), a delegated worker (`worker`), a review (`inv`), and a `3`-phase `--investigation` cohort (`investigation-team`, Phase `1` investigator, Phase `2` merger, Phase `3` reviewer).
   - Verified that `agent tokens <team>`, `agent tokens <investigation-team>`, and `agent tokens --round R12` list all `8` tasks with their exact roles, `ROUND_ID` values, role sums, model sums, sub-round sums, and total (`5,500` input, `2,200` cached, `550` output, `$0.5500` cost).
6. **Item `6` — Pass the full gate on the PR merged with `main`:**
   - `GATE_WAIT=1 make gate` passed all `8/8` steps with `0` lint findings on both [`1db0f4b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1db0f4b7e85526f43fbb98d026211b3919a53d37) and [`977f540`](https://github.com/mateusz-szczyrzyca/team-agents/commit/977f5400c170b1a187335ed490687be435226f5b).

## Item `3` — `task_field` audit

[`aaadc20`](https://github.com/mateusz-szczyrzyca/team-agents/commit/aaadc20e3171d4ca196badb2464cc8e8adabf01f) changed `task_field` in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L797-L799) to `unset "$2"` before sourcing `meta.env`, matching `investigation_field` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L5122)).

### Keys read with `task_field` that are never written into `meta.env`

- **Result:** `0` keys. Every key read by all `589` `task_field` calls in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent) (`85` literal keys and `3` variable arguments) has explicit `meta_set` or creation-time writers into `meta.env`.
- None of the `85` keys overlap with configuration variables in [`config/agents.conf`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/config/agents.conf) or top-level environment variables exported by `start_runner` (`AGENTS_HOME`, `PATH`, `XDG_RUNTIME_DIR`), `cmd__runner` (`LC_ALL`, `MCP_TOOL_TIMEOUT`, `AGENT_RUN_LONG_AT`), `git_identity` (`GIT_*`), `agents_gh_env` (`AGENT_REAL_GH`, `AGENT_GITHUB_*`, `AGENT_GH_*`, `AGENT_REVIEW_BYLINE`, `AGENTBUS_BIN`), or the CLI runners (`CLAUDE_CODE_DISABLE_BACKGROUND_TASKS`, `BASH_DEFAULT_TIMEOUT_MS`, `BASH_MAX_TIMEOUT_MS`, `ENABLE_TOOL_SEARCH`, `OPENCODE_*`).
- `handback` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L2676-L2844)) passes environment overrides using the `AGENT_` prefix (`AGENT_PRECHECK_PASS`, `AGENT_FIX_PASS`, `AGENT_FORMAT_BOUNCED`, `AGENT_HANDBACK_LEAD`), and `resume_run` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L7309-L7313)) reads those `AGENT_*` environment variables directly and writes `PRECHECK_PASS`, `FIX_PASS`, and `FORMAT_BOUNCED` into `meta.env`.

### Complete list of keys checked across all `589` `task_field` calls

1. **All `85` literal keys read with `task_field` (every one written to `meta.env` via `meta_set`):**
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
2. **All `3` variable-key `task_field` calls in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent):**
   - `$2` in `investigation_field` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L5122)): already ran `unset "$2"` before calling `task_field`.
   - `$field` in `cmd_debug_agent` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L8806)): bound only to `AGENT` or `MODEL`; did not rely on environment fallback.
   - `$c` in `cmd_debug_agent` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L8886-L8888)): iterates over `AGENT KIND MODEL TYPE STATUS NOTE CREATED FINISHED RUN HELPER_ID PARENT_TASK TEAM`; prints only persisted fields of `$t`.
3. **Optional `meta.env` keys that also appear as shell variables in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent) (including after `meta_load "$id"` in `cmd__runner`):**
   - `SOLO_CUT`: set in `watch_run` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L4309-L4334)) to create `$dir/solo-cut`, which `long_run_end` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L8642)) persists via `meta_set "$id" SOLO_CUT 1`; `fresh_session_due` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L8574)) reads that persisted value on the next run before `watch_run` runs. Did not rely on fallback.
   - `REMOTE_NODE`: set by `remote_identity` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L965)) for `<task>@<node>` arguments; `task_field` reads `REMOTE_NODE` only on `remote-member` proxy tasks ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L7333-L8436)) where `remote_member_task` persists `REMOTE_NODE`. Did not rely on fallback.
   - `INVESTIGATION_PHASE1_TASKS`, `KIND`, `INVESTIGATION_TEAM`, `PARENT_TASK`, `AGENT`, `BRANCH`: in `investigation_phase1_recover` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent#L5012-L5029)), callers previously had to prefix `KEY="" task_field ...` specifically to prevent `task_field` from leaking the dispatching runner's `meta_load` state into another task's lookup.

## Item `4` — Specification coverage table (Part `1` and Part `2`)

| Part | Specification requirement | Covering test file and test name |
|---|---|---|
| Part `1` | `round_id_of`: no `Round:` line gives `?`; one valid `Round: <id>` line gives `<id>` with blanks/CR trimmed; two `Round:` lines or a malformed ID (`R12//A`, `/R12`, `R12/`, `R12 A`, `R#12`, empty) is refused naming the line number; mid-line `Round:` is ignored | [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L16-L33) — `round_id_of: none gives ?, one line its ID with blanks trimmed, two lines or a broken ID are refused` |
| Part `1` | `agent run` refuses two `Round:` lines or `Round: R12//A` before creating any task directory or branch | [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L35-L44) — `agent run refuses a text with two Round: lines, or a broken ID, before anything starts` |
| Part `1` | `agent run` records `ROUND_ID` from task text in solo and `--team` runs, and `?` when absent | [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L46-L61) — `agent run records the round of its text in every task it makes; a text without one gives ?` |
| Part `1` | `agent run --investigation` records `ROUND_ID` in the cohort parent and in Phase `1`, Phase `2` (merger), and Phase `3` (reviewer) tasks | [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L63-L76) — `agent run --investigation records the round in the investigation team and in each of its phases` |
| Part `1` | `agent run --investigation` refuses a broken `Round:` line before creating any task directory | [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L78-L84) — `agent run --investigation refuses a broken Round: line before anything starts` |
| Part `1` | `agent run --resume` inherits the previous round when the addition has no `Round:` line, adopts a new `Round:` line when given without mutating the old task, and refuses a broken `Round:` line | [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L86-L105) — `agent run --resume keeps the previous round, takes the resume text's own Round: line, refuses a broken one` |
| Part `1` | System hand-back resumes (`AGENT_HANDBACK_LEAD`) keep the previous round and do not parse `Round:` lines in quoted review/fix-pass text | [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L107-L117) — `a hand-back's text (the system's, not the human's) never decides the round: the previous one is kept` |
| Part `1` | A delegated worker (`KIND=helper`) inherits its requester's `ROUND_ID` at creation and keeps it when the requester's `ROUND_ID` later changes | [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L131-L148) — `a worker gets its requester's round when it is made, and keeps it when the requester's changes` |
| Part `1` | A task-requested investigation or review (`inv-add`) inherits the requesting task's `ROUND_ID` at creation | [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L150-L170) — `an investigation and a review asked for by a task get that task's round` |
| Part `1` | Every task-metadata constructor that writes `ROUND=0` (`6` constructors) also writes `ROUND_ID` | [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats#L172-L179) — `every place that makes a task's metadata (ROUND=0) records its ROUND_ID with it` |
| Part `1` | `agent tokens <team>` computes the cycle-safe closure over `PARENT_TASK` and `INVESTIGATION_TEAM` at any depth, excludes other teams' tasks, and appends `ROLE` and `ROUND` to every row | [`tests/unit/tokens_round.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/tokens_round.bats#L53-L73) — `agent tokens <team>: every task reachable from the team's, at any depth, each with its role and round` |
| Part `1` | `agent tokens <task>` appends `ROLE` and `ROUND`, showing `-` for a task created before `ROUND_ID` existed | [`tests/unit/tokens_round.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/tokens_round.bats#L75-L86) — `agent tokens <task> ends its row with the role and the round; a task made before rounds shows -` |
| Part `1` | An old task without `ROUND_ID` does not inherit an ambient `ROUND_ID` from the caller's environment and is excluded from `agent tokens --round '?'` | [`tests/unit/tokens_round.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/tokens_round.bats#L88-L100) — `an old task without ROUND_ID does not inherit the caller's ambient value` |
| Part `1` | `agent tokens --round R12` selects `R12` and `<id>/...` (`R12/A`, `R12/A/#74`), leaves out `R120`, `R1`, and unlabelled tasks, and prints `role <name>`, `model <name>`, and `total` sums | [`tests/unit/tokens_round.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/tokens_round.bats#L121-L145) — `agent tokens --round R12: R12 and the rounds inside it, never R120 or R1; sums per role and model, the total` |
| Part `1` | `agent tokens --round` fails cleanly when no task matches the round or when the round argument is missing | [`tests/unit/tokens_round.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/tokens_round.bats#L147-L154) — `agent tokens --round without a task of that round says so` |
| Part `2` | Run start lines in `activity.jsonl` carry `round <id> pr #<n>` (or `pr -` before a PR), and `agent tokens --runs` still parses them | [`tests/unit/round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_runs.bats#L17-L33) — `a run's start line carries the round and the PR, or pr - before the task has one` |
| Part `2` | A task without a `Round:` line marks its runs with `round ? pr -` | [`tests/unit/round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_runs.bats#L35-L41) — `a task without a Round: line marks its runs with round ?` |
| Part `2` | A worker created while its requester was on PR `#74` records `REQUESTER_PR=74` and counts under `sub-round R12/A/#74` after the requester moved to `#75` | [`tests/unit/round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_runs.bats#L43-L71) — `a worker made while its requester was on PR #74 counts under #74 after the requester moved to #75` |
| Part `2` | `agent tokens --round` splits a member's usage across `sub-round` rows (`#74`, `#75`, `(no PR)`) while keeping one row per task with the task's total | [`tests/unit/round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_runs.bats#L94-L104) — `agent tokens --round: a member's usage split by its runs' PRs; its row keeps the task's total` |
| Part `2` | Runs without a marker (pre-marker runs or usage before the first run line) count under `sub-round <round>/?`, never dropped, with `total` unchanged | [`tests/unit/round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_runs.bats#L106-L112) — `agent tokens --round: runs without a marker count under <round>/?, never dropped; the total unchanged` |
| Part `2` | [`docs/node-improvements.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/docs/node-improvements.md) documents remote-member usage synchronisation and run markers, and [`docs/multi-instance.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/docs/multi-instance.md#L437-L441) links to it | [`tests/unit/round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_runs.bats#L114-L120) — `docs/node-improvements.md lists the nodes' future work, first the usage of remote members; multi-instance links it` |

## Item `5` — Verification on fake-CLI run data

Under `GATE_WAIT=1 tools/test-lock.sh`, an isolated temporary-home Bats run executed:

- `agent run --investigation --team inv $'Investigate a temporary fixture\nRound: R12/live'` (`20261110-074752-investigation-team` cohort parent, Phase `1` investigator `20261110-074752-inv`, Phase `2` merger `20261110-074754-inv` opening PR `#73`, and Phase `3` reviewer `20261110-074757-inv` on PR `#73`)
- `agent run --team lead,member $'Team work\nRound: R12/live'` (`20261110-074759-lead` and `20261110-074759-member`, with `PR_NUMBER=74` on `lead`, `20261110-074752-investigation-team` linked via `PARENT_TASK` to `member`, a delegated worker `20261110-074802-worker-h1` from `member`, and a review `20261110-074804-inv1` from `lead`)

### `agent tokens 20261110-074759-team`

```text
TASK                                     AGENT                    KIND                    IN       CACHED        OUT       COST ROLE          ROUND
20261110-074759-lead                     lead                     worker                1000          400        100     0.1000 coordinator   R12/live
20261110-074759-member                   member                   worker                2000          800        200     0.2000 member        R12/live
20261110-074804-inv1                     inv                      investigation          400          150         40     0.0400 review        R12/live
20261110-074752-investigation-team       investigation-team       investigation_team no usage reported                               investigation R12/live
20261110-074802-worker-h1                worker                   helper                 300          100         30     0.0300 worker        R12/live
20261110-074752-inv                      inv                      investigation          500          200         50     0.0500 investigation R12/live
20261110-074754-inv                      inv                      investigation          600          250         60     0.0600 investigation R12/live
20261110-074757-inv                      inv                      investigation          700          300         70     0.0700 review        R12/live
total                                                                                   5500         2200        550     0.5500
```

### `agent tokens --round R12`

```text
TASK                                     AGENT                    KIND                    IN       CACHED        OUT       COST ROLE          ROUND
20261110-074752-inv                      inv                      investigation          500          200         50     0.0500 investigation R12/live
20261110-074752-investigation-team       investigation-team       investigation_team no usage reported                               investigation R12/live
20261110-074754-inv                      inv                      investigation          600          250         60     0.0600 investigation R12/live
20261110-074757-inv                      inv                      investigation          700          300         70     0.0700 review        R12/live
20261110-074759-lead                     lead                     worker                1000          400        100     0.1000 coordinator   R12/live
20261110-074759-member                   member                   worker                2000          800        200     0.2000 member        R12/live
20261110-074802-worker-h1                worker                   helper                 300          100         30     0.0300 worker        R12/live
20261110-074804-inv1                     inv                      investigation          400          150         40     0.0400 review        R12/live
role coordinator                                                                        1000          400        100     0.1000
role investigation                                                                      1100          450        110     0.1100
role member                                                                             2000          800        200     0.2000
role review                                                                             1100          450        110     0.1100
role worker                                                                              300          100         30     0.0300
model claude                                                                            5500         2200        550     0.5500
sub-round R12/live/#73                                                                   700          300         70     0.0700
sub-round R12/live/#74                                                                  2700         1050        270     0.2700
sub-round R12/live/(no PR)                                                              2100          850        210     0.2100
total                                                                                   5500         2200        550     0.5500
```

## Checks

- `GATE_WAIT=1 make gate` at [`1db0f4b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1db0f4b7e85526f43fbb98d026211b3919a53d37) (merge of `origin/main`): `8/8` steps passed (`check-generated`, `lint-shell`, `lint-go`, `lint-docs`, `unit-go`, `unit-bats`, `harness`, `unit-agent-bats`), `0` lint findings.
- `GATE_WAIT=1 make gate` at [`977f540`](https://github.com/mateusz-szczyrzyca/team-agents/commit/977f5400c170b1a187335ed490687be435226f5b) (with the `3` new Bats files in `SHELL_SCRIPTS`): `8/8` steps passed, `0` lint findings.
- `shellcheck -S warning tests/unit/round_id.bats tests/unit/round_runs.bats tests/unit/tokens_round.bats`: `0` findings, `0` suppressions.
- `tools/check-agent-test-map.sh`: passed (`182` files, `760` total tests).

## Skipped tests

None.

## Not done

- Updating PR [#73](https://github.com/mateusz-szczyrzyca/team-agents/pull/73) on GitHub or pushing to `agents/inv-kimi/20261109-215048` is left to the Phase `2` merger as required by the Phase `1` strict-independence rules; the ready-to-use PR description for Item `7` is provided below.

## Ready-to-use PR description (Item `7`)

- **Title:** `Add round accounting across teams and runs`
- **Summary:**
  - Delivers both Part `1` and Part `2` of `harness/round-accounting` in one PR (accepted by the human in place of two separate PRs):
    - **Part `1`:** Parses and validates `Round: <id>` in task text (`round_id_of`) before creating tasks, worktrees, or remote jobs; records `ROUND_ID` across all `6` task constructors (`create_task`, `resume_run`, `start_helper`, `start_investigation`, `cmd__investigation_team`, `remote_member_task`) and their callers in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent); computes the cycle-safe `PARENT_TASK` / `INVESTIGATION_TEAM` closure in `team_task_closure` so investigation phases and nested workers/reviews are included in `agent tokens <team>`; appends `ROLE` and `ROUND` columns to `agent tokens`; adds `agent tokens --round <id>` with `role` and `model` sums; isolates `task_field` from ambient shell variables.
    - **Part `2`:** Appends `round <id> pr #<n>` (or `pr -`) to run start lines in `activity.jsonl`; snapshots `REQUESTER_PR` at worker/investigation/review creation so a child remains attributed to the PR active when it was requested; adds `sub-round` sums (`<round>/#<n>`, `<round>/(no PR)`, and `<round>/?` for unmarked runs) to `agent tokens --round <id>`; documents remote-node usage synchronisation in [`docs/node-improvements.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/docs/node-improvements.md), linked from [`docs/multi-instance.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/docs/multi-instance.md#L437-L441).
- **Merge with `origin/main` (Item `1`):**
  - Merged `origin/main` ([`e35df3b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e35df3b22fad1837ea5d77569343ccc32e3fe6b8)) in [`1db0f4b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1db0f4b7e85526f43fbb98d026211b3919a53d37) with `0` conflicts; [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/README.md) kept both the `Round accounting` documentation and `main`'s `R12A`–`R12G` native GitHub write-formatting and parity-ledger updates.
- **ShellCheck lint coverage (Item `2`):**
  - Added [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats), [`tests/unit/round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_runs.bats), and [`tests/unit/tokens_round.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/tokens_round.bats) to `SHELL_SCRIPTS` in [`Makefile`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/Makefile#L50), quoted `"done"` in `11` `wait_status` calls (`SC1010`), and replaced `ls | grep || true` with a glob loop (`SC2010`) in [`977f540`](https://github.com/mateusz-szczyrzyca/team-agents/commit/977f5400c170b1a187335ed490687be435226f5b).
- **`task_field` isolation check (Item `3`):**
  - Audited all `589` `task_field` calls (`85` literal keys and `3` variable arguments) in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/bin/agent): `0` keys are never written to `meta.env`, and `0` callers relied on falling back to ambient environment variables.
- **Specification and fake-CLI verification (Items `4`–`6`):**
  - All `21/21` tests in [`tests/unit/round_id.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_id.bats), [`tests/unit/tokens_round.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/tokens_round.bats), and [`tests/unit/round_runs.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/977f5400c170b1a187335ed490687be435226f5b/tests/unit/round_runs.bats) cover every Part `1` and Part `2` requirement; fake-CLI team + worker + review + `3`-phase investigation-team verification confirmed exact closure, roles, rounds, and sums; full gate (`make gate`) passed `8/8` steps with `0` lint findings and `0` suppressions.
