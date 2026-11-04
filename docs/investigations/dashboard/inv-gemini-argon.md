# Investigation Report — Dashboard (`20261104-160248-inv-gemini-argon`)

- Investigator: `inv-gemini-argon` (`gemini/gemini-4-argon-high`)
- Task: `20261104-160248-inv-gemini-argon`
- Branch: `agents/inv-gemini-argon/20261104-160248`
- Base commit: [`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f)

---

## 1. Dashboard 20-entry limit & `"Full dashboard view"`

### Findings & implementation

- Previously, [`live/web_views.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/live/web_views.go#L413-L458) set `dashShown = 80` and rendered up to `80` task rows on the main dashboard (`GET /`), with an inline `?all=1` toggle rather than a separate full-list view in the navigation menu.
- In [`live/web_views.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8f9b49c9f5f25f464ed0331406de7337ddc25b8a/live/web_views.go#L429-L486) and [`live/web.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8f9b49c9f5f25f464ed0331406de7337ddc25b8a/live/web.go#L67-L75):
  - Changed `dashShown` from `80` to `20` so `GET /` displays at most the `20` latest task entries by default.
  - Added `GET /dashboard/full` (`viewDashboardFull`) and [`live/web/templates/dashboard_full.html`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8f9b49c9f5f25f464ed0331406de7337ddc25b8a/live/web/templates/dashboard_full.html) as a dedicated page rendering the complete task list (`SSE` stream `/events/dashboard?all=1`).
  - Added `"Full dashboard view"` (`/dashboard/full`) to the top navigation bar in [`live/web/templates/layout.html`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8f9b49c9f5f25f464ed0331406de7337ddc25b8a/live/web/templates/layout.html#L20) and linked `"Full dashboard view"` from [`live/web/templates/partials.html`](https://github.com/mateusz-szczyrzyca/team-agents/blob/8f9b49c9f5f25f464ed0331406de7337ddc25b8a/live/web/templates/partials.html#L106).
- Commits:
  - $\color{red}{\textbf{RED}}$: [`0c73736`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0c73736c1db5d62e71f4786ffb3df3e589f482d8) — `TestDashboardShowsAtMost20LatestEntriesAndFullDashboardViewShowsAll` in [`live/web_test.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0c73736c1db5d62e71f4786ffb3df3e589f482d8/live/web_test.go)
  - $\color{green}{\textbf{GREEN}}$: [`8f9b49c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8f9b49c9f5f25f464ed0331406de7337ddc25b8a) — `Limit default dashboard to 20 entries and add Full dashboard view (green)`
  - Refactor: [`098d173`](https://github.com/mateusz-szczyrzyca/team-agents/commit/098d173754d3868874f3211159336ec361f68b67) — `Remove unused dashTasks helper and allow reviews when autopilot is off (refactor)`

---

## 2. `/teams` `"Stop this team"` option & preventing automatic resume

### Findings & implementation

- Previously, `/teams` ([`live/web/templates/teams.html`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/live/web/templates/teams.html)) had no action button to stop a team, `/team/{id}` only had `"Close team"` (`POST /team/{id}/close`), and `team_close` in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L7917-L7935) did not stop active/scheduled timers or prevent automatic resume hooks (`_wake`, `_precheck`, `_fixpass`, `_merge`) from restarting the coordinator or members.
- In [`live/web.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/22e163a8dd612999fcb89e7da4bbf39563fce1fe/live/web.go#L90-L98), [`live/web_views.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/22e163a8dd612999fcb89e7da4bbf39563fce1fe/live/web_views.go#L351-L364), [`live/web/templates/teams.html`](https://github.com/mateusz-szczyrzyca/team-agents/blob/22e163a8dd612999fcb89e7da4bbf39563fce1fe/live/web/templates/teams.html#L20-L32), [`live/web/templates/team.html`](https://github.com/mateusz-szczyrzyca/team-agents/blob/22e163a8dd612999fcb89e7da4bbf39563fce1fe/live/web/templates/team.html#L14-L28), and [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/22e163a8dd612999fcb89e7da4bbf39563fce1fe/bin/agent#L7931-L7967):
  - Added `"Stop this team"` (`POST /team/{id}/stop` → `agent team --stop <id>`) to both `/teams` and `/team/{id}`, and exposed `TeamSummary.Stopped` (`isTeamStopped`).
  - Updated `cmd_team` (`--stop` / `--close`) and `team_close` in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/22e163a8dd612999fcb89e7da4bbf39563fce1fe/bin/agent#L7931-L7967) to persist `TEAM_STOPPED=1`, `AUTOPILOT_OFF=1`, and `VERIFY=0` in the coordinator and member `meta.env` files, stop any sleeping member tasks (`STATUS=stopped`, `ENDED_AT`), mark every member as `stopped` on the bus (`bus team-set "$team" "$m" stopped`), and stop all running/timer units (`stop_task "$tid"`).
  - Updated `cmd__wake`, `cmd__precheck`, `cmd__fixpass`, `do_fix_pass`, and `cmd__merge` in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/22e163a8dd612999fcb89e7da4bbf39563fce1fe/bin/agent#L1869-L2952) to refuse automatic resume when `TEAM_STOPPED=1` or when `team_done_flag "$team"` is `true`.
- Commits:
  - $\color{red}{\textbf{RED}}$: [`c6fadd7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c6fadd78a765b6e522b93d3549edb83b366dd187) — `TestTeamsViewHasStopThisTeamOption` in [`live/web_test.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c6fadd78a765b6e522b93d3549edb83b366dd187/live/web_test.go) and `team --stop stops the team and blocks automatic resume` in [`tests/unit/team_close.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c6fadd78a765b6e522b93d3549edb83b366dd187/tests/unit/team_close.bats)
  - $\color{green}{\textbf{GREEN}}$: [`22e163a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/22e163a8dd612999fcb89e7da4bbf39563fce1fe) — `Add Stop this team option on /teams and disable automatic resume (green)`
  - Refactor: [`098d173`](https://github.com/mateusz-szczyrzyca/team-agents/commit/098d173754d3868874f3211159336ec361f68b67) — `Remove unused dashTasks helper and allow reviews when autopilot is off (refactor)`

---

## 3. Priority of `--investigation` over resuming normal teams in the same project

### Findings & implementation

- Previously, `new_run` in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L5495-L5507) blocked starting a *new* `--team` when an `--investigation` was active, queued, or waiting on the host (`active_investigations`), and queued `--investigation` runs when a team was active, **but `resume_run` had no check at all**. Resuming a normal team (`agent run --resume <team-or-member>`) while an `--investigation` was running in the same repository was allowed and could collide with investigators running tests in that repository.
- In [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/2d43908a9f3ecb71e72ca9d8fed4b9161c4f72f9/bin/agent#L5490-L5518) and [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/2d43908a9f3ecb71e72ca9d8fed4b9161c4f72f9/bin/agent#L5836-L5844):
  - Added `active_investigation_in_repo <repo_root>`, which scans `TASKS_DIR/*/meta.env` for top-level `--investigation` tasks (`INVESTIGATION=1`, empty `INV_PARENT`, `REPO_ROOT=<repo_root>`) in `running`, `starting`, `queued`, or `waiting` state, or with any investigator (`INV_PARENT=<inv_id>`) in `running`, `starting`, `queued`, `waiting`, or `sleeping` state, or with a running phase-2 merger (`INV_MERGE_TASK`).
  - Note: investigations requested by normal models via the MCP `request_investigation` tool (`bus inv-start` / `spawn_investigator`) create tasks with `INVESTIGATOR_OF=<id>` and `INVESTIGATION=0`, so they are unaffected and continue to work normally as part of a team's job.
  - Updated `resume_run` to check `active_investigation_in_repo "$REPO_ROOT"` whenever a normal team coordinator or team member (`TEAM=1` or non-empty `TEAM_ID`, and `INVESTIGATION!=1` and empty `INV_PARENT`) is being resumed, failing with a clear message telling the caller which `--investigation` is in progress.
- Commits:
  - $\color{red}{\textbf{RED}}$: [`782ea38`](https://github.com/mateusz-szczyrzyca/team-agents/commit/782ea38df13088c23c8bc4bba8007f6511cc824d) — `Test refusing normal team resume during --investigation in same project (red)` in [`tests/unit/investigation_team_admission.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/782ea38df13088c23c8bc4bba8007f6511cc824d/tests/unit/investigation_team_admission.bats)
  - $\color{green}{\textbf{GREEN}}$: [`2d43908`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2d43908a9f3ecb71e72ca9d8fed4b9161c4f72f9) — `Refuse normal team resume while --investigation runs in same project (green)`

---

## 4. Background jobs report: inventory, where defined, what they do, and periods

Even when no agent CLI (`claude`, `codex`, `gemini`, `opencode`) is actively running, the installation runs **12 distinct background job types**: **4 standing `systemd --user` services/timers** installed by [`install.sh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/install.sh#L163-L260) and **8 transient `systemd-run --user` background units** spawned dynamically by [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent) and [`agentbus`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/agentbus/team.go).

### Summary table of all 12 background jobs

| # | Unit / Job Name | Kind | Defined In | Period / Trigger | What It Does |
|---|---|---|---|---|---|
| `1` | `agent-supervise.timer` & `agent-supervise.service` | Standing timer + oneshot service | [`install.sh#L164-L184`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/install.sh#L164-L184), [`bin/agent#L6545-L6555`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L6545-L6555) (`cmd__supervise`) | Every `3min` (`OnBootSec=3min`, `OnUnitActiveSec=3min`) | Reaps dead runners (`120s` grace), restarts machine-interrupted runs, cleans question watches, retries remote questions, cancels orphaned remote jobs, warns on abandoned teams (`60m`) and delayed review pairs (`30m`), and dispatches queued `--investigation` runs and rotated reviews (`dispatch`). |
| `2` | `agentweb.service` | Standing daemon (`Restart=on-failure`, `RestartSec=2`) | [`install.sh#L186-L197`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/install.sh#L186-L197), [`bin/agent#L7068-L7075`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L7068-L7075), [`live/web.go#L134`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/live/web.go#L134) | Continuous; SSE poll every `1s`; agent cache `30s`; Tailscale whois cache `1m` (`10s` on error) | Serves the web dashboard, pushes live SSE updates (`/events/dashboard`, `/events/task/{id}`, `/events/bus`), and executes web actions (`kick`, `stop`, `team --stop`, `run`). |
| `3` | `agentlead.service` (multi-host lead only) | Standing daemon (`Restart=on-failure`, `RestartSec=2`) | [`install.sh#L206-L225`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/install.sh#L206-L225), [`bin/agent#L7078-L7092`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L7078-L7092), [`agentbus/lead_http.go#L59-L83`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/agentbus/lead_http.go#L59-L83) | Continuous; node heartbeat TTL `NODE_TTL_SEC=90s`; absent member check `--member-grace` (`AGENT_RESTART_GRACE_MIN=10m`, min poll `10s`) | Serves the multi-host HTTP bus API (`agentbus lead`) and marks remote members on stale nodes as `absent` after the grace window. |
| `4` | `agentnode.service` (multi-host worker node only) | Standing daemon (`Restart=on-failure`, `RestartSec=5`) | [`install.sh#L235-L254`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/install.sh#L235-L254), [`bin/agent#L8445-L8533`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L8445-L8533) (`cmd_node`) | Loop every `NODE_POLL_SEC=5s` (or immediate wake via `node-doorbell` long-poll); auth refresh every `NODE_AUTH_POLL_SEC=300s` (`5m`); log push every `NODE_LOG_PUSH_SEC=10s` | Registers node capabilities with the lead, starts/stops assigned remote member and helper tasks, pushes logs and artifacts, and syncs worktree states. |
| `5` | `agent-<id>-r<run>.service` | Transient unit (`systemd-run --user`) | [`bin/agent#L5409-L5421`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L5409-L5421) (`start_runner` → `cmd__runner`), [`bin/agent#L3234-L3282`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L3234-L3282) (`watch_run`) | Per task run (`new` or `resume`); in-process watchdog polls every `AGENT_HANG_POLL=15s` | Builds the prompt, launches the agent CLI with `agentbus serve`, normalizes output into `activity.jsonl`, kills hung runs after `AGENT_HANG_MIN=30m` of silence or `AGENT_LONG_RUN_HARD=200` tool calls, and triggers post-run hooks (`finish_team`, `finish_investigation_run`, `finish_review_run`, `start_precheck`, `schedule_wake`, `dispatch`). |
| `6` | `agent-<id>-precheck-<ts>.service` (and phase 2 `agent-<parent>-precheck-<n>.service`) | Transient unit (`systemd-run --user`) | [`bin/agent#L1846-L2015`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L1846-L2015) (`start_precheck`, `cmd__precheck`), [`bin/agent#L4238-L4247`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L4238-L4247) | Triggered when a `--verify` / `--autopilot` task or coordinator (or phase-2 merger) finishes with `STATUS: DONE` | **Why activity happens when no agent is running:** Waits up to `90s` (`18 × 5s`) for team members to settle (`team_wait_settled`), runs the repository gate (`make gate`, polling progress every `AGENT_STAGE_POLL_SEC=2s` up to `AGENT_REVIEW_GATE_MIN=30m`), checks round test rules and PR state, and either bounces the task back (`agent run --resume ... --same-round`, up to `AGENT_PRECHECK_MAX=2` times) or queues the automated `2`-model PR review (`request_review` + `dispatch`). |
| `7` | `agent-<id>-fixpass-<ts>.service` | Transient unit (`systemd-run --user`) | [`bin/agent#L1619-L1623`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L1619-L1623) (`start_task_unit`), [`bin/agent#L2021-L2067`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L2021-L2067) (`cmd__fixpass`, `do_fix_pass`) | Triggered after both review runs complete (`finish_review_run`) when findings are not `ready-to-merge` | Waits up to `90s` (`18 × 5s`) for team members to settle, appends the reviewers' findings to `fix-prompt.md`, and resumes the coordinator/task in the same round (`agent run --resume <ref> --same-round`, up to `VERIFY_MAX_FIX_PASSES=2` passes). |
| `8` | `agent-<id>-merge-<ts>.service` | Transient unit (`systemd-run --user`) | [`bin/agent#L1619-L1623`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L1619-L1623) (`start_task_unit`), [`bin/agent#L2206-L2341`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L2206-L2341) (`cmd__merge`) | Triggered in `--autopilot` mode when both reviewers return `VERDICT: ready-to-merge` | **Why git merges, gates, and new rounds start with no agent running:** Checks `merge_blockers` (human-held paths, modified existing tests), marks draft PRs ready (`gh pr ready`), merges the PR (`gh pr merge --merge --match-head-commit`), runs the post-merge gate (`run_review_gate`, up to `30m`), and if `WORK_ROUND < WORK_ROUNDS`, automatically starts the next round (`agent run --resume <ref>`). |
| `9` | `agent-<id>-wake-<ts>.timer` & `.service` | Transient timer (`systemd-run --user --on-active=...`) + direct bus hook | [`bin/agent#L2856-L2952`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L2856-L2952) (`schedule_wake`, `cmd__wake`), [`agentbus/team.go#L93`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/agentbus/team.go#L93) (`wakeFn`) | Timer delays: `5s` (long-run cut at `AGENT_LONG_RUN_SOFT=120`), `5m` (`AGENT_RATE_LIMIT_PAUSE_MIN`, `AGENT_OVERLOAD_PAUSE_MIN`, `AGENT_NETWORK_PAUSE_MIN`), `15m` (`AGENT_WAKE_LOOP_PAUSE_MIN`), `30m` or provider reset timestamp (`AGENT_QUOTA_FALLBACK_MIN`); or immediate via `agentbus` when a message/board task arrives | Wakes a `sleeping` team member or rate-limited/quota-paused task by calling `agent _wake <id>`, which verifies wake conditions (unread bus messages, open board items, or `WAKE_REASON` timer expiry) and runs `agent run --resume <id>`. |
| `10` | `agent-<id>-answers-<round>.path` & `.service` | Transient systemd path unit (`--path-property=PathModified=...`) | [`bin/agent#L959-L1035`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L959-L1035) (`watch_answers`, `cmd__answers_saved`) | Triggered immediately on filesystem write to `~/agents/tasks/<id>/questions-<round>.md` | Checks if the human added `**ANSWERED**` (and filled every `**Verdict:**`), stops the path unit, and automatically resumes the waiting task (`agent run --resume <id> --same-round`). |
| `11` | `agent-remote-questions-<task>@<node>.service` | Transient unit (`systemd-run --user`) | [`bin/agent#L878-L917`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L878-L917) (`cmd__remote_questions`) | Triggered when `agentlead` receives a question file from a remote worker node (`NODE_QUESTION_CMD`) | Writes the mirrored question proxy `meta.env` on the lead host, starts `watch_answers`, opens the questions file in VS Code (`open_questions_editor`), and sends desktop/email notifications. |
| `12` | `agent-auth-<name>-<pid>.service` | Transient unit (`systemd-run --user`) | [`bin/agent#L8666-L8708`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L8666-L8708) (`cmd_authenticated`, `auth_providers`, `cmd__authcheck`) | On demand (`agent authenticated`) and every `300s` (`NODE_AUTH_POLL_SEC`) on worker nodes; timeout `180s` (`360 × 0.5s`) | Probes whether each configured provider CLI (`claude`, `codex`, `gemini`, `opencode`) is authenticated inside a clean user systemd unit. |

### Why activity happens when no agent appears to be running

1. **Between coordinator completion and PR review (`agent-<id>-precheck-<ts>`):** After the coordinator exits with `STATUS: DONE`, no agent CLI is running, but `_precheck` waits up to `90s` for members to settle and then executes `make gate` inside the task worktree (up to `30m`), followed by spawning the two reviewer tasks (`request_review` + `dispatch`).
2. **Between review completion and the next fix pass (`agent-<id>-fixpass-<ts>`):** After the second reviewer finishes, `_fixpass` waits up to `90s` for team settlement, constructs `fix-prompt.md`, and launches `agent run --resume <id> --same-round`.
3. **Between review approval, PR merge, and the next round (`agent-<id>-merge-<ts>`):** After both reviewers output `VERDICT: ready-to-merge`, `_merge` runs `gh pr ready`, `gh pr merge`, checks out the base branch, runs `make gate` on the merged commit (up to `30m`), and then starts Round `N+1` via `agent run --resume <id>`.
4. **Phase 1 → Phase 2 `--investigation` transition (`finish_investigation_run`):** When the last Phase 1 investigator finishes (`bin/agent:4110-4252`), `_runner` spawns the Phase 2 merger task (`agent run --inv-merge <parent>`), which when finished triggers `agent-<parent>-precheck-<n>` to run the gate and open the final PR.
5. **Every `3` minutes (`agent-supervise.timer`):** `cmd__supervise` runs `dispatch`, which starts queued `--investigation` cohorts once running teams finish, or starts the second reviewer of a rotated review pair once the first reviewer finishes.

---

## 5. Stopped by demand vs `"sleeping"` (`20261104-122828-team` / `haiku`)

### Root cause in `20261104-122828-team`

Inspection of `/home/dev/agents/tasks/20261104-122828-claude-haiku/meta.env`, `/home/dev/agents/bus/state.json`, and `/home/dev/agents/logs/agent.jsonl` revealed three compounding bugs:

1. Coordinator `gemini-1` (`20261104-122828-team`) finished at `12:38:24` (`team.done = true` in `bus/state.json`). At that moment `claude-haiku` (`MODEL=haiku`) was still `running`, so `finish_team` (`bin/agent:4030`) skipped it (`if [[ $(meta_get "$mf" STATUS) == "sleeping" ]]`).
2. At `12:46:13`, the user clicked `Kick` in the web UI (`agent kick 20261104-122828-claude-haiku web`). In [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L7289-L7299), `cmd_kick` checked `if [[ -n $team ]]` without checking whether the team was already done (`team_done_flag "$team" == true`), so it set `STATUS=sleeping` in `meta.env` and `state="sleeping"` in `bus/state.json` on an already-closed team.
3. At `13:01:29`, `agent stop --running` (`stop_all` in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L7202-L7245)) stopped `20261104-122828-claude-haiku` and wrote `STATUS=stopped` to `meta.env`, **but never updated the bus member state** (`bus team-set`), leaving `state: "sleeping"` in `bus/state.json`. Furthermore, `agentbus` had no `MemberStopped = "stopped"` enum value (`MemberState` only had `starting`, `working`, `sleeping`, `done`, `failed`, `absent`), and `live/web_views.go` rendered `m.State` (`"sleeping"`) directly without checking whether the task's `STATUS` was `"stopped"`.

### Fix implemented

- Added `MemberStopped MemberState = "stopped"` to [`agentbus/types.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/e3cf60f1c5a54e22d8a98253c10f3850b5510dee/agentbus/types.go#L76-L98), prevented `wakeSleeping` (`agentbus/team.go`) from auto-waking a member whose state is `MemberStopped`, and treated `MemberStopped` as a terminal state in `superviseRemoteMembers` (`agentbus/remote_member.go`).
- Updated `cmd_stop`, `stop_all`, `finish_team`, `team_after_run`, `cmd__wake`, and `cmd_kick` in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/e3cf60f1c5a54e22d8a98253c10f3850b5510dee/bin/agent) so stopping a member on demand (or kicking a member of a finished/stopped team) sets `STATUS=stopped` in `meta.env` and `stopped` in `bus/state.json`.
- Added `effectiveMemberState` in [`live/web_views.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/e3cf60f1c5a54e22d8a98253c10f3850b5510dee/live/web_views.go#L412-L426) so the web UI displays `"stopped"` (never `"sleeping"`) both for new tasks and for legacy tasks on disk where `meta.env` has `STATUS=stopped` while `bus/state.json` still had `state="sleeping"`.
- Commits:
  - $\color{red}{\textbf{RED}}$: [`b561eb4`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b561eb430a1efbd9d48c867166eca803e0f312db) — `TestMemberStoppedState` (`agentbus`), `TestStoppedTeamMemberIsNotShownAsSleeping` (`live`), and `tests/unit/kick.bats`
  - $\color{green}{\textbf{GREEN}}$: [`e3cf60f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e3cf60f1c5a54e22d8a98253c10f3850b5510dee) — `Record and display stopped team members as stopped, not sleeping (green)`

---

## 6. Serena MCP server (`https://github.com/oraios/serena`) across all models

### Findings on the instance & codebase

- `serena` is installed on the host at `/home/dev/.local/bin/serena` (Serena `0.1.4`, with `serena-mcp-server` in the same uv tool environment), and `/home/dev/team-agents/.serena/project.yml` is configured for languages `go`, `bash`, and `typescript`.
- Three issues prevented models from reliably using Serena in runs:
  1. [`config/agents.conf`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/config/agents.conf#L139) defaulted to `AGENT_SERENA=""` (disabled unless manually overridden), and [`install.sh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/install.sh) did not enable `AGENT_SERENA="serena"` in `~/agents/agents.conf` after installing `serena`.
  2. Every task runs inside an isolated git worktree (`~/agents/worktrees/<repo>/<task>/`), where `.serena` is git-excluded (`worktree_exclude`), so `.serena/project.yml` from `REPO_ROOT` was missing in the task worktree. Without `.serena/project.yml` in `--project "$WORK_DIR"`, Serena had to auto-generate a single-language project config on first start instead of using the repository's configured multi-language (`go`, `bash`, `typescript`) settings.
  3. [`prompts/serena.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/prompts/serena.md) only mentioned `serena_repl` and did not mention calling `initial_instructions` when `serena_repl` is activated or using Serena's symbol tools (`get_symbols_overview`, `find_symbol`, `find_referencing_symbols`) when registered directly.

### How each model CLI receives and uses Serena

| CLI (`CLI_TYPE`) | Where Configured in `bin/agent` | Context & Mode Passed to `serena start-mcp-server` | How the Model Uses Serena |
|---|---|---|---|
| `claude` | [`run_claude` (`bin/agent#L3515-L3562`)](https://github.com/mateusz-szczyrzyca/team-agents/blob/ee4864db5c3036ab947aa757c8561b2a206bf31b/bin/agent#L3515-L3562) | `--context claude-code --project "$WORK_DIR" --mode interactive --mode editing --mode one-shot --open-web-dashboard false --enable-gui-log-window false` (plus `serena-repl` when `serena_has_mode serena-repl` is true) | Via `--mcp-config <task>/mcp-claude.json` (`mcpServers.serena`). |
| `codex` | [`run_codex` (`bin/agent#L3678-L3705`)](https://github.com/mateusz-szczyrzyca/team-agents/blob/ee4864db5c3036ab947aa757c8561b2a206bf31b/bin/agent#L3678-L3705) | `--context codex --project "$WORK_DIR" ...` | Via `-c mcp_servers.serena.command=...` and `-c mcp_servers.serena.args=[...]`. |
| `gemini` | [`run_gemini` (`bin/agent#L3797-L3834`)](https://github.com/mateusz-szczyrzyca/team-agents/blob/ee4864db5c3036ab947aa757c8561b2a206bf31b/bin/agent#L3797-L3834) | `--context agent --project "$WORK_DIR" ...` | Via `--mcp-config <task>/mcp-gemini.json` (`mcpServers.serena`). |
| `opencode` | [`run_opencode` (`bin/agent#L3885-L3937`)](https://github.com/mateusz-szczyrzyca/team-agents/blob/ee4864db5c3036ab947aa757c8561b2a206bf31b/bin/agent#L3885-L3937) | `--context agent --project "$WORK_DIR" ...` | Via `OPENCODE_CONFIG_CONTENT` (`mcp.serena.type="local"`, `mcp.serena.command=[...]`, `mcp.serena.enabled=true`). |

### Changes & tests

- Set `AGENT_SERENA="${AGENT_SERENA:-serena}"` in [`config/agents.conf`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ee4864db5c3036ab947aa757c8561b2a206bf31b/config/agents.conf#L139) and updated [`install.sh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ee4864db5c3036ab947aa757c8561b2a206bf31b/install.sh#L151-L154) to auto-enable `AGENT_SERENA="serena"` in `~/agents/agents.conf` when `serena` is installed.
- Updated `worktree_exclude` in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ee4864db5c3036ab947aa757c8561b2a206bf31b/bin/agent#L4462-L4480) to copy `$REPO_ROOT/.serena/project.yml` into `$WORK_DIR/.serena/project.yml` whenever a worktree is created.
- Updated [`prompts/serena.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ee4864db5c3036ab947aa757c8561b2a206bf31b/prompts/serena.md) to instruct models to call `initial_instructions` first when using `serena_repl`, and to use `get_symbols_overview`, `find_symbol`, and `find_referencing_symbols` when Serena symbol tools are registered directly.
- Added `4` end-to-end unit tests in [`tests/unit/serena.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/ee4864db5c3036ab947aa757c8561b2a206bf31b/tests/unit/serena.bats#L135-L249) verifying default config, `.serena/project.yml` worktree seeding, prompt instructions, and that all four model CLIs (`claude`, `codex`, `gemini`, `opencode`) invoke Serena MCP tools and normalize the tool call in `activity.jsonl`.
- Commits:
  - $\color{red}{\textbf{RED}}$: [`dceced2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/dceced26c7d937062e33e7036bff38ac7182e8b7) — `Test Serena default config, worktree project.yml seeding, and CLI usage (red)`
  - $\color{green}{\textbf{GREEN}}$: [`ee4864d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ee4864db5c3036ab947aa757c8561b2a206bf31b) — `Enable Serena by default, seed worktree project.yml, and clarify prompt (green)`

---

## 7. Verification of `docs/investigations/i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md`

### Verification of `## 7. Summary matrix` and `## 8. Inconsistencies found`

We audited every row of `## 7. Summary matrix` and all `12` items in `## 8. Inconsistencies found` in [`docs/investigations/i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/docs/investigations/i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md#L483-L619) against current `main` (and updated the file in place):

- **Fixed on `main` (`6` of `12` items):**
  - **Item 1 (Lead-crash failover omitted from `team.md`):** **FIXED** — [`prompts/team.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/prompts/team.md#L43-L45) now includes the `team_Elect_lead` failover instruction (`If the coordinator crashes (a system notice says so), call team_elect_lead...`).
  - **Item 2 (`team_check` missing from `team.md` MCP tools table):** **FIXED** — [`prompts/team.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/prompts/team.md#L106) now lists `team_check | Run the repository check command (shared; waits if already running)`.
  - **Item 3 (`request_investigation` and `notebook_append` missing from `team.md` table):** **FIXED** — [`prompts/team.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/prompts/team.md#L112-L115) now lists `request_investigation`, `checkpoint_save`, `checkpoint_load`, and `notebook_append`.
  - **Item 5 (`inv-merge.md` lacked `{{ROUND_INSTRUCTIONS}}`):** **FIXED** — [`prompts/inv-merge.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/prompts/inv-merge.md#L168) now includes `{{ROUND_INSTRUCTIONS}}`.
  - **Item 6 (Watchdog `AGENT_LONG_RUN_HARD` warning text in `bin/agent`):** **FIXED** — [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L3264-L3272) now checks `if ((MEMBER || TEAM || INVESTIGATOR || INV_WORK || INV_MERGE))` and sends the checkpoint/sleep message to those modes, and a `STATUS: PARTIAL` message to solo/helper/reviewer modes.
  - **Item 7 (`inv-work.md` and `inv-merge.md` omitted from `AGENTS.md` prompt list):** **PARTIALLY RELEVANT** — `AGENTS.md` ("Prompts (`prompts/*.md`)") still lists `protocol.md`, `team.md`, `investigator.md`, `helper.md` and `review.md` (`AGENTS.md` is read-only per repository rules; noted for the human), while [`prompts/inv-work.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/3d99ef5be777c5e3d185df6c88de611c549b8c2f/prompts/inv-work.md) and [`prompts/inv-merge.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/3d99ef5be777c5e3d185df6c88de611c549b8c2f/prompts/inv-merge.md) are now kept in sync with the other prompts (including `{{ROUND_INSTRUCTIONS}}` and the new PR rule).
- **Still relevant on `main` (`6` of `12` items, with current line numbers updated in `merged.md`):**
  - **Item 4 (`delegate` vs `team_add` guidance for members):** Still relevant in [`prompts/team.md#L84-L98`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/prompts/team.md#L84-L98) vs [`agentbus/delegate.go#L114-L162`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/agentbus/delegate.go#L114-L162).
  - **Item 8 (Orphaned `AGENTS.md` files inside `tests/unit/fixtures/repos/*/AGENTS.md`):** Still relevant (`build_prompt` at [`bin/agent#L5332`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L5332) still instructs agents to read every `AGENTS.md` in directories they work in).
  - **Item 9 (MCP tool visibility vs prompt documentation):** Still relevant (`agentbus/mcp.go` registers all `32` tools unconditionally for every `agentbus serve` instance except `--no-bus` runs).
  - **Item 10 (`STATUS: SLEEPING` in `prompts/team.md` vs `team_after_run` in `bin/agent`):** Still relevant ([`bin/agent#L3978-L3989`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L3978-L3989) converts member `DONE` to `sleeping` while the team is open).
  - **Item 11 (`prompts/helper.md` has no `{{WRITING_INSTRUCTIONS}}`):** Still relevant ([`prompts/helper.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/prompts/helper.md)).
  - **Item 12 (`prompts/review.md` does not forbid modifying code or running `git commit` / `git push`):** Still relevant ([`prompts/review.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/prompts/review.md) now explicitly forbids opening a new PR, and still relies on prompt instructions rather than OS/git read-only enforcement).
- Updated `## 7. Summary matrix` in `merged.md` to reflect:
  - The new PR rule across all `7` mode prompts (`protocol.md`, `team.md`, `helper.md`, `investigator.md`, `inv-work.md`, `inv-merge.md`, `review.md`).
  - `{{ROUND_INSTRUCTIONS}}` now present in `inv-merge.md`.
  - Lead-crash failover (`team_elect_lead`), `team_check`, `request_investigation`, `checkpoint_*`, and `notebook_append` now documented in `team.md`.
  - `AGENT_SERENA` now defaulting to `"serena"` with `.serena/project.yml` seeded into every task worktree.

---

## 8. New PR rule across all mode prompts

### Implementation

Added the explicit rule — *"Never open a new PR if you are not the coordinator or if a PR about the subject is already opened (state `Draft` or `Open`). Always continue in the mentioned PR, never create a new PR."* — to all `7` mode prompt templates:

- [`prompts/protocol.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/3d99ef5be777c5e3d185df6c88de611c549b8c2f/prompts/protocol.md#L79-L80) (Solo & Coordinator)
- [`prompts/team.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/3d99ef5be777c5e3d185df6c88de611c549b8c2f/prompts/team.md#L121-L122) (Team Member)
- [`prompts/helper.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/3d99ef5be777c5e3d185df6c88de611c549b8c2f/prompts/helper.md#L33-L34) (Delegated Helper)
- [`prompts/investigator.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/3d99ef5be777c5e3d185df6c88de611c549b8c2f/prompts/investigator.md#L44-L45) (Sub-investigator)
- [`prompts/inv-work.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/3d99ef5be777c5e3d185df6c88de611c549b8c2f/prompts/inv-work.md#L39-L41) (Phase 1 Investigator)
- [`prompts/inv-merge.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/3d99ef5be777c5e3d185df6c88de611c549b8c2f/prompts/inv-merge.md#L74-L76) (Phase 2 Merger)
- [`prompts/review.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/3d99ef5be777c5e3d185df6c88de611c549b8c2f/prompts/review.md#L41-L42) (PR Reviewer)

Commits:
- $\color{red}{\textbf{RED}}$: [`c41deab`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c41deabf58dca605c15d01c7b9d22207a2d5ef2e) — `Test PR rule across all mode prompts (red)` in [`tests/unit/member_pr.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c41deabf58dca605c15d01c7b9d22207a2d5ef2e/tests/unit/member_pr.bats) and [`tests/unit/one_pr_per_round.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c41deabf58dca605c15d01c7b9d22207a2d5ef2e/tests/unit/one_pr_per_round.bats)
- $\color{green}{\textbf{GREEN}}$: [`3d99ef5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3d99ef5be777c5e3d185df6c88de611c549b8c2f) — `Add rule never to open a new PR across all mode prompts (green)`

---

## Checks

- `make gate`: `7/7` steps passed (`lint-shell`, `lint-go`, `doccheck`, `errgen`, `test-agentbus`, `test-live`, `test-bats`) with `0` lint findings.
- `go test ./...` in `agentbus/`: passed (including `TestMemberStoppedState` and `TestLegacyStateLoads`).
- `go test ./...` in `live/`: passed (including `TestStoppedTeamMemberIsNotShownAsSleeping`, `TestDashboardShowsAtMost20LatestEntriesAndFullDashboardViewShowsAll`, and `TestTeamsViewHasStopThisTeamOption`).
- `bats tests/unit`: passed all new and existing unit tests (`kick.bats`, `web.bats`, `team_close.bats`, `investigation_team_admission.bats`, `serena.bats`, `member_pr.bats`, `one_pr_per_round.bats`).

## Skipped tests

- None (`0` skipped tests added).

## Not done

- `AGENTS.md` was not modified (per Rule 2 of `AGENTS.md`: *"Never change `AGENTS.md` or `CLAUDE.md`"*). If desired by the human, the "Prompts (`prompts/*.md`)" bullet in `AGENTS.md` can be updated to also name `inv-work.md` and `inv-merge.md`.
