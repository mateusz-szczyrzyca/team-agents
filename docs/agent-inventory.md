# R01 Baseline Inventory: bin/agent Rewrite to Go

**Base commit:** `940f6fdc067069f92a8023c84ed1b5eb011b95ae`
**Baseline snapshot:** bin/agent at 8,583 lines, SHA-256 `52fefa2be198cca4a57512a9236c2dbcd159a6a42a1e65dcb7d8b9b69df8073b`

## Public Commands

The primary entry points, all starting with `agent <command>`:

1. **run** — start a task: `agent run <agent> "task"` or `-f file.md`
2. **status** (or no args) — task list
3. **agents** — configured agents and whether busy
4. **show** — task details: `agent show <id>`
5. **answer** — edit questions: `agent answer <id>`
6. **resume** — continue after answers: `agent resume <id> ["note"]`
7. **stop** — stop a run: `agent stop <id>` or `--running` (all)
8. **clean** — remove task worktree: `agent clean <id>`
9. **inv** — investigation queue: `agent inv [number | --all]`
10. **dispatch** — start next investigation (fallback)
11. **live** — live panel: `agent live [--feed] [--remind MIN]`
12. **web** — web interface (foreground; normally agentweb.service)
13. **lead** — lead job listener (foreground; normally agentlead.service)
14. **node** — node daemon (foreground; normally agentnode.service)
15. **jobs** — remote jobs: `agent jobs [<id>]` or `--cancel <id>`
16. **nodes** — registered nodes on lead
17. **kick** — cut run short: `agent kick <task>`
18. **tokens** — usage per task/team: `agent tokens <team|task>` or `--runs <task>`
19. **round-size** — change size against limit: `agent round-size <task>`
20. **ack** — acknowledge finished: `agent ack <id|all>`
21. **team** — teams and members: `agent team [id | --all | <id> --close]`
22. **helpers** — worker requests: `agent helpers [--all]`
23. **transcript** — full log: `agent transcript <id|team> [--raw]`
24. **archive** — pack finished: `agent archive <id|team> | --finished`
25. **discard** — remove untouched: `agent discard <id>...`
26. **notes** — repository notebook: `agent notes [repo] [--section S] [--search Q] [-n N]`
27. **authenticated** — check model/provider: `agent authenticated [agents...] | --providers`
28. **bus** — bus state: `agent bus [N]`
29. **say** — message agents: `agent say <agent|all|team:ID> "text"`
30. **autopilot** — toggle team mode: `agent autopilot <team> [on|off]`

Special modifiers: `run --team`, `run --investigation`, `run --need`, `run --on`, `run --resume`, `run --last-team`, `run --help`.

## Hidden Commands

Commands not listed in help but present and callable:

1. **agent as gh** — When `argv[0]` is `gh`, acts as a GitHub filter (agent_gh function):
   - Trusted account filtering for comments/reviews/PRs
   - GraphQL query rejection for privacy
   - Fork/unknown-author filtering
2. **agent _runner** — Internal runner: `agent _runner <task> <run>` (systemd unit invocation)
3. **agent _fn** — Function/callback helpers (system/prompt/questions infrastructure)

## Function Families

Major internal function groups organized by responsibility:

