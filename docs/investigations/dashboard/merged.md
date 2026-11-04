# Dashboard, Team Lifecycle, Background Jobs, Serena MCP, and Prompt Rules — Phase 2 Merged Report

- **Phase 2 merger:** `inv-gemini-argon` (`gemini/gemini-4-argon-high`, task `20261104-171402-inv-gemini-argon`, branch `agents/inv-gemini-argon/20261104-171402`)
- **Base commit:** [`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f)
- **Phase 1 investigators:**
  - `inv-astra` (`codex/gpt-6-astra`, task `20261104-160248-inv-astra`): [`acfed8b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/acfed8b510d9fbc709074f6e79c51fd4d131a333) — Phase 1 report in [`inv-astra.md`](inv-astra.md)
  - `inv-gemini-argon` (`gemini/gemini-4-argon-high`, task `20261104-160248-inv-gemini-argon`): [`01366c8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/01366c886f253e3dc34b255fe76254af2c5b3b00) — Phase 1 report in [`inv-gemini-argon.md`](inv-gemini-argon.md)

---

## 1. Compact 20-entry dashboard and `"Full dashboard view"`

### Integrated solution

- **Compact default view (`GET /`):**
  - `dashShown = 20` in `live/web_views.go` caps `GET /` to the `20` newest eligible entries (selected by creation timestamp `CreatedAt` across local tasks and recent remote lead jobs, then ordered with active tasks first and newest finished tasks second).
  - When entries are truncated (`Cut: true`), `dash-table` in `live/web/templates/partials.html` renders a footer link to `/dashboard/full`.
- **Full dashboard view (`GET /dashboard/full`):**
  - Added `"Full dashboard view"` to the left navigation menu in `live/web/templates/layout.html` (`<a href="/dashboard/full" data-views="dashboard_full">Full dashboard view</a>`).
  - Added `GET /dashboard/full` (`viewDashboard` / `viewDashboardFull` with `s.pages["dashboard_full"] = s.pages["dashboard"]`) and SSE stream `GET /events/dashboard/full` (plus `?all=1` compatibility) retaining all local tasks and remote jobs regardless of the `24h` acknowledged-task cutoff (`remoteDashboardRows(busDir, full)`).
- **Tests:**
  - `TestDashboardCompactAndFullHistory` (`live/web_dashboard_controls_test.go`, from `inv-astra`)
  - `TestDashboardShowsAtMost20LatestEntriesAndFullDashboardViewShowsAll` (`live/web_test.go`, from `inv-gemini-argon`)
  - `dashboard shows only 20 latest entries by default and Full dashboard view shows all` (`tests/unit/web.bats`, from `inv-gemini-argon`)

---

## 2. `/teams`: `"Stop this team"` and durable automation fence

### Integrated solution

- **Web endpoints and UI (`live/`):**
  - Added `POST /team/{id}/stop` (`postTeamStop` in `live/web_views.go`), calling `agent team <id> --stop` after validating team existence (`404` when absent, `409` on script error).
  - Added `"Stop this team"` buttons (`hx-post="/team/{{.ID}}/stop"` with confirmation dialog) in `live/web/templates/teams.html` and `live/web/templates/team.html`.
- **CLI and runner fence (`bin/agent`):**
  - Added `agent team <id> --stop` (`team_stop` in `bin/agent`), which discovers all team member tasks and transitive local delegated descendants (`PARENT_TASK` walk), writes `stop-requested` and `TEAM_STOPPED=1`, disables autopilot (`AUTOPILOT_OFF=1`, `VERIFY=0`, clears `REVIEW_PAIR_PENDING` / `REVIEW_SLOT_PENDING` / `AUTO_STAGE`), stops active local tasks, marks sleeping members `stopped` on both `meta.env` and `agentbus` (`bus team-set --state stopped`), and finishes the bus team (`finish_team`).
  - Added `task_stop_requested` (walking `PARENT_TASK` with cycle guard, checking both `stop-requested` and `TEAM_STOPPED=1`) and `automation_allowed` fences in `start_task_unit`, `cmd__merge`, `start_precheck`, `cmd__precheck`, `review_precheck`, `review_followup`, `review_fix_pass`, `cmd__fixpass`, `do_fix_pass`, `retry_review_slot`, `review_pair_done`, `start_review_pair`, `request_review`, `cmd__wake`, `start_investigation`, `start_helper`, `cmd__runner`, `cmd_answer`, and `cmd_resume`.
- **Tests:**
  - `TestDashboardStopTeamAction` (`live/web_dashboard_controls_test.go`, from `inv-astra`)
  - `TestTeamsPageStopTeamButtonAndEndpoint` (`live/web_test.go`, from `inv-gemini-argon`)
  - `dashboard team stop persists every member and prevents wake`, `dashboard stopped automation cannot launch a fix pass or review`, `dashboard stopped runner exits without restarting team automation` (`tests/unit/dashboard_lifecycle.bats`, from `inv-astra`)
  - `stopping a team stops its members and prevents automatic resume` (`tests/unit/team_close.bats`, from `inv-gemini-argon`)

---

## 3. `--investigation` priority over normal team resumption in the same project

### Integrated solution

- Added `normal_resume_allowed` in `bin/agent`:
  - Checks whether the target task belongs to a normal team (`TEAM` non-empty, `INVESTIGATION_TEAM` empty, `KIND` in `worker|remote-member|""`).
  - Scans `$TASKS/*/meta.env` for any non-terminal (`STATUS` not `done|failed|stopped`) top-level `--investigation` coordinator (`KIND=investigation_team`) or phase child (`INVESTIGATION_TEAM` non-empty) in the same repository (matching `REPO_ROOT` when present on both, or `REPO_NAME`).
  - Ordinary investigations requested by normal models (`KIND=investigation` with empty `INVESTIGATION_TEAM`) and investigations in other repositories do not block resumption.
  - Enforced in `resume_run` (before mutating team/task state; the fix pass and the next `--autopilot` round go through it and report the refusal), in `agent answer` / `agent resume` (`human_resume_allowed`), and in `cmd__wake`, which defers the wake (`WAKE_DEFERRED=1`) for `supervise_deferred_wakes` to replay once the investigation ends.
  - Follow-up by `inv-opus` (`20261104-200148-inv-opus`): the first version also checked it in `start_runner` (`mode != new`) and `automation_allowed`. That dropped the precheck, reviews and review results of a normal team already at work (with status `0`, nothing recorded), refused its bounces and pause resumes (left stuck), and lost every held-back wake (the bus wakes a member once per message). A finished cohort still marked `waiting` (`investigation_completed_wait`) no longer blocks.
- **Tests:**
  - `dashboard independent investigation blocks normal resume before mutation`, `dashboard independent investigation blocks message wake`, `dashboard ordinary investigations and other projects do not block resume` (`tests/unit/dashboard_lifecycle.bats`, from `inv-astra`)
  - `resuming a normal team in the same project is refused while --investigation is in progress, but allowed for normal model investigations` (`tests/unit/investigation_team_admission.bats`, from `inv-gemini-argon`)
  - `tests/unit/team_stop_followup.bats` (`8` tests, from `inv-opus`): deferred and replayed wakes, precheck and bounces of a team at work, answers refused, a finished cohort still marked `waiting`, finished members kept by `agent team <id> --stop`.

---

## 4. Background jobs report: inventory, where defined, what they do, and periods

Re-verified against the code by `inv-opus` (task `20261104-200148-inv-opus`): the first version of this section named
units and subcommands that do not exist (`_unpause`, `_investigation-controller`, `_remote_member_done`,
`agent-invteam-*`, `agent-rmember-*`, `agent-rquestion-*`, a consultation timer). Every row below names a `systemd-run`
call, a `spawnDetached` call or a unit file that is in the code.

When no agent CLI runs, work still happens through **4 standing `systemd --user` units** (2 of them only on a lead or
a node), **8 kinds of transient `systemd-run` units** started by `bin/agent`, **4 kinds of detached `agent` processes**
started by `agentbus`, and the polling loops inside those processes.

### 4.1 Standing `systemd --user` units (written by `install.sh`)

| # | Unit | Runs | Period / trigger | What it does |
|---|---|---|---|---|
| `1` | `agent-supervise.timer` → `agent-supervise.service` | `agent _supervise` (`cmd__supervise`) | `OnBootSec=3min`, `OnUnitActiveSec=3min` | cleans answer watches nobody waits for (`supervise_answer_watches`); retries remote question files (`supervise_remote_questions`); replays wakes an `--investigation` held back (`supervise_deferred_wakes`); a `running`/`starting` task without an active unit is failed after `AGENT_SUPERVISE_GRACE_SEC` (`120` s) or, after a reboot, resumed (`restart_interrupted`); on a lead cancels remote member jobs whose team ended (`supervise_member_jobs`); reports an open team with no local member alive for `AGENT_TEAM_ABANDON_MIN` (`60` min) once (`supervise_teams`); reports review pairs waiting longer than `AGENT_REVIEW_WAIT_MIN` (`30` min) (`supervise_review_waits`); runs `dispatch` while an `--investigation` cohort is pending or a review rotation waits — that is what advances the `--investigation` phases between runs |
| `2` | `agentweb.service` | `agent web` (`live/web*.go`) | always on | the web interface; its live views re-read the files every `1` s; its buttons call `agent stop`, `agent kick`, `agent team <id> --stop`, `agent run --resume`, `agent resume` |
| `3` | `agentlead.service` (only with `LEAD_LISTEN`) | `agent lead` → `agentbus lead` | always on; a `30` s ticker | the multi-host bus API; every `30` s puts remote members whose node stays away longer than the grace out of their teams (`checkAbsentMembers`) |
| `4` | `agentnode.service` (only with `LEAD_URL`) | `agent node` | a loop every `NODE_POLL_SEC` (`5` s) | heartbeats to the lead; starts, wakes, kicks and cancels the jobs the lead assigns; pushes logs and artifacts |

### 4.2 Transient `systemd-run --user` units (all in `bin/agent`)

| # | Unit name | Runs | Started by | When | What it does |
|---|---|---|---|---|---|
| `5` | `agent-<task>-r<run>` | `agent _runner <task> <mode>` | `start_runner` | every run: a new task, a resume, a wake, an answer, a bounce (questions, status, round, `--verify`) | the agent CLI's run with its watchdog, then what its `STATUS:` line asks for (`team_after_run`, reviews, `dispatch`) |
| `6` | `agent-<task>-precheck-<epoch>` | `agent _precheck <task>` | `start_precheck` | a `--verify`/`--autopilot` round ends | waits for the team to settle (`team_wait_settled`, `5` s polls, `10` min at most), checks tests-first and the PR, runs the gate in a temporary worktree (`AGENT_REVIEW_GATE_MIN`, `30` min), then requests the review |
| `7` | `agent-<parent>-precheck-<n>` | `agent _precheck <parent>` | `investigation_pipeline_tick` | an `--investigation`'s merger has published its Phase 2 result | the same checks for the merged result before its review pair |
| `8` | `agent-<task>-fixpass-<epoch>` | `agent _fixpass <task>` | `review_fix_pass` → `start_task_unit` | a review found critical/major problems | waits for the team to settle, then resumes it with the findings (`agent run --resume … --same-round`), `VERIFY_MAX_FIX_PASSES` times at most |
| `9` | `agent-<task>-merge-<epoch>` | `agent _merge <task>` | `review_pair_done` → `start_task_unit` | `--autopilot`: both reviews say ready to merge | merges the PR, runs the gate on the merged base, starts the next round (`agent run --resume <team> --autopilot`) |
| `10` | `agent-<task>-wake-<epoch>` (timer, `--on-active`) | `agent _wake <task>` | `schedule_wake` from `pause_task` and `long_run_end` | a pause ends (provider limit or quota, network, the wake-loop guard), or `5` s after a long run ended on request | resumes the paused task, or starts the member's next run in a fresh session |
| `11` | `agent-<task>-answers-<round>` (path unit, `PathModified=`) | `agent _answers_saved <task> <round>` | `watch_answers` | the questions file of a waiting task is saved | resumes the task once every question is answered |
| `12` | `agent-remote-questions-<task>@<node>` | `agent _remote_questions_ready <task>@<node>` | `cmd__remote_questions` (the lead) | a node's question file arrived | opens it for the human and notifies |

`agent authenticated` also uses `systemd-run` (`agent-auth-*`, `_authcheck`) but waits for it in the foreground (`--wait`):
it is not a background job.

### 4.3 Detached `agent` processes started by `agentbus` (`spawnDetached`, `setsid -f`, no unit)

| # | Runs | Where | When |
|---|---|---|---|
| `13` | `agent _wake <task>` | `agentbus/team.go` (`spawnWake`) | a message or new board work for a sleeping team member |
| `14` | `agent _remote_member <task> <status> <branch> <note>` | `agentbus/remote_member.go` | on the lead: a remote member's job reports its end |
| `15` | `agent _dispatch-helpers` | `agentbus/collab.go` (`agentCmdFn`) | a normal model queues a worker request (`delegate`) |
| `16` | `agent _remote_questions <task>@<node>` | `agentbus/lead_command.go` (`--questions-cmd`) | on the lead: a node sends a question file; it starts unit `12` |

### 4.4 Loops inside these processes

- `watch_run` (in every `_runner`): every `AGENT_HANG_POLL` (`15` s) — the hang watchdog (`AGENT_HANG_MIN`, `30` min
  without activity), long-run warnings and limits.
- `review_precheck` (in `_precheck`): the gate's progress written for the dashboard every `AGENT_STAGE_POLL_SEC` (`2` s).
- `team_wait_settled` (in `_precheck`, `_fixpass`, `_merge`): `5` s polls, `120` times at most.
- `agent live`: a `1` s ticker; `agent web`: `1` s file polls per open live view.

### 4.5 Why something happens while no agent runs

1. Every `3` minutes `agent _supervise` runs (row `1`): it ends dead runs, reports, and dispatches queued
   investigations, review pairs and the next `--investigation` phase.
2. After a round ends, `_precheck` runs the gate for minutes (rows `6`, `7`); after the reviews, `_fixpass` or `_merge`
   (rows `8`, `9`) wait for the team to settle and then resume it or merge.
3. Timers and watches fire on their own: a pause's end (row `10`), a saved answers file (row `11`).
4. `agentbus` starts `agent` processes when a message arrives for a sleeping member or a worker request is queued
   (rows `13`–`16`).

---

## 5. Stopped model shown as `stopped` instead of `sleeping` (`20261104-122828-team` / `claude-haiku`)

### Forensics of `20261104-122828-team` (`20261104-122828-claude-haiku`)

- In `/home/dev/agents/tasks/20261104-122828-claude-haiku/meta.env`, `claude-haiku` had `STATUS=sleeping`, `KICKED=2026-11-04T13:19:50+01:00`, `FINISHED=2026-11-04T12:41:59+01:00`, and `~/agents/bus/state.json` recorded `"state": "sleeping"`.
- Root causes identified across both Phase 1 investigations:
  1. `live/web_views.go` (`postStop`, `POST /task/{id}/stop`) unconditionally called `agent kick <id> web` even for local team members, which is a temporary run interruption (`sleeping`) rather than a durable stop (`agent stop`).
  2. `agentbus/types.go` (`MemberState`) had no `stopped` value (`MemberActive`, `MemberSleeping`, `MemberPaused`, `MemberDone`, `MemberExcluded`, `MemberAbsent`), and `bin/agent` mapped `stopped` members to `done` or `sleeping`.
  3. Even when `cmd_kick` ran after the team was already closed (`team_done_flag == true`), it still overwrote the member's `STATUS` and bus state to `sleeping`.
  4. `live/web_views.go` (`viewTeams` and `viewTeam`) rendered `m.State` from `bus/state.json` without checking whether the member's task had `STATUS=stopped` or `TEAM_STOPPED=1`.

### Integrated solution

- Added `MemberStopped MemberState = "stopped"` to `agentbus/types.go`, allowed it in `TeamSetArgs` (`agentbus/team.go`), and mapped `stopped` remote member jobs to `MemberStopped` (`agentbus/remote_member.go`).
- Updated `postStop` (`live/web_views.go`) to invoke `agent stop <id>` for local team members (retaining `agent kick` for remote jobs).
- Updated `finish_team`, `stop_all`, `cmd_stop`, `cmd__wake`, `cmd_kick`, and `team_stop` (`bin/agent`) to record `stopped` in both `meta.env` and `agentbus` (`bus team-set --state stopped`).
- Added `effectiveMemberState` (`live/web_views.go`) so `/teams` and `/team/{id}` always display `stopped` (with `st-stopped` orange badge) when either `m.State == "stopped"`, `task.Status == "stopped"`, or `task.TeamStopped` is set.
- **Tests:**
  - `TestMemberStoppedState` (`agentbus/types_test.go`, from `inv-gemini-argon`)
  - `TestDashboardStopMemberPersistsIntent`, `TestDashboardTeamDisplaysStoppedMember` (`live/web_dashboard_controls_test.go`, from `inv-astra`)
  - `TestStoppedTeamMemberIsNotShownAsSleeping` (`live/web_test.go`, from `inv-gemini-argon`)
  - `kicking a team member after its team is done marks it stopped, not sleeping`, `stopping a team member on demand marks its bus member state as stopped, not sleeping` (`tests/unit/kick.bats`, from `inv-gemini-argon`)

---

## 6. Serena MCP server verification across all models and prompts

### Verification and integrated solution

- On this host, `serena` is installed at `/home/dev/.local/bin/serena` (on `RUN_PATH`), and `bin/agent` wires `serena start-mcp-server --context <ctx> --project "$WORKTREE" --open-web-dashboard false` for all 4 CLIs:
  - `claude` → `--context claude-code` (`mcp_add_serena`)
  - `codex` → `--context codex` (`codex_serena_flags`, `startup_timeout_sec=120`)
  - `gemini` (Antigravity) → `--context antigravity` (`mcp_add_serena` in `$WORKTREE/.agents/mcp_config.json`)
  - `opencode` → `--context ide` (`opencode_mcp_json` in `OPENCODE_CONFIG_CONTENT`)
- Fixed 3 gaps found in Phase 1:
  1. **Default configuration (`config/agents.conf`, `install.sh`):** Enabled `AGENT_SERENA="serena"` in `config/agents.conf` and updated `install.sh` to auto-enable `AGENT_SERENA="serena"` when `serena` is installed.
  2. **Worktree language-server seeding (`bin/agent`):** Updated `worktree_exclude` so when `.serena/` is excluded for a run's worktree and `$REPO_ROOT/.serena/project.yml` exists, it copies `project.yml` into `$WORKTREE/.serena/project.yml`.
  3. **Prompt coverage across all modes and continuations (`bin/agent`, `prompts/serena.md`):** Added `continuation_rules` so `prompts/serena.md` (naming `initial_instructions`, `serena_repl`, `get_symbols_overview`, `find_symbol`, `find_referencing_symbols`) is included in every role (`solo`, `team`, `helper`, `investigation`, `review`, `inv-work`, `inv-merge`) and every session mode (`new`, `resume`, `fresh`, and watchdog `hang-resume`).
- **Tests:**
  - `dashboard PR ownership and Serena guidance reach every role and continuation`, `dashboard resume does not advertise Serena when disabled` (`tests/unit/dashboard_prompt_modes.bats`, from `inv-astra`)
  - `config/agents.conf enables AGENT_SERENA=serena and worktrees inherit .serena/project.yml from REPO_ROOT`, `every model CLI (claude, codex, gemini, opencode) receives the Serena prompt and invokes the serena MCP server when needed` (`tests/unit/serena.bats`, from `inv-gemini-argon`)

---

## 7. Verification and update of `docs/investigations/i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md`

### Reconciliation of Phase 1 audits

- Updated `## 7. Summary matrix` and `## 8. Inconsistencies found` in [`docs/investigations/i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md`](../i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md).
- Reconciled a Phase 1 discrepancy on items `5` and `8` of `## 8. Inconsistencies found`:
  - `inv-astra` verified against current `agentbus/main.go:1068-1086` and `prompts/review.md:3-4,29-30` that **11 of the 12 historical items (`1`–`9`, `11`, `12`) are already resolved** (`Server.instructions` dynamically checks `s.tools()`, and `prompts/review.md` already says `"opened by a team of agents or by an investigation"` and `"without the agentbus tools, do not use them"`).
  - **Item `10` (`Local and remote delegation give different rules`) remains partly relevant** (`node_launch_job` sets `KIND=worker` / `REMOTE_JOB=1`, routing remote helpers to `protocol.md` instead of `helper.md`, though `prompts/pr-ownership.md` now forbids PR creation across all modes).
  - **Item `11` (`Polish text in prompts`)** is resolved for review labels (`"Recenzja PR"`), while a separate residual (`## Kontekst` in `start_helper` at `bin/agent:4798`) was noted by `inv-astra`.

---

## 8. PR ownership rule across every mode and continuation

### Integrated solution

- Added `prompts/pr-ownership.md` (rendered via `continuation_rules` in `build_prompt` and watchdog `hang-resume` in `bin/agent`) and added explicit rules in all `7` mode prompt templates (`prompts/protocol.md`, `prompts/team.md`, `prompts/helper.md`, `prompts/investigator.md`, `prompts/inv-work.md`, `prompts/inv-merge.md`, `prompts/review.md`):
  - Never open a new PR if you are not the coordinator or if a PR about the subject is already opened (state `Draft` or `Open`).
  - Always continue in the mentioned PR; never create a new or duplicate PR.
- **Tests:**
  - `dashboard PR ownership and Serena guidance reach every role and continuation` (`tests/unit/dashboard_prompt_modes.bats`, from `inv-astra`)
  - `every mode's prompt states the rule never to open a new PR when not coordinator or when a Draft or Open PR already exists` (`tests/unit/member_pr.bats`, from `inv-gemini-argon`)
