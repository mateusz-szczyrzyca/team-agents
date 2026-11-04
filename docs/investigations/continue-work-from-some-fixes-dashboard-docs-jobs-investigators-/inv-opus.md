# Continuing PR #60 — `inv-opus` (task `20261104-200148-inv-opus`)

Topic: continue the work of `some-fixes-dashboard-docs-jobs-investigators.md` (8 items) in
[#60](https://github.com/mateusz-szczyrzyca/team-agents/pull/60).

Branch `agents/inv-opus/20261104-200148` is PR #60's head
[`c02a062`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c02a062a8fd60f12fb2236776705fffd3f222376)
(fast-forward from `main` [`453d462`](https://github.com/mateusz-szczyrzyca/team-agents/commit/453d462129d2fb4a8e10148a7cf6749629e034c4))
plus `4` commits. Merging this branch therefore brings in PR #60 and these fixes together.

## Why PR #60 stopped

The `--investigation` (`20261104-160238-investigation-team`) ended `failed: review failed`, and nothing was merged. Its only
possible reviewer, `inv-astra` (task `20261104-180510-inv-astra`, codex `gpt-6-astra`), ended after `75` s with
`Selected model is at capacity. Please try a different model.` The code was never reviewed.
(`INVESTIGATION_REVIEW_RETRIES=1`; the merger `inv-gemini-argon` could not review its own result, so no other reviewer
was available.) The rest of this report is the review that did not happen, and its fixes.

## Status of the 8 items

| # | Item | State after this branch |
|---|---|---|
| `1` | dashboard: `20` latest entries, "Full dashboard view" page, tests | done in PR #60; no defect found |
| `2` | `/teams`: "Stop this team", no automatic resume, tests | done in PR #60; fixed: an undefined function in the stop fence, and finished members being rewritten to `stopped` |
| `3` | `--investigation` has priority over resuming normal teams | done in PR #60, but it also dropped work of teams already running; narrowed, with deferred wakes (below) |
| `4` | report of the background jobs | PR #60's report named units that do not exist; replaced by a verified inventory |
| `5` | a member stopped on demand shown as `sleeping` | done in PR #60; root cause confirmed (below) |
| `6` | Serena for every model | config and prompts done in PR #60; **the live installation still has it off** (below) |
| `7` | verify §7/§8 of the worker-prompt audit | done in PR #60; its last open Polish heading (item `11`) fixed here |
| `8` | rule: never open a new PR unless coordinator and none Draft/Open | done in PR #60; no defect found |

## Findings and fixes

### 1. `team_is_stopped` is called but never defined

`automation_allowed` ran `team_is_stopped "$team"` for every team task. The function does not exist. Bash printed
`team_is_stopped: command not found` on every automation check (wakes, prechecks, reviews, merges) and treated the
team as not stopped. The check added nothing: `team_stop` sets `TEAM_STOPPED=1` on every member, and
`task_stop_requested` follows `PARENT_TASK`. The call is removed.

### 2. The investigation priority dropped work of normal teams already running

PR #60 put `normal_resume_allowed` inside `automation_allowed` and `start_runner` (`mode != new`). While an
`--investigation` ran in the same repository, a normal team **that was already working** lost:
- its precheck, review requests and review results: `return 0`, nothing recorded, nothing ran later;
- its bounces (questions without recommendations, an unknown `STATUS`, a round, `--verify`), the end of its pauses,
  and its resumes after a restart: `resume_failed` was logged and the task was left stuck;
- every held-back wake. The bus wakes a sleeping member once per message, so a dropped wake leaves the team waiting
  for good. `agent run --resume` refuses a team that still has a `sleeping` member, so it could not recover this either.

The topic asks that *resuming* normal teams is not possible. Now:
- `agent run --resume <team>` is refused, as in PR #60. The fix pass and the next `--autopilot` round go through it,
  and the human is told.
- `agent answer` / `agent resume` of a waiting member is refused, with the reason (`human_resume_allowed`).
- A message wake (and a pause's end) of a team member is **deferred**, not dropped. The task gets `WAKE_DEFERRED=1`
  (log `wake_deferred_investigation`). `agent _supervise` (every `3` min) replays it with `supervise_deferred_wakes`
  (log `wake_replayed`) once no investigation blocks the task. This keeps PR #60's test
  `dashboard independent investigation blocks message wake`.
- Checks, reviews and bounces of a team already at work go on.
- A finished cohort still marked `waiting` (`investigation_completed_wait`) no longer blocks.

`automation_allowed` is now only the human's stop fence.

### 3. `agent team <id> --stop` rewrote finished members

`team_stop` ran `cmd_stop` on every member and delegated descendant, including `done` and `failed` ones. Each got
`STATUS=stopped` and a new `FINISHED`. Now only tasks that were not finished before the stop become `stopped`.
`done`, `failed`, `excluded` and `hung` keep their status and time. The stop marks (`stop-requested`,
`TEAM_STOPPED=1`) are still written on all of them, so a finished coordinator's pending review or merge is still fenced.

### 4. A stopped task's `agent resume` failed silently

It returned `1` with no output. It now says the task was stopped and how to start it again
(`agent run --resume <team|task>`).

### 5. Item 5: why `claude-haiku` showed `sleeping`

From `~/agents/logs/agent.jsonl` for `20261104-122828-claude-haiku`:
- `11:46:14Z`: `task_kicked from=web`. The web ■ button ran `agent kick`; by design a kicked member sleeps.
- `12:01:29Z`: `stop_all` (`agent stop --running`) set the task to `STATUS=stopped` but never updated the bus member
  state, so the team views kept showing `sleeping`.

PR #60 fixes both:
- `stop_all`, `cmd_stop` and `finish_team` now write `stopped` on the bus.
- The web ■ button runs `agent stop` for local team members.
- The views prefer the task's `stopped` status.

### 6. Item 6: Serena is installed but off on this host

- `serena` is installed: `~/.local/bin/serena` → `~/.local/share/uv/tools/serena-agent/bin/serena`.
- The main checkout has `.serena/project.yml`.
- But `~/agents/agents.conf` has **no `AGENT_SERENA` line**. The installed `bin/agent` defaults it to empty, so no run
  gets the server. This run's `mcp.json` has no servers.

After PR #60, `install.sh` adds `AGENT_SERENA="serena"` to an existing `agents.conf` when `serena` is found.
**The human must run `install.sh` (or add that line by hand); agents may not.** The tests check the configuration
given to each of the `4` CLIs and the prompt text. They cannot prove that a model actually chooses Serena's tools.

### 7. Item 4: the background jobs report was partly invented

PR #60's section 4 of [`docs/investigations/dashboard/merged.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4030df13af4c710107becc70021e233dfc67b66a/docs/investigations/dashboard/merged.md)
listed:
- subcommands that do not exist: `_unpause`, `_investigation-controller`, `_remote_member_done`;
- unit names that do not exist: `agent-invteam-*`, `agent-rmember-*`, `agent-rquestion-*`;
- a consultation timer that does not exist;
- wrong values: a `20` min hang limit (the default is `30`), and `systemd-run` as the way agentbus starts wakes (it
  uses `setsid -f`).

The section is replaced by an inventory checked against every `systemd-run` and `spawnDetached` call and the unit files:
- `4` standing units: `agent-supervise.timer` (every `3` min), `agentweb`, `agentlead`, `agentnode`;
- `8` kinds of transient `systemd-run` units: runner, precheck (×`2`), fix pass, merge, wake timer, answers path
  watch, remote questions;
- `4` kinds of detached processes started by `agentbus`: `_wake`, `_remote_member`, `_dispatch-helpers`,
  `_remote_questions`;
- the polling loops inside them.

### 8. Item 7: `## Kontekst`

`start_helper` wrote the requester's context under the Polish heading `## Kontekst`. It is `## Context` now, and the
audit marks its item `11` resolved. Item `10` (a remote helper gets the solo protocol) is still open, as the audit says.

## Commits

- [`87433fc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/87433fc35fe7f2b550ede35b09d2bb6a4e72101c) — $\color{red}{\textbf{RED}}$: `8` tests in [`tests/unit/team_stop_followup.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4030df13af4c710107becc70021e233dfc67b66a/tests/unit/team_stop_followup.bats); `7` failed on PR #60, `1` guards behaviour that moves
- [`6852b0e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6852b0e883cedec740038d8c534756b235eae3f8) — $\color{green}{\textbf{GREEN}}$: findings 1–4 and 7 in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4030df13af4c710107becc70021e233dfc67b66a/bin/agent), [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4030df13af4c710107becc70021e233dfc67b66a/README.md), [`docs/investigations/dashboard/merged.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/4030df13af4c710107becc70021e233dfc67b66a/docs/investigations/dashboard/merged.md)
- [`747818a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/747818a9f295ffc1025fae8ccb82a2612b50697c) — $\color{red}{\textbf{RED}}$: the helper context heading test
- [`4030df1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4030df13af4c710107becc70021e233dfc67b66a) — $\color{green}{\textbf{GREEN}}$: `## Context`, and [the audit](https://github.com/mateusz-szczyrzyca/team-agents/blob/4030df13af4c710107becc70021e233dfc67b66a/docs/investigations/i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md) updated

## Checks

- `make gate` on PR #60's head `c02a062` before any change: `8/8` steps.
- `make gate` after the last commit: `8/8` steps, `0` lint findings, `589` bats tests.
- `bats tests/unit/team_stop_followup.bats`: `9/9`.
- `bin/agent round-size 20261104-200148-inv-opus` (PR #60 and this branch against `main`): `2,005` lines
  (`+1,877` / `−128`) in `41` files. That is `5` lines over the `2,000` soft limit and within the `3,000` hard limit.
  This branch alone, on top of PR #60: `+296` / `−66` in `6` files, plus this report.
- No existing test was changed. `9` rows were added to `tests/agent-go-map.tsv` for the new tests.

## Flaky tests seen (not caused by this change)

Two of the four gate runs with these changes each failed one existing test that passes alone. Neither touches the changed code.
- `wake_cmd.bats` "agent wake on a remote member's proxy task asks its node to resume it": `5/5` passes alone. It
  rewrites `meta.env` while the member's runner may still be writing it.
- `investigation_team_admission.bats` "a leased parent without manifest keeps the lease while its tagged child is
  still running": `4/4` passes alone. Under load the child can still be `starting` when `_supervise` looks.

## Compatibility

- New optional `meta.env` key `WAKE_DEFERRED` (`1` or empty); a missing key means no deferred wake.
- New log events `wake_deferred_investigation` and `wake_replayed`. The pause timer's `wake_deferred` keeps its meaning.
- `agent answer` / `agent resume` of a stopped task now exit with a message instead of silently with `1`.
- No bus format change beyond PR #60's additive `stopped` member state.

## Not done / for the merger and the human

- `AGENT_SERENA` on this host: run `install.sh` or add `AGENT_SERENA="serena"` to `~/agents/agents.conf` (finding 6).
- A review blocked by model capacity fails the whole investigation (`INVESTIGATION_REVIEW_RETRIES=1`, one possible
  reviewer). A retry after a capacity error, or another reviewer, would have let PR #60 be reviewed. Not changed here.
- The next `--autopilot` round refused by an investigation is reported to the human, not deferred like a wake.
- Item `10` of the worker-prompt audit (the remote helper protocol) is still open.
- Suggested addition to `AGENTS.md`: "a function called in `bin/agent` must be defined; `shellcheck` does not check
  this". The `team_is_stopped` bug passed every check.