- **Initialization & Config:** `load_config`, `load_secrets`, `apply_defaults`
- **Task & Run Management:** `task_*`, `run_*`, `alloc_task`, `alloc_cohort`, `resume_*`
- **Metadata Codec:** `meta_*`, `task_field`, `investigation_field`, `shellquote`
- **Git & Worktree:** `git_*`, `check_out`, `checkout_rollback`, `fetch_ref`
- **Systemd Integration:** `unit_*`, `unit_active`, `launch_runner`, `systemd_*`
- **Process Management:** `_run`, `_wait`, `_trap`, `process_*`, `watchdog`
- **Logging & Events:** `log_*`, `notice`, `gh_log`, `loginfo`, `logwarn`, `logerror`
- **Bus/MCP Interface:** `bus_*`, `quiz_*`, `consult_*`, `investigation_*`, `board_*`
- **GitHub Filtering (agent_gh):** `gh_log`, `gh_filter`, `gh_report`, `gh_show`
- **Prompts & Context:** `prompt_*`, `template_*`, `render_prompt`, `load_checkpoint`
- **Questions & Notifications:** `question_*`, `notify_*`, `answer_*`, `wait_for_answer`
- **Limits & Quotas:** `check_limits`, `quota_*`, `network_*`, `reset_pause`
- **Team Coordination:** `team_*`, `member_*`, `coordination_*`, `wake_*`
- **Investigation Queue:** `investigation_*`, `inv_queue`, `inv_*`
- **Remote/Node:** `remote_*`, `node_*`, `lead_*`, `jobs_*`
- **Review & Merging:** `review_*`, `find_pr`, `merge_pr`, `autopilot_*`
- **Archive & Cleanup:** `archive_*`, `pack_*`, `verify_archive`, `discard_*`

## Configuration Variables

59 configuration variables set in `config/agents.conf` and defaults in code:

**Agent Roster:**
- NORMAL_AGENTS, WORKER_AGENTS

**Watchdog & Hang Detection:**
- AGENT_HANG_MIN, AGENT_HANG_STRIKES
- AGENT_IDLE_RUN_SEC, AGENT_WAKE_LOOP_MAX
- AGENT_WAKE_RATE_MAX, AGENT_WAKE_LOOP_PAUSE_MIN

**Pause & Quota Limits:**
- AGENT_QUOTA_PAUSE_MIN, AGENT_RELEASE_PAUSE_MIN
- AGENT_RATE_LIMIT_PAUSE_MIN, AGENT_LIMIT_TAIL_LINES

**Language & Identity:**
- AGENT_LANGUAGE, REPO_LANGUAGE
- GIT_NAME_FORMAT, GIT_EMAIL, GIT_SIGNING_KEY

**Execution Binaries:**
- CLAUDE_BIN, CODEX_BIN, AGY_BIN, OPENCODE_BIN
- AGENTBUS_BIN, OPENCODE_BASH_TIMEOUT_MS

**Delivery & Notifications:**
- RUN_PATH, NOTIFY_CMD, QUESTIONS_EDITOR
- ENV_NOTES_FILE, VSCODE_SERVER_DIRS

**Investigation Queue:**
- INVESTIGATION_QUEUE_MAX
- AGENT_CONSULT_MIN

**Verification & Rounds:**
- VERIFY_MAX_FIX_PASSES
- ROUND_MAX_LINES, ROUND_SOFT_LINES, ROUND_EXCLUDE, ROUND_BOUNCES

**GitHub & Web:**
- AGENT_HUMAN_GITHUB
- AGENT_WEB_LISTEN, AGENT_WEB_USERS, AGENT_WEB_DEVICES, AGENT_WEB_URL

## Persisted State Files

Located in `~/agents/` under exclusive flock:

