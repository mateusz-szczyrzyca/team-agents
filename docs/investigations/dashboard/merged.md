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
  - Enforced in `resume_run` (before mutating team/task state), `start_runner` (`mode != new`), and `automation_allowed` (blocking background wakes, prechecks, fix passes, and merges while `--investigation` runs).
- **Tests:**
  - `dashboard independent investigation blocks normal resume before mutation`, `dashboard independent investigation blocks message wake`, `dashboard ordinary investigations and other projects do not block resume` (`tests/unit/dashboard_lifecycle.bats`, from `inv-astra`)
  - `resuming a normal team in the same project is refused while --investigation is in progress, but allowed for normal model investigations` (`tests/unit/investigation_team_admission.bats`, from `inv-gemini-argon`)

---

## 4. Background jobs report: inventory, where defined, what they do, and periods

Even when no agent CLI (`claude`, `codex`, `agy`/`gemini`, `opencode`) is actively running, the installation runs **4 standing `systemd --user` services/timers** installed by `install.sh` and **11 transient `systemd-run --user` background unit types** (plus **3 in-process background loops/goroutines**) spawned dynamically by `bin/agent`, `agentbus`, and `agentlive`.

### 4.1 Standing `systemd --user` units (defined in `install.sh`)

| # | Unit | Type | Defined In | Period / Trigger | What It Does |
|---|---|---|---|---|---|
| `1` | `agent-supervise.timer` + `agent-supervise.service` | Timer + oneshot (`agent _supervise`) | `install.sh:164-184`, `bin/agent` (`cmd__supervise`) | Every `3min` (`OnBootSec=3min`, `OnUnitActiveSec=3min`) | Reaps dead runners (`120s` grace), restarts machine-interrupted runs (`RUN_BOOT` mismatch), cleans stale question watches, retries undelivered remote questions, cancels orphaned remote jobs (`supervise_lead_jobs`), warns on abandoned open teams (`AGENT_TEAM_ABANDON_MIN=60m`) and delayed review pairs (`AGENT_REVIEW_WAIT_MIN=30m`), advances independent `--investigation` state machines (`supervise_investigation_teams`), and dispatches queued investigations and review pairs (`dispatch`). |
| `2` | `agentweb.service` | Long-running daemon (`agent web`) | `install.sh:186-197`, `bin/agent` (`cmd_web`), `live/web.go` | Continuous (`Restart=on-failure`, `RestartSec=2s`); SSE poll `1s`; agent cache `30s`; Tailscale whois cache `1m` (`10s` on error) | Serves the web UI (`127.0.0.1:7777`), streams live SSE updates (`/events/dashboard`, `/events/dashboard/full`, `/events/task/{id}`, `/events/bus`), and executes web actions (`kick`, `stop`, `team --stop`, `run --resume`). |
| `3` | `agentlead.service` | Long-running daemon (`agent lead`, when `LEAD_LISTEN` set) | `install.sh:206-225`, `bin/agent` (`cmd_lead`), `agentbus/lead_http.go` | Continuous (`Restart=on-failure`, `RestartSec=2s`); node heartbeat TTL `NODE_TTL_SEC=90s`; member-grace sweep `max(grace/4, 10s)` (`AGENT_RESTART_GRACE_MIN=10m`) | Serves the multi-host HTTP bus API (`agentbus lead`) and marks remote members on stale worker nodes as `absent` after the grace window, returning their board items. |
| `4` | `agentnode.service` | Long-running daemon (`agent node`, when `LEAD_URL` set) | `install.sh:235-254`, `bin/agent` (`cmd_node`) | Continuous (`Restart=on-failure`, `RestartSec=5s`); poll `NODE_POLL_SEC=5s` (or doorbell wake up to `max(NODE_POLL_SEC, 25s)`); auth probe `NODE_AUTH_POLL_SEC=300s`; log push `NODE_LOG_PUSH_SEC=10s` | Sends node heartbeats and capabilities to the lead, launches/cancels assigned remote member and helper tasks, relays remote questions/answers, and pushes logs and artifacts. |

### 4.2 Transient `systemd-run --user` units (defined in `bin/agent` and `agentbus`)

