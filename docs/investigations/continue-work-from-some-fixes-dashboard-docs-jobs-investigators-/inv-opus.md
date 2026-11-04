# Fixing the review of PR #61 — `inv-opus` (task `20261104-221457-inv-opus`)

Topic: continue the work of `some-fixes-dashboard-docs-jobs-investigators.md`; check the review comments of
[#61](https://github.com/mateusz-szczyrzyca/team-agents/pull/61) and fix the issues.

The review is [the one by `ms-ai-agents` on #61](https://github.com/mateusz-szczyrzyca/team-agents/pull/61#pullrequestreview-5774854213): `4` major and `5`
minor findings, plus `7` line comments that repeat them. There are no comments or reviews by other accounts on #61.

**All `9` findings are fixed** on branch `agents/inv-opus/20261104-221457`.

## How the branch is built

#61's head branch `agents/inv-opus/20261104-203141` belongs to another task, and this task may push only its own
branch. So:
- the branch is reset to #61's head [`6ac8698`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6ac869862988c16e922e252a101259a18520e69e), so it
  contains all of #61 (and #60) unchanged;
- `main` is merged in, because #61 conflicted with it: [`3774353`](https://github.com/mateusz-szczyrzyca/team-agents/commit/377435356721c4e3326456de0ee0eebb8bb61c30);
- the fixes are `3` commits on top.

The branch **fast-forwards from #61's head**. A merger can push it to #61's branch, or the human can decide (see
"For the merger and the human").

**The conflict:** a single hunk in `cmd__merge`. `main` sets `PR_MERGE_OWN=1` before `gh pr merge`, and #61
checks `automation_allowed` there. Both are kept, with the stop check first, so a stopped coordinator neither merges
nor marks the merge as its own.

## Commits

- [`3774353`](https://github.com/mateusz-szczyrzyca/team-agents/commit/377435356721c4e3326456de0ee0eebb8bb61c30) — merge of `main` into #61's head
- [`bb1f72c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/bb1f72c67ac0f3096828fd319afe5a838200975b) — $\color{red}{\textbf{RED}}$:
  - `18` bats tests in [`tests/unit/team_stop_review.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/tests/unit/team_stop_review.bats): `12` fail, `6` are guards that pass
  - `2` Go tests in [`live/web_team_stop_done_test.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/live/web_team_stop_done_test.go): `1` fails
  - their rows in [`tests/agent-go-map.tsv`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/tests/agent-go-map.tsv)
- [`265b651`](https://github.com/mateusz-szczyrzyca/team-agents/commit/265b651088c80a22d6b6912a673de4f768f67559) — $\color{green}{\textbf{GREEN}}$: the fixes, in:
  - [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/bin/agent)
  - [`install.sh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/install.sh)
  - [`prompts/pr-ownership.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/prompts/pr-ownership.md) and [`prompts/inv-merge.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/prompts/inv-merge.md)
  - the two team templates ([`teams.html`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/live/web/templates/teams.html), [`team.html`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/live/web/templates/team.html))
- [`477fff7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/477fff74303bd1412609430a72d7272350cc3bf6) — documentation: [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/README.md),
  [`docs/investigations/dashboard/merged.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/docs/investigations/dashboard/merged.md), the
  `dashboardView` comments in [`live/web_views.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/live/web_views.go) (`Test-First: not possible`, docs only)

## The findings and what was done

### Major 1 — a single `agent stop` told nobody who waited for the task

**Fixed.** The early return in `runner_exit` now calls `stopped_handover`
([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/bin/agent)). It does not run the stopped task's own follow-up: no notification, wake,
review or fix pass. But it tells whoever waits for the task, as `main` did:
- an agent-requested investigator → `inv_finish … failed`, which wakes the requester; for an `--autopilot` review, also `review_without_verdict`
- a helper → `helper_finish … failed`
- a team member → `member_out`: its claimed board items go back and the other members are told
- then `dispatch` and `dispatch_helpers`

The review's sketch differs from the fix in four ways:
- **`inv_finish` gets `failed`, not `stopped`.** `stopped` is not an `InvestigationStatus`
  (`agentbus/types.go`), so `agentbus inv-set` would have rejected it and the requester would still wait.
- **No `member_out` when the team is closed** (`team_done_flag`). `agent stop <coordinator>` runs `finish_team`
  first, and nobody is left to take the work over.
- **No `member_out` for a remote team's member** (`REMOTE_TEAM=1`), as in `team_after_run`: the lead follows its state.
- **No `member_out` for a member of a team stopped with `agent team <id> --stop`** (`TEAM_STOPPED=1`), as the review suggested.

Tests:
- `a member stopped by the human gives its board items back and its coordinator is told` (real bus: the board item is `todo` again, `member_out` reaches the coordinator)
- `an investigator requested by an agent and stopped by the human delivers a failed result to its requester`
- `a helper stopped by the human sends a failed result to the agent that delegated to it`
- guards:
  - `a member of a team the human stopped as a whole is not reported out and keeps its board items`
  - `the members of a closed team are not told that its stopped coordinator is out`
  - `an independent investigator stopped by the human delivers nothing to a requester`

### Major 2 — a `hung` investigation member blocked normal teams for good

**Fixed.** The new `investigation_over` treats `done|failed|stopped|excluded|hung` as finished, the same set
`agent ack` and the node code use, plus #60's completed-handoff case. `blocking_investigation` also skips a member
whose cohort (`INVESTIGATION_TEAM`) has ended, whatever the member's own status says. That was the review's
"better still" option. It is read with `INVESTIGATION_TEAM=""`, because `task_field` falls back to the caller's
environment.

Tests:
- `a hung or excluded member of a finished --investigation does not block a normal team`
- `a hung member blocks nobody even while its --investigation's record is not finished`
- `a member still marked running does not block a normal team once its --investigation was stopped`
- guard: `a running member of an --investigation in progress still blocks a normal team`

### Major 3 — `install.sh` overrode an explicit Serena opt-out

**Fixed.**
- The `sed -i` on an existing `agents.conf` is gone. An existing value, including `AGENT_SERENA=""`, is never changed.
- A missing `AGENT_SERENA` line is added only when Serena is installed, and `install.sh` prints what it added.
- A **fresh** `agents.conf` gets `AGENT_SERENA=""` when Serena is not installed, with a message. So the template
  no longer names a missing command (the review's "separately" point).

Tests (they run the real `install.sh` with stub `go` / `systemctl`):
- `install.sh keeps an explicit empty AGENT_SERENA in an existing agents.conf, Serena installed`
- `install.sh adds AGENT_SERENA to an existing agents.conf without it when Serena is installed, and says so`
- `install.sh leaves Serena off in a fresh agents.conf when it is not installed, and says so`
- guard: `install.sh enables Serena in a fresh agents.conf when it is installed`

### Major 4 — the PR ownership rule had no way out for a coordinator whose subject PR is on another branch

**Fixed in the prompts. I chose the review's option (b), not (a).** Option (a), a new stacked PR, is exactly what
the human's item `8` forbids: "never create a new PR". Option (b) asks the human instead:
- [`prompts/pr-ownership.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/prompts/pr-ownership.md): a non-coordinator reports to its
  coordinator. A coordinator whose matching PR's head branch is not its own does not push to that branch and does
  not open a PR. It pushes its own branch, asks in its questions file (naming the PR and its branch), and ends with
  `STATUS: QUESTIONS`.
- [`prompts/inv-merge.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/477fff74303bd1412609430a72d7272350cc3bf6/prompts/inv-merge.md): the same for the Phase 2 merger. It first merges
  that PR's head into `{{BRANCH}}`, so either answer can be carried out at once. The prompt also explains why it
  cannot continue the PR itself: it pushes only `{{BRANCH}}`, and the precheck looks for the PR of `{{BRANCH}}`.

Test: `the PR ownership rules tell a coordinator what to do when the subject's PR is on a branch it may not push`
(solo and merger prompts).

**This topic is itself such a case.** The Phase 2 merger of this investigation will find #61 open on a branch that
is not its own. Under the new rule, it asks the human.

### Minor findings

1. **A fresh session got the rules twice — fixed.** `build_prompt … fresh` sets `local RULES_RENDERED=1` before
   its nested `resume` part, which then skips `continuation_rules`. Test: `a fresh session's prompt has the PR
   ownership rules and the Serena part once`.
2. **`agent answer` refused too early — fixed.** The stop check is split out of `human_resume_allowed` into
   `human_stop_respected`; `cmd_answer` uses only that.
   - The resume is still refused by `agent resume`.
   - It is also refused by the answers path unit (`cmd__answers_saved` → `cmd_resume`), which notifies the human
     to save again later.

   Tests:
   - `answers to a waiting member can be written while --investigation is in progress; the resume is refused`
   - guard: `a stopped task's answers are still refused before the editor opens`
3. **The stop button showed on finished teams — fixed.** `/teams`: `stopped`, or the button only when the team
   is not `Done`. `/team/<id>`: `{{if not (or .Stopped .T.Done)}}`.

   Tests:
   - `TestFinishedTeamOffersNoStop`
   - guard: `TestStoppedFinishedTeamSaysStopped`
4. **The docs described the 20-entry cut wrongly — fixed.** The README, the dashboard report and the
   `dashboardView` comments now say: the first `20` entries in display order. That is remote jobs at work, then
   local tasks at work, then finished ones, the most recently finished first. Creation time does not choose them.
5. **Autopilot stayed off after a resume — documented.** The README's "Stopping a team" now says that a resumed
   cohort inherits `AUTOPILOT_OFF=1`. Use `--autopilot` or `agent autopilot <team> on`. The behaviour is not
   changed: a stop that keeps autopilot off is what item `2` asks for.

## Checks

`make gate` on [`477fff7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/477fff74303bd1412609430a72d7272350cc3bf6), with this run's `AGENT_*` and `AGENTBUS_BIN` variables removed from
the environment (see below):
- all `8` steps passed
- `0` lint findings
- `570/570` + `42/42` unit bats tests
- `57/57` harness tests
- Go tests ok

Other checks:
- The RED commit fails only on its new tests: lint `0` issues, the test map valid.
- Size against `main`: `2,622` lines (`+2,493` / `−129`) in `44` files, #60 and #61 included. This task's
  part is `515` lines (`+484` / `−31`) in `12` files.
- Existing tests: `git diff --numstat origin/main...HEAD` over `*_test.go` and `tests/**` shows `0` deleted lines.

## Problem found: the bats suite picks up the running installation's `AGENTBUS_BIN`

With this run's environment, the existing test `stopping a team keeps its finished members done, with their
finishing time` (`tests/unit/team_stop_followup.bats`) fails. It fails the same way on #61's head `6ac8698`.

**The cause:** `AGENTBUS_BIN`, set by the running installation, is inherited by the tests that source
`bin/agent`. They then call the **installed** (older) `agentbus` instead of the one the suite builds. The bus
directory is still the test's temporary one, so the real bus is not touched. Unset, the test passes.

#61's merger hit the same thing and worked around it the same way. Suggestion (not done, rule 1 forbids changing
`tests/unit/helpers.bash`): have `setup()` unset `AGENTBUS_BIN`, or have `make gate` clear it.

## Compatibility

- **No persisted format changes.**
- **One new log event:** `member_out_failed`, for a failed `member_out` during a hand-over. The other events it uses
  already existed: `investigation_finish_failed`, `helper_finish_failed`, `review_followup_failed`, `dispatch_failed`.
- **`install.sh` no longer changes an existing `AGENT_SERENA`.** It only adds a missing line, and says so. It
  never rewrites an existing value.
- **The `agent answer` exit status during an `--investigation` changes.** Before, it refused before the editor
  opened. Now it opens the editor, and the following resume is refused with the same message.

## Lint suppressions

None. The gosec finding G304 in the new Go test is avoided by reading through `os.DirFS`.

## For the merger and the human

- **The branch fast-forwards from #61's head.** It contains #60, #61, `main` and these fixes.
- **Two ways to land it:**
  - push it to #61's branch `agents/inv-opus/20261104-203141`; that is the human's or #61's owner's call;
  - or answer the merger's question under the new rule (major 4).
- **Not done:** `AGENTBUS_BIN` leaking into the bats suite (above). It needs a change to the test helpers or the
  Makefile, and the human's approval.
- **A suggestion for AGENTS.md:** say that the gate must run without the runner's `AGENT*` variables.

---

# Earlier report (task `20261104-200148-inv-opus`), kept unchanged

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