- **bus/state.json** — shared bus state: tasks, runs, teams, investigations, queues, reviews
- **bus/messages.jsonl** — append-only event log of inter-agent messages
- **bus/checkpoints/** — per-round checkpoints (findings, merged commits, board state)
- **tasks/<id>/meta.env** — task metadata: status, run ID, branch, author, checkpoint, answers
- **tasks/<id>/activity.jsonl** — per-run activity (normalized agent output events)
- **tasks/<id>/<run>/transcript.jsonl** — raw agent CLI output (if captured)
- **tasks/<id>/questions-N.md** — questions for the human (one per round)
- **tasks/<id>/answers-N.md** — human's marked verdict/decisions
- **tasks/<id>/A.inv**, **tasks/<id>/B.inv** — investigation slot occupation markers
- **logs/agent.jsonl** — agent CLI events (bus, discoveries, errors, warnings)
- **notebooks/** — human-readable findings and convention notes (additive; no overwrite)

## Structured Log Events

Stable event names (keys in activity.jsonl, bus messages, and logs/agent.jsonl):

**Core Lifecycle:** `run_start`, `run_start_ack`, `run_finish`, `run_stopped`, `run_failed`, `run_crashed`
**Status Transitions:** `status_new`, `status_starting`, `status_running`, `status_sleeping`, `status_paused`, `status_done`, `status_failed`
**Team Events:** `team_new`, `team_admission`, `team_member_added`, `team_member_excluded`, `team_coordination_open`, `team_done`, `team_closed`
**Investigation:** `investigation_new`, `investigation_queued`, `investigation_started`, `investigation_completed`, `investigation_failed`, `investigation_pipeline_phase`
**Questions:** `questions_posted`, `questions_answered`, `questions_bounce`, `answer_validated`, `answer_incorrect`
**GitHub:** `github_trust`, `github_graphql_refused`, `github_comment_ignored`, `github_comment_marked_ignored`, `github_pr_merge`, `github_pr_ready`
**Limits & Signals:** `quota_pause`, `rate_limit_pause`, `signal_terminated`, `signal_interrupted`, `watchdog_hung_group`
**Errors:** `bus_locked_timeout`, `storage_write_failed`, `process_failed`, `memory_quota_exceeded`

(See `bin/agent` for complete list; the names are stable contracts with dashboards and alerts.)

## Linux Locks

Exclusive locks via `flock`:

- **bus/state** — exclusive lock guarding bus state reads/writes (lock timeout: 5 s)
- **~/.ssh/known_hosts.lock** — (if SSH is used; not primary to agent workflow)
- Task-specific: task meta.env write operations hold task directory brief exclusive access

Lock order: always acquire bus state lock before task locks; never sleep while holding any lock; deadlock prevention: all operations must complete or timeout within 5 s.

## Bats Test Baseline

- **Test Files:** 120 .bats files in tests/unit/
- **Test Cases:** 483 @test declarations (exact: 483 at base 940f6fdc)
- **Test Families:** agent, agentbus (integration via subprocess), agentlive, installation, configuration, GitHub filtering
- **Classification:** 359 black-box cases (external commands/effects only), 121 source-coupled cases (source extraction/inspection)
- **Helpers:** setup_isolation, control_fixture, pair_of, and many stubs in tests/unit/stubs/

## Round Allocation Summary (R01–R56)

All 56 rounds fit within the repository's 1,500 added-plus-deleted lines per round (excluding generated and vendored). Strategic split points to prevent oversizing:

| Rounds | Category | Focus |
|--------|----------|-------|
| R01 | Documentation | Baseline inventory, ledger structure, no code |
| R02–R04 | Core Skeleton | Module setup, CLI scaffolding, test lanes |
| R05–R10 | Configuration & CLI | Config loading, GitHub trust, metadata codec |
| R11–R17 | Prompt & Context | Template rendering, checkpoints, adapters |
| R18–R22 | Process & Execution | Watchdog, normalizer, runner, delivery |
| R23–R27 | Solo Workflows | Questions, notifications, resume, cleanup |
| R28–R31 | Team Coordination | Admission, finish, member transitions |
| R32–R34 | Review & Autopilot | Precheck, pairing, merge eligibility |
| R35–R39 | Investigation Teams | Dispatch, phases, completion, clarification |
| R40 | Advisory Resume | Manifest recovery, admission |
| R41–R49 | Remote & Node | Job dispatch, reporting, sync, controls |
| R50–R54 | Remaining Gaps | Archives, transcripts, helpers, commands |
| R55 | Cutover | Installer tests, oracle preservation, build wiring |
| R56 | Qualification | Rollback/crash scenarios, release docs, binary acceptance |

---

**Report:** This inventory documents the static structure before any Go implementation. Ledger, Bats classification, and function-family mapping are derived from the baseline commit. Exact counts (483 tests, 59 variables, 30 public commands) will be verified in R01 verification and do not change baseline behavior.