| # | Unit Pattern | Entrypoint | Defined In | Trigger / Period | What It Does |
|---|---|---|---|---|---|
| `5` | `agent-<task>-r<run>.service` | `agent _runner <task> <mode>` | `bin/agent` (`start_runner`, `cmd__runner`) | Per task start, resume, wake, or bounce | Prepares MCP config (`agentbus`, optional `serena`), builds prompt, launches the model CLI with `watch_run`, handles hang auto-resumes, records final status, and triggers post-run hooks (`team_after_run`, `review_followup`, `dispatch`, `dispatch_helpers`). |
| `6` | `agent-<task>-precheck-<epoch>.service` | `agent _precheck <task>` | `bin/agent` (`start_precheck`, `cmd__precheck`, `review_precheck`) | Triggered when a `--verify` / `--autopilot` round finishes (`STATUS: DONE` or `ROUND_DONE`) or Phase 2 merge finishes; polls gate progress every `AGENT_STAGE_POLL_SEC=2s` (`team_wait_settled` polls `5s` up to `120`× = `10m`) | Waits for team members to settle, verifies test-first diff (`round_test_changes`), single open PR (`round_open_prs`), PR formatting (`pr_format_issues`), and runs `make gate` in an isolated detached worktree (timeout `AGENT_REVIEW_GATE_MIN=30m`). |
| `7` | `agent-<task>-fixpass-<epoch>.service` | `agent _fixpass <task>` | `bin/agent` (`review_fix_pass`, `cmd__fixpass`, `do_fix_pass`) | Triggered when a `--verify` or `--autopilot` review returns `needs-fixes` or `incomplete` (`team_wait_settled` polls `5s` up to `10m`) | Waits for team members to settle, then resumes the team (`--same-round`) with the review findings (up to `VERIFY_MAX_FIX_PASSES=2`, or `3` pairs in `--autopilot`). |
| `8` | `agent-<task>-merge-<epoch>.service` | `agent _merge <task>` | `bin/agent` (`review_pair_done`, `cmd__merge`) | Triggered in `--autopilot` when both reviews in a pair return `ready-to-merge` (`team_wait_settled` polls `5s` up to `10m`) | Checks `merge_blockers`, readies draft PR, merges PR (`--match-head-commit`), runs `gate_at` on merged base branch, and if `WORK_ROUND < WORK_ROUNDS`, starts round `N+1` via `agent run --resume <team> --autopilot`. |
| `9` | `agent-wake-<task>-<ns>.service` | `agent _wake <task>` | `agentbus/team.go` (`wakeMember`), `bin/agent` (`cmd__wake`) | On-demand when a bus message with `wake=true` or board assignment targets a `sleeping` team member | Acquires `wake.lock`, checks whether the member is still `sleeping` and its agent is free; if free, starts a resume runner, or sets `WAKE_PENDING=1`. |
| `10` | `agent-consult-<team>-<epoch>.timer` + `.service` | `agent _wake <lead-task>` | `bin/agent` (`start_consult_timer`, `stop_consult_timer`) | One-shot timer after `AGENT_CONSULT_MIN=5m` (`--on-active`) when a team coordinator enters `STATUS: CONSULTING` | Wakes the sleeping coordinator if not all members have posted their `consultation_reply` within the deadline. |
| `11` | `agent-<task>-pause-<run>.timer` + `.service` | `agent _unpause <task> <run>` | `bin/agent` (`pause_task`, `cmd__unpause`) | One-shot timer (`--on-active=<secs>s`): network `15m`, rate-limit `5m`, overloaded `10m`, repeated empty wakes `30m`, or provider quota reset timestamp | Releases a paused task and resumes its run (or marks a sleeping team member ready for wakes). |
| `12` | `agent-<task>-answers-<round>.path` + `.service` | `agent _answers_saved <task> <round>` | `bin/agent` (`watch_answers`, `cmd__answers_saved`) | `systemd` `PathChanged=` watch on `questions-<round>.md` while `STATUS=waiting` | Validates human answers when `**ANSWERED**` is saved in `questions-<round>.md` and automatically runs `agent resume <task>`. |
| `13` | `agent-invteam-<id>-<epoch>.service` | `agent _investigation-controller --task <id>` | `bin/agent` (`start_investigation_controller`, `cmd__investigation_controller`) | Detached controller unit for `--investigation` | Drives the 6-phase independent investigation pipeline (`Phase 1` cohort → `Phase 2` merge → precheck → `Phase 3` review pair → optional `Phase 4` fix → precheck → `Phase 5` re-review → `Phase 6` handoff). |
| `14` | `agent-rmember-<task>-<epoch>.service` | `agent _remote_member_done <task> <state>` | `bin/agent` (`cmd_lead`, `cmd__remote_member_done`) | Spawned by `cmd_lead` when `agentbus lead` emits `MEMBER_JOB\t<task>\t<state>` on stdout | Handles terminal transitions of remote team members on the lead (re-queuing unfinished board items on failure/stop and waking the coordinator if all members finished). |
| `15` | `agent-rquestion-<job>-<round>-<epoch>.service` | `agent _remote_question <job> <round>` | `bin/agent` (`cmd_lead`, `cmd__remote_question`) | Spawned by `cmd_lead` when `agentbus lead` emits `QUESTION\t<job>\t<round>` on stdout | Materializes a worker node's question file on the lead, opens it in VS Code, starts a `PathChanged` watch, and sends a push notification. |

### 4.3 In-process background loops inside running commands

- **`watch_run` (`bin/agent`):** Polls every `10s` inside each `agent _runner` while the model CLI runs; enforces `AGENT_HANG_MIN` (`20m` inactivity kill), `AGENT_WARN_MIN` (`30m` runtime notification), and `AGENT_LONG_RUN_SOFT` / `AGENT_LONG_RUN_HARD` (`120` / `160` steps).
- **`gate` progress poller (`bin/agent`, `review_precheck`):** Polls `precheck.log` every `AGENT_STAGE_POLL_SEC=2s` while `make gate` runs in `agent _precheck`, updating `AUTO_STAGE` for the dashboard.
- **`agent live` TUI ticker (`live/main.go`):** Refreshes the terminal UI every `2s` and emits periodic waiting/stopped reminders every `--remind` minutes (`30m` default).

### 4.4 Why activity happens when no agent CLI is running

1. **Between rounds or before reviews (`_precheck`, `_merge`):** After all team members finish (`STATUS=done`/`sleeping`), `agent _precheck` or `agent _merge` waits for members to settle (`team_wait_settled`) and runs `make gate` in a temporary detached worktree for several minutes with zero model CLIs active.
2. **Between review completion and fix pass (`_fixpass`):** After reviewer tasks finish, `agent _fixpass` waits for any late member runs to settle before invoking `agent run --resume` for the next fix pass.
3. **Every 3 minutes (`agent-supervise.timer`):** `agent _supervise` runs even when the system is idle, reaping dead units, checking abandoned teams/review waits, advancing `--investigation` phases, and dispatching queued work.
4. **Pause/consultation timers and file watches:** `agent-<task>-pause-<run>.timer`, `agent-consult-<team>.timer`, and `agent-<task>-answers-<round>.path` fire asynchronously on schedule or file save and launch new runners.

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
