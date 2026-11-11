# R01 Baseline Inventory: bin/agent Rewrite to Go

**Base commit:** `940f6fdc067069f92a8023c84ed1b5eb011b95ae`
**Baseline snapshot:** `bin/agent` at 8,583 lines, SHA-256 `52fefa2be198cca4a57512a9236c2dbcd159a6a42a1e65dcb7d8b9b69df8073b`
**Baseline test results:** Full legacy Bats suite passes all 480 test cases across 120 files (446 parallel + 34 serial in `tests/unit/`) in Lane A. `make gate` passes all 6 baseline steps (`check-generated`, `lint-shell`, `lint-go`, `lint-docs`, `unit-go`, `unit-bats`) with zero findings.

## Public Commands

The 33 primary CLI entry points dispatched in `main()` (`bin/agent:8526–8578`) and documented in `usage()` (`bin/agent:8475–8524`):

1. **run** — start a task (`cmd_run`): `agent run <agent> "task"` or `-f file.md`. Flags: `--team`, `--verify`, `--autopilot`, `--investigation`, `--need <caps>`, `--on <instance>`, `--resume <team|task>`, `--same-round`, `--last-team`. Note: flag `--help` is not an option of `cmd_run`; help is provided via `agent help`.
2. **status | ls** — task list (`cmd_status`). Note: invoking `bin/agent` with no arguments defaults `$cmd` to `help` (`main()`: `local cmd=${1:-help}`), not `status`.
3. **agents** — configured agents and whether they are busy (`cmd_agents`).
4. **show** — task details and last message (`cmd_show <id>`).
5. **answer** — open questions in `$EDITOR`, validate answers, resume (`cmd_answer <id>`).
6. **resume** — resume after questions are answered (`cmd_resume <id> ["note"]`).
7. **log** — follow runner log with formatted structured records (`cmd_log <id> [--raw]`).
8. **stop** — stop a run (`cmd_stop <id>`) or stop all active runs and queues (`cmd_stop --running`).
9. **clean** — remove a task worktree while keeping the branch (`cmd_clean <id>`).
10. **bus** — show bus status and last N messages (`cmd_bus [N]`).
11. **inv** — view investigation queue or details of one request (`cmd_inv [number | --all]`).
12. **live** — interactive terminal dashboard panel (`cmd_live [--feed] [--remind MIN]`).
13. **web** — web interface in foreground (`cmd_web`; normally runs as `agentweb.service`).
14. **lead** — lead instance job listener in foreground (`cmd_lead`; normally runs as `agentlead.service`).
15. **node** — node daemon in foreground (`cmd_node [--once]`; normally runs as `agentnode.service`).
16. **jobs** — list or manage remote jobs on lead or node (`cmd_jobs [<id>]` or `--cancel <id>`).
17. **nodes** — list registered worker nodes on lead (`cmd_nodes`).
18. **kick** — cut run short (`cmd_kick <task>`).
19. **wake** — resume a paused task immediately without waiting for pause expiration (`cmd_wake <task>`).
20. **tokens** — inspect token usage per task/team (`cmd_tokens <team|task>` or `--runs <task>`).
21. **round-size** — measure PR diff size against configured round limits (`cmd_round_size <task>`).
22. **ack** — acknowledge finished tasks so they leave the live panel (`cmd_ack <id|all>`).
23. **authenticated** — verify agent models with provider APIs (`cmd_authenticated [agent...] | --providers`).
24. **team** — inspect teams and members (`cmd_team [id | --all]`), or close a team (`cmd_team <id> --close`).
25. **autopilot** — display or toggle team autopilot mode (`cmd_autopilot <team> [on|off]`).
26. **helpers** — list worker delegation requests (`cmd_helpers [--all]`).
27. **transcript** — display chronological log of task or team (`cmd_transcript <id|team> [--raw]`).
28. **archive** — pack finished task into 7z archive (`cmd_archive <id|team> | --finished`).
29. **discard** — remove unstarted task worktree, branch, and metadata (`cmd_discard <id>...`).
30. **notes** — inspect repository notebook (`cmd_notes [repo] [--section S] [--search Q] [-n N]`).
31. **dispatch** — trigger manual dispatch of next queued investigation (`cmd_dispatch`).
32. **say** — send a message to agents or teams over the bus (`cmd_say <agent|all|team:ID> "text"`).
33. **help | -h | --help** — display CLI usage text (`usage`).

## Hidden and Internal Commands

Internal entry points and callbacks dispatched by `main()`:

1. **agent as gh** — when `filepath.Base(argv[0]) == "gh"` (`bin/agent:200`), delegates to `agent_gh` to filter GitHub API responses (trusted authors, reject GraphQL, ignore external comments/reviews).
2. **_runner <task> <mode>** — transient systemd unit entry point (`cmd__runner`, line 4507).
3. **_agents-json** — emit configured agent list in JSON format (`cmd__agents_json`, line 7984).
4. **_dispatch-helpers** — helper request matching and dispatch loop (`dispatch_helpers`, line 4269).
5. **_wake <task>** — execute scheduled wake action for a paused task (`cmd__wake`, line 2233).
6. **_remote_member <proxy> <status> <branch> <note>** — remote member callback entry point (`cmd__remote_member`, line 6239).
7. **_precheck <task>** — run gate check and test verification on round head (`cmd__precheck`, line 1602).
8. **_merge <task>** — perform automated autopilot merge of approved round PR (`cmd__merge`, line 1353).
9. **_fixpass <task>** — launch review fix pass run for a team (`cmd__fixpass`, line 1754).
10. **_supervise** — background watchdog supervising active units and orphaned runners (`cmd__supervise`, line 6067).
11. **_answers_saved <id> <round> [source]** — callback triggered when human finishes editing questions (`cmd__answers_saved`, line 5781).
12. **_remote_questions <task>** — remote questions staging and preparation (`cmd__remote_questions`, line 712).
13. **_remote_questions_ready <task>** — mark remote questions as ready for pickup (`cmd__remote_questions_ready`, line 723).
14. **_authcheck <provider|--agent NAME>** — run single provider probe for authentication verification (`cmd__authcheck`, line 8064).
15. **_investigation-team [--pipeline] --team names [-f topic-file] [--] [topic]** — launch and supervise independent investigation cohort (`cmd__investigation_team`, line 4095).

## Function Families

All 318 bash functions declared in `bin/agent` grouped by operational domain with source line anchors and planned round ownership:

- **GitHub CLI Filter (`agent as gh`) [R10, R11, R12]:** `agent_gh` (16), `gh_log` (25), `gh_report` (44), `gh_filter` (57), `gh_show` (69), `count_of` (1182), `findings_count` (1191), `agents_gh_env` (1236), `review_byline` (1252), `github_trusted` (1261), `trusted_prs` (1272), `pr_format_issues` (1286), `merge_blockers` (1317), `github_repo_url` (1159), `github_owner` (1176).
- **Git Identity & Commit Trailers [R10]:** `git_identity` (2498), `commit_trailers` (2523).
- **Structured Logging, Side Effects & Error Handling [R08]:** `log_event` (221), `log_info` (251), `log_warn` (252), `log_error` (253), `try` (258), `jq_soft` (272), `die` (284), `act` (2795), `last_event` (2802), `render_log` (5898).
- **Agent Roster & Providers [R05, R51]:** `agent_provider` (294), `agent_tier` (304), `investigator_names` (392), `is_investigator` (393), `agent_running_task` (543), `agent_desc` (1069), `agent_cli_present` (6799).
- **Bus MCP Bridge & Communication [R08]:** `bus` (455), `notice` (932), `push_text` (940), `bus_messages_from` (2406), `bus_args_json` (2777).
- **Task Metadata, Identification & Lock Serialization [R06, R07, R14]:** `meta_file` (465), `meta_set` (467), `meta_load` (513), `task_field` (522), `resolve_id` (524), `lock_task_ids` (4946), `unlock_task_ids` (4956), `allocate_run_ids` (4963), `create_task` (5033), `drop_unstarted_task` (5091), `job_refusal` (5104), `prepare_job_task` (5124), `discard_task` (5180), `abandon_run` (5197), `boot_id` (6205).
- **Systemd & Process Management [R15, R20]:** `unit_name` (536), `unit_active` (538), `start_task_unit` (1347), `killtree` (2807), `watch_run` (2814), `activity_progress` (2853), `runner_exit` (4440), `cmd__runner` (4507), `start_runner` (4925).
- **Questions, Answers & Notifications [R23, R24]:** `count_questions` (568), `validate_answers` (580), `questions_missing_recommendations` (605), `expand_answers` (618), `answers_marked` (643), `questions_note` (648), `file_mtime` (668), `questions_ready` (675), `watch_answers` (795), `stop_answers_watch` (807), `open_in_editor` (817), `notify_send` (898), `notify_failure_keep` (918), `notify` (3068), `questions_from_nostatus` (4398), `questions_from_final` (4406), `cmd_answer` (5740), `cmd__answers_saved` (5781), `supervise_answer_watches` (1828).
- **Prompt Construction & Session Hygiene [R16, R17]:** `render` (856), `agents_md_text` (2531), `privileges_text` (2552), `peers_list` (2561), `build_prompt` (2567), `cli_prompt` (2866), `round_text_strip` (6387), `checkpoints_latest` (6392), `fresh_session_due` (6407).
- **Provider CLI Execution Adapters [R18, R19]:** `run_claude` (2899), `codex_mcp_args` (2929), `run_codex` (2938), `run_gemini` (2967), `opencode_config` (3009), `run_opencode` (3032).
- **Watchdog, Limits, Quotas & Wake Control [R20, R21, R25]:** `stopping` (458), `cmd__wake` (2233), `limit_keep` (2295), `limit_lines` (2299), `last_error_line` (2319), `limit_kind` (2327), `limit_line` (2342), `quota_reset_seconds` (2353), `duration_seconds` (2393), `recent_run_starts` (2414), `wake_loop_applies` (2421), `wake_loop_check` (2430), `schedule_wake` (2454), `pause_task` (2463), `resume_paused` (2476), `pause_until_from_note` (1798), `pause_reason_from_note` (1816), `pl_commits` (4358), `verify_gaps` (4363), `final_status` (4393), `stop_all` (5921), `cmd_stop` (5962), `cmd_clean` (5990), `cmd__supervise` (6067), `long_run_end` (6432), `cmd_wake` (6494), `cmd_kick` (6515).
- **Team Coordination & Board [R28, R29]:** `team_members_list` (863), `team_json` (884), `team_lead` (894), `team_done_flag` (895), `team_tasks` (896), `team_summary` (949), `finish_team` (956), `team_waits_on_others` (984), `last_team_id` (1139), `team_lead_task` (1862), `team_after_run` (2101), `team_all` (2194), `supervise_teams` (6131), `cmd_team` (8213), `team_close` (8226).
- **Rounds, Precheck, Review Pairs & Autopilot [R31, R32, R33, R34]:** `round_limit` (991), `round_limit_text` (995), `round_soft` (1004), `round_base_ref` (1009), `skips_in_repo` (1033), `skips_added` (1040), `pr_skipped_section` (1050), `skip_note` (1057), `attribution_block` (1074), `round_check` (1092), `round_pr_url` (1121), `cmd_round_size` (1127), `review_verdict` (1199), `review_task_verdict` (1204), `round_test_changes` (1218), `cmd__merge` (1353), `cmd_autopilot` (1417), `supervise_review_waits` (1438), `review_gate_cmd` (1452), `auto_stage` (1462), `human_on_pr` (1472), `round_open_prs` (1483), `round_pr_note` (1498), `round_pr_of_team` (1516), `round_pr_supersede` (1527), `gate_at` (1335), `gate_progress` (1541), `pair_stage` (1554), `start_precheck` (1571), `team_wait_settled` (1585), `cmd__precheck` (1602), `review_precheck` (1622), `review_followup` (1720), `review_fix_pass` (1744), `cmd__fixpass` (1754), `do_fix_pass` (1762), `review_without_verdict` (1875), `retry_review_slot` (1885), `review_pair_done` (1900), `free_investigator_list` (1967), `start_review_pairs` (1988), `start_review_pair` (2003), `start_review_slot` (2046), `request_review` (2067).
- **Investigation Teams & Single Pipeline [R35, R36, R37, R38, R39, R40]:** `is_independent_investigation` (2888), `pick_investigator` (3170), `is_one_of` (3194), `remember_investigator` (3201), `free_investigator` (3206), `investigation_resume_write` (3215), `investigation_resume_tick` (3222), `investigation_resume_admit` (3269), `investigation_resume` (3324), `investigation_phase1_write` (3372), `investigation_published` (3381), `investigation_phase1_recover` (3409), `investigation_team_tick` (3441), `investigation_field` (3523), `investigation_completed_wait` (3527), `investigation_handoff` (3537), `investigation_phase_start` (3565), `investigation_phase_choose` (3636), `investigation_phase_failed` (3658), `investigation_pipeline_tick` (3678), `_investigator_is_busy` (3779), `investigation_phase_pick` (3799), `investigation_team_stop` (3912), `investigation_teams_cleanup` (3983), `investigation_teams_tick` (3999), `investigation_team_admit` (4015), `investigation_teams_admit` (4079), `cmd__investigation_team` (4095), `dispatch` (4136), `start_investigation` (4155), `inv_finish` (4211), `validate_investigation_team` (5497), `validate_investigation_topic` (5515), `cmd_inv` (6015), `cmd_dispatch` (6028).
- **Worker Helper Delegation [R35, R50]:** `maybe_resume_worker` (4233), `free_worker` (4249), `dispatch_helpers` (4269), `start_helper` (4305), `helper_finish` (4346), `cmd_helpers` (8255).
- **Task Lifecycle, Run Admission & Resume [R13, R26, R27, R30]:** `resume_history` (5207), `resume_run` (5218), `cmd_run` (5532), `cmd_resume` (5816), `member_out` (6047), `restart_interrupted` (6210), `resume_abandon` (6288), `worktree_stash` (6371).
- **Read & Status Inspection [R09]:** `cmd_status` (5848), `cmd_show` (5870), `cmd_agents` (6002), `cmd_ack` (7999), `cmd__agents_json` (7984).
- **Multi-Instance Lead & Node Networking [R41, R42, R43, R44, R45, R46, R47, R48, R49, R50]:** `remote_identity` (686), `remote_question_context` (692), `cmd__remote_questions` (712), `cmd__remote_questions_ready` (723), `supervise_remote_questions` (753), `remote_answers_saved` (774), `remote_start_check` (1845), `handover_failed` (1853), `supervise_member_jobs` (6111), `remote_member_task` (6163), `node_stop_doorbell` (6199), `cmd__remote_member` (6239), `remote_agent_wait_free` (6273), `remote_member_start` (6318), `cmd_lead` (6584), `node_jobs_file` (6597), `node_jobs_load` (6600), `node_jobs_save` (6612), `node_save` (6622), `node_lock` (6625), `lead_url` (6632), `node_request` (6643), `submit_remote_job` (6646), `cmd_jobs` (6678), `cmd_nodes` (6771), `node_activity_json` (6810), `node_job_result` (6828), `node_heartbeat_json` (6837), `node_select_agent` (6866), `node_time` (6884), `node_map_state` (6895), `node_transition` (6907), `node_reconcile` (6924), `node_start_ack` (6945), `node_process_pending_launches` (6961), `node_recover_tasks` (7031), `node_bind_task` (7083), `node_scan_tasks` (7091), `node_flush_reports` (7128), `node_clear_control_acks` (7156), `node_apply_answer` (7168), `node_apply_kick` (7219), `node_handle_controls` (7241), `node_handle_stop` (7251), `node_handle_reply` (7279), `node_poll_remote_helpers` (7320), `node_sync_save_json` (7422), `node_read_meta` (7440), `node_mark_job_obsolete` (7500), `node_sync_terminal_unchanged` (7513), `node_sync_has_new_logs` (7528), `node_send_sync` (7550), `node_sync_task` (7591), `node_sync_tasks` (7898), `node_poll` (7931), `cmd_node` (7952).
- **Authentication Probing [R51]:** `auth_classify` (8020), `api_error_text` (8038), `agent_auth_target` (8046), `cmd__authcheck` (8064), `auth_probe` (8082), `opencode_model_for` (8149), `cmd_authenticated` (8158), `auth_providers` (8192).
- **Archives & Discards [R52]:** `archive_index` (8336), `archive_group` (8348), `sevenzip` (8366), `archive_one` (8368), `cmd_discard` (8428), `cmd_archive` (8440).
- **Interactive UI, Reporting, Notes, Bus CLI & Dispatch Dispatcher [R02, R53]:** `cmd_live` (6030), `cmd_log` (5909), `runs_usage` (6339), `cmd_tokens` (6443), `cmd_web` (6574), `cmd_notes` (8242), `cmd_transcript` (8257), `cmd_bus` (8466), `cmd_say` (8468), `usage` (8475), `main` (8526).

## Configuration Variables

Loaded from `config/agents.conf` with defaults initialized in `bin/agent`:

### Agent Roster & Provider Settings
- `AGENT_TYPE` (associative array: name -> CLI type: `claude`, `codex`, `gemini`, `opencode`)
- `NORMAL_AGENTS` (indexed array of agent names eligible for coordinator role)
- `WORKER_AGENTS` (indexed array of cheap worker agent names)
- `AGENT_MODEL` (associative array: name -> model identifier/preset)
- `AGENT_EXTRA_ARGS` (associative array: name -> extra CLI arguments)

### Investigators
- `INVESTIGATORS_TYPE` (associative array: investigator name -> CLI type)
- `INVESTIGATORS_MODEL` (associative array: investigator name -> model ID)
- `INVESTIGATORS_EXTRA_ARGS` (associative array: investigator name -> CLI arguments)
- `INVESTIGATION_QUEUE_MAX` (max queued investigation requests; default: `3`)
- `AGENT_INVESTIGATION_SEED` (decimal entropy seed for reproducible assignment selection)
- Legacy compatibility fallbacks: `INVESTIGATOR_NAME`, `INVESTIGATOR_TYPE`, `INVESTIGATOR_MODEL`, `INVESTIGATOR_EXTRA_ARGS`

### Watchdog, Quotas & Wake Guard
- `AGENT_HANG_MIN` (watchdog hang threshold in minutes; default: `30`)
- `AGENT_HANG_STRIKES` (consecutive hang threshold before marking hung; default: `3`)
- `AGENT_IDLE_RUN_SEC` (minimum run duration in seconds considered non-idle; default: `120`)
- `AGENT_WAKE_LOOP_MAX` (consecutive idle runs before pausing member; default: `4`)
- `AGENT_WAKE_RATE_MAX` (max runs per 10 minutes before pausing member; default: `30`)
- `AGENT_WAKE_LOOP_PAUSE_MIN` (wake-loop guard pause duration in minutes; default: `15`)
- `AGENT_QUOTA_PAUSE_MIN` (fallback pause duration for exhausted quota; default: `30`)
- `AGENT_RELEASE_PAUSE_MIN` (pause threshold after which board items are released; default: `20`)
- `AGENT_RATE_LIMIT_PAUSE_MIN` (pause duration for HTTP 429 rate limit; default: `5`)
- `AGENT_LIMIT_TAIL_LINES` (stderr lines inspected for limit classification; default: `30`)
- `AGENT_NETWORK_PAUSE_MIN` (pause duration for network/provider unreachable; default: `15`)
- `AGENT_NETWORK_MAX_PAUSES` (max network pauses before failing task; default: `12`)

### Supervisor & Restarts
- `AGENT_SUPERVISE_GRACE_SEC` (grace period before checking unit presence; default: `120`)
- `AGENT_RESTART_MAX` (maximum host restarts tolerated per task; default: `2`)
- `AGENT_RESTART_GRACE_MIN` (lead tolerance for absent node before ejecting remote member; default: `10`)
- `AGENT_TEAM_ABANDON_MIN` (timeout for reporting abandoned open team; default: `60`)

### Review, Verification, Rounds & Autopilot
- `AGENT_REVIEW_DRAW` (investigator selection mode: `random` or `list`; default: `random`)
- `ROUND_MAX_LINES` (hard round limit for PR diff lines; default: `3000`)
- `ROUND_SOFT_LINES` (target soft round limit; default: `2000`)
- `ROUND_EXCLUDE` (patterns excluded from round line count; default: `*_gen.go go.sum package-lock.json vendor/`)
- `ROUND_ALWAYS_EXCLUDE` (internal unreviewable artifacts always excluded: `.agents/reports/`)
- `ROUND_BOUNCES` (maximum times a round is bounced to be split; default: `2`)
- `ROUND_MAX_LINES_REPO` (associative array: repo name -> custom line limit)
- `VERIFY_MAX_FIX_PASSES` (maximum review fix cycles before escalating to human; default: `2`)
- `AGENT_HUMAN_GITHUB` (GitHub username of human maintainer for mentions)
- `AGENT_REVIEW_GATE` (gate check command: `auto` for `make gate`, or `off`; default: `auto`)
- `AGENT_REVIEW_GATE_MIN` (timeout for precheck gate run in minutes; default: `30`)
- `AGENT_GATE_SCOPES` (array of `"patterns => command"`: the gate of a change whose every path matches an entry; default: empty)
- `AGENT_STAGE_POLL_SEC` (interval for reporting gate progress; default: `2`)
- `AGENT_PRECHECK_MAX` (maximum retries for precheck gate failures; default: `2`)
- `AGENT_REVIEW_WAIT_MIN` (wait threshold before reporting stalled review pair; default: `30`)
- `AUTO_MERGE_HUMAN_PATHS` (protected path prefixes requiring human merge review)
- `AUTO_MERGE_HOLD_TEST_CHANGES` (hold autopilot merge if tests were modified; default: `yes`)

### Prompt Context & CLI Limits
- `AGENT_PROMPT_CHECKPOINTS` (number of recent checkpoints embedded in prompt; default: `2`)
- `AGENT_FRESH_SESSION_TOKENS` (threshold for input tokens per step triggering fresh session: `in / steps > AGENT_FRESH_SESSION_TOKENS`; default: `80000`)
- `AGENT_FRESH_SESSION_RATIO` (threshold ratio comparing input tokens per step `in / steps` against the median of earlier session starts `new` or `fresh`: `in / steps > AGENT_FRESH_SESSION_RATIO * median`; default: `2`)
- `AGENT_LONG_RUN_STEPS` (soft step limit prompting worker to wrap up; default: `120`)
- `AGENT_LONG_RUN_HARD` (hard step limit terminating worker run; default: `200`)
- `AGENT_PROMPT_ARG_MAX` (byte threshold before passing prompt via file; default: `100000`)
- `CLAUDE_MAX_TURNS` (Claude Code turn limit; default: `300`)
- `AGY_PRINT_TIMEOUT` (Antigravity CLI print timeout; default: `8h`)
- `OPENCODE_BASH_TIMEOUT_MS` (Opencode bash tool timeout in ms; default: `600000`)

### Paths, Environment & Git Identity
- `AGENTS_HOME` (root directory; default: `$HOME/agents`)
- `CONF` (path to `agents.conf`; default: `$AGENTS_HOME/agents.conf`)
- `TASKS` (`$AGENTS_HOME/tasks`), `PROMPTS` (`$AGENTS_HOME/prompts`), `BUS_DIR` (`$AGENTS_HOME/bus`), `LOG_DIR` (`$AGENTS_HOME/logs`)
- `RUN_PATH` (execution PATH passed to runners)
- `XDG_RUNTIME_DIR` (user runtime directory used by systemd --user and transient units)
- `CLAUDE_BIN`, `CODEX_BIN`, `AGENTBUS_BIN`, `AGENTLIVE_BIN`, `AGY_BIN`, `OPENCODE_BIN`
- `AGENT_LANGUAGE` (human-facing language; default: `English`)
- `REPO_LANGUAGE` (repo-facing language; default: `English`)
- `GIT_NAME_FORMAT` (git author format; default: `%a (team-agents)`)
- `GIT_EMAIL` (git author email; fallback: `<agent>@team-agents.invalid`)
- `GIT_SIGNING_KEY` (SSH commit signing key)
- `ENV_NOTES_FILE` (hardware/environment notes; default: `$AGENTS_HOME/hardware.md`)
- `BRANCH_PREFIX` (git branch namespace prefix; default: `agents`)
- `AGENT_GITHUB_TRUSTED` (trusted GitHub accounts for PR and review filtering)
- `AGENT_GITHUB_TRUSTED_RESOLVED` (resolved space-separated list of trusted GitHub accounts)

### Questions & Notifications
- `NOTIFY_CMD` (command executed with notification file path)
- `NOTIFY_SCRIPT` (legacy notification script executed in `$NOTIFY_DIR`)
- `NOTIFY_DIR` (`$AGENTS_HOME/notifications`)
- `NOTIFY_MAX_CHARS` (notification body length cap; default: `1500`)
- `QUESTIONS_EDITOR` (editor for questions: `vscode` or `off`; default: `vscode`)
- `VSCODE_SERVER_DIRS` (paths searched for VS Code remote CLI)
- `AGENT_CONSULT_MIN` (minutes coordinator waits for team consult replies; default: `5`)

### Multi-Instance & Web Interface
- `LEAD_LISTEN` (lead Tailscale address for incoming node jobs; empty = not a lead)
- `LEAD_NODES` (allowed node devices/tags)
- `INSTANCE_NAME` (short host name on lead)
- `NODE_TTL_SEC` (timeout before silent node is marked offline; default: `90`)
- `LEAD_URL` (lead HTTP URL reached by worker node; empty = not a node)
- `NODE_POLL_SEC` (node heartbeat interval in seconds; default: `5`)
- `INSTANCE_CAPABILITIES` (indexed array of capability tags for node)
- `NODE_REPOS` (associative array: repo name -> local clone path)
- `AGENT_WEB_LISTEN` (web interface address; default: `127.0.0.1:8787`)
- `AGENT_WEB_USERS`, `AGENT_WEB_DEVICES`, `AGENT_WEB_URL`

### Authentication Probing
- `LIVE_REMIND_MIN` (reminder interval in minutes; default: `1`)
- `AUTH_MODEL_ANTHROPIC`, `AUTH_MODEL_OPENAI`, `AUTH_MODEL_GEMINI`, `AUTH_MODEL_OPENROUTER`, `AUTH_MODEL_DEEPINFRA`
- `AUTH_OPENROUTER_URL`, `AUTH_DEEPINFRA_URL`

## Persisted State Files

### Global Files in `~/agents/`
- **bus/state.json** — shared bus state (tasks, runs, teams, investigations, board, registrations) guarded under `flock` on `bus/bus.lock` by `agentbus`.
- **bus/bus.lock** — exclusive lock file for `agentbus` transactions.
- **bus/messages.jsonl** — append-only bus event message journal.
- **bus/jobs.json** — active and queued remote job registrations on lead.
- **bus/node-jobs.json** — local node job transition state snapshot.
- **bus/node-daemon.lock** — mutual exclusion lock file for single node daemon process.
- **bus/sync_log_cursors.json** — log sync progress markers across remote nodes.
- **bus/checkpoints/<key>.md** — checkpoint records (`task-<id>.md`, `team-<id>.md`).
- **bus/abandoned-teams/<team>** — marker file for abandoned teams.
- **bus/remote/<node>/<task>/metadata.json** — remote node task metadata mirrors.
- **bus/notebooks/<repo>/sections.json** — named overwritable notebook sections per repository.
- **bus/notebooks/<repo>/log.jsonl** — append-only notebook entries per repository.
- **task-ids.lock** — serialization lock for allocating unique task and cohort IDs.
- **dispatch.lock** — mutual exclusion lock for investigation queue dispatch and cohort admission.
- **helpers.lock** — mutual exclusion lock for worker helper dispatch.
- **notify-failures.jsonl** — persistent log of failed notification deliveries.
- **notify-failures.jsonl.lock** — lock file serializing notification failure logging.
- **last-investigator** — round-robin rotation state tracking last assigned investigator.
- **logs/agent.jsonl** — append-only structured log records from the `agent` script.
- **.stop-all** — timestamp marker file active during `agent stop --running`.
- **tasks-archived/*.7z** — compressed archives of completed tasks.
- **notifications/* — notification message files.
- **reviews/<repo>-<PR-number>/pair.lock** — lock serializing verdict recording and completion decision for a review pair.
- **reviews/<repo>-<PR-number>/A.verdict**, **B.verdict** — recorded verdicts for review slots A and B.
- **reviews/<repo>-<PR-number>/A.md**, **B.md** — review commentary and findings.
- **reviews/<repo>-<PR-number>/A.inv**, **B.inv** — investigator request/assignment IDs for review slots A and B. Written at `bin/agent:2062` during `start_review_slot`, read at `bin/agent:1905–1906` in `review_pair_done` to match a completed review task back to its slot, displayed at `bin/agent:2041` in board stage messages, and read at `bin/agent:1564` in `reviews_waiting_summary`.
- **reviews/<repo>-<PR-number>/decided** — marker indicating one reviewer completed the pair decision.
- **reviews/<repo>-<PR-number>/A.agent**, **B.agent** — assigned investigator names for slots A and B.
- **reviews/<repo>-<PR-number>/A.retried**, **B.retried** — slot retry tracking markers.
- **tasks/.team-<team>.stuck** — timestamp marker tracking when a team became stuck (all members done or sleeping without coordinator closure; line 2193).
- **remote-questions/<node>/<task>/ready.lock** — lock serializing remote question readiness staging and watch setup.
- **remote-questions/<node>/<task>/answers.lock** — lock serializing remote answer validation and forwarding.
- **remote-questions/<node>/<task>/questions-<round>.md** — staged questions files for remote tasks.
- **remote-questions/<node>/<task>/questions-<round>.notified** — marker indicating human editor and notifications were triggered.

### Per-Task Files in `~/agents/tasks/<id>/`
- **meta.env** — task metadata formatted with shell `%q` quoting.
- **meta.env.lock** — exclusive lock guarding reads and writes of `meta.env` and transitions.
- **task.md** — original task prompt / specification.
- **result.md** — final summary or output reported by the agent.
- **final.md** — extracted final message response from the agent turn.
- **runner.log** — combined stdout and fd 3 structured log output of the runner process.
- **runner.pid** — PID of currently executing runner process.
- **activity.jsonl** — normalized activity event stream produced by `agentbus normalize`.
- **activity-full.jsonl** — complete uncompacted normalized event stream.
- **job_transitions.jsonl** — transition journal for remote node jobs.
- **out-$n.jsonl** — raw CLI event stream for run `$n` (Claude Code, Gemini).
- **events-$n.jsonl** — raw CLI event stream for run `$n` (Codex).
- **err-$n.log** — stderr capture for run `$n`.
- **last-$n.md** — agent turn output for run `$n` (Codex).
- **prompt-$n.md** — fully rendered prompt provided to the CLI for run `$n`.
- **mcp.json** / **opencode-config.json** — provider-specific MCP configuration files in task directory (Claude Code, OpenCode). Note: Gemini/Antigravity uses `.agents/mcp_config.json` created in the working tree (`$WORKTREE/.agents/mcp_config.json`).
- **questions-$round.md** / **questions.md** — questions submitted to human for round `$round`.
- **fix-pass.n** — stores the count of critical and major findings to address in the fix pass. Written at `bin/agent:1744–1746` in `review_fix_pass`; read at `bin/agent:1758` in `cmd__fixpass` and passed to `do_fix_pass` (the retry iteration index is the separate `FIX_PASS` metadata field read at `bin/agent:1765`).
- **fix-pass.md** — fix-pass review findings and instructions for follow-up run. Written at `bin/agent:1747` alongside fix-pass.n.
- **note-$RUN.md** — resume notes passed to the agent CLI for run $RUN. Written at `bin/agent:5824` by `cmd_resume` when additional notes are supplied upon resuming a waiting task; read at `bin/agent:2615` and `bin/agent:2768` during prompt construction to include prior run notes in resume/fresh sessions.
- **history.md** — accumulated history of round progress, questions, and decisions.
- **postmerge.log** / **precheck.log** — gate check and test logs during verification.
- **phase1.json** — manifest of accepted cohort member tasks in independent investigation.
- **resume.json** — manifest of resumed investigation review tasks.
- **resume-request.md** — advisory resume request instructions.
- **sync_manifest.json** — node-side task sync snapshot with schema `{metadata, files, complete}` where `files` maps filenames to `{sha, cut}`. Written at `bin/agent:7879–7882` in `node_sync_task`; read at `bin/agent:7516` in `node_sync_terminal_unchanged` to verify complete terminal snapshots without reading task artifacts (separate from log cursors stored in `bus/sync_log_cursors.json` at lines 7530, 7883).
- **acked** — empty marker file indicating a finished/terminal task has been acknowledged by human. Written at `bin/agent:8005` and `bin/agent:8012` by `cmd_ack`; removed at `bin/agent:4534` by `cmd__runner` when the next run starts.
- **long-run** — stores observed step count when reaching threshold to signal wrap-up or trigger fresh session. Written at `agentbus/normalize.go:62–71` by normalizer `countStep` upon reaching the soft step threshold `$AGENT_LONG_RUN_STEPS`, and at `bin/agent:4601` by `cmd__runner` as a hard-cut fallback when `$AGENT_LONG_RUN_HARD` steps are reached; consumed and removed at `bin/agent:6433–6435` by `long_run_end` to schedule a fresh-session wake-up; also cleared at `bin/agent:4534` on runner startup.
- **answers.lock** — exclusive lock serializing question saving, editing, and answers watch.
- **wake.lock** — exclusive lock serializing scheduled task wake-ups.
- **stop-requested** / **node-stop-requested** — lifecycle stop marker files.
- **stop.lock** — exclusive lock serializing investigation team cancellation.

## Event Streams (Distinct Contracts)

### 1. Activity Records (`activity.jsonl`)
Produced by `agentbus normalize` with schema `{ts, k, text}`:
- `text`: agent message
- `cmd`: executed shell command
- `edit`: file modification
- `mcp`: call to an agentbus tool
- `tool`: any third-party tool call
- `err`: error reported by CLI or tool
- `info`: runner metadata, step cost, model info

### 2. Bus Events (`bus/messages.jsonl`)
Inter-agent messages posted via `agentbus` with typed kinds:
- Chat & Coordination: `chat`, `quiz_question`, `consult_request`, `consult_reply`, `worker_request`, `checkpoint`, `stop_all`, `script`
- Reservations: `reserved`, `released`, `auto_released`, `forced_release`
- Team Lifecycle: `team_created`, `team_resumed`, `team_finished`, `coordinator_claimed`, `coordinator_takeover`, `coordinator_handover`, `excluded_notice`, `member_excluded`
- Board: `board_item_added`, `board_result`, `board_returned`
- Investigations & Reviews: `investigation_requested`, `investigation_promoted`, `investigation_started`, `investigation_resumed`, `investigation_waiting`, `investigation_requeued`, `investigation_finished`, `decision_recorded`, `review_queued`

### 3. Structured Log Records (`logs/agent.jsonl` & `runner.log`)
JSON objects emitted via `log_event()` with schema `{ts, level, component: "agent", event, pid, subcommand, task, ...}`.
There are **246 distinct structured log events** in `bin/agent`: 198 direct event names emitted via `log_info`, `log_warn`, and `log_error`, plus 48 additional events emitted exclusively through `try` and `jq_soft` helper wrappers (which log warnings via `log_warn` upon command failure or JSON parse error).

**Direct Log Events (198 names):**
`agents_gh_missing`, `agents_md_missing`, `answers_incomplete`, `answers_resume_failed`, `answers_saved`, `answers_sent`,
`answers_watch_failed`, `answers_watch_list_failed`, `answers_watch_stopped`, `archived`, `auth_check_failed`, `autopilot_merge_failed`,
`autopilot_merge_held`, `autopilot_merged`, `autopilot_next_round`, `autopilot_next_round_failed`, `autopilot_postmerge_gate_failed`, `autopilot_switched`,
`cli_finished`, `command_failed`, `dispatch_lock_timeout`, `editor_not_connected`, `editor_open_failed`, `editor_opened`,
`fix_pass_unit_failed`, `fresh_session`, `gate_worktree_left`, `git_identity`, `handover_commented`, `helper_remote_list_failed`,
`helper_remote_list_parse_failed`, `helper_start_failed`, `helper_starting`, `independent_run_finished`, `investigation_manifest_recovered`, `investigation_pr_lookup_failed`,
`investigation_precheck_interrupted`, `investigation_publication_check_failed`, `investigation_random_failed`, `investigation_seed_failed`, `investigation_seed_invalid`, `investigation_start_failed`,
`investigation_starting`, `job_origin_lookup_failed`, `long_run_cut`, `long_run_ended`, `long_run_wake_failed`, `member_grant_already_gone`,
`member_job_cancel_failed`, `member_job_orphaned`, `member_out`, `merge_fetch_failed`, `meta_lock_timeout`, `meta_write_failed`,
`node_agent_cli_missing`, `node_answer_applied`, `node_answer_rejected`, `node_answer_resume_failed`, `node_answer_stale`, `node_assignment_obsolete`,
`node_checkout_failed`, `node_controls_rejected`, `node_doorbell_already_gone`, `node_doorbell_lost`, `node_heartbeat_accepted`, `node_heartbeat_failed`,
`node_helper_get_failed`, `node_helper_prepare_failed`, `node_helper_prepare_storage_failed`, `node_helper_reply_invalid`, `node_helper_submit_failed`, `node_helper_submitted`,
`node_kick_failed`, `node_kick_rejected`, `node_kick_stale`, `node_launch_failed`, `node_remote_helpers_list_failed`, `node_remote_helpers_parse_failed`,
`node_reply_invalid`, `node_report_failed`, `node_sleep_interrupted`, `node_start_report_failed`, `node_started`, `node_stop_failed`,
`node_stopped`, `node_storage_failed`, `node_sync_accepted`, `node_sync_artifact_failed`, `node_sync_failed`, `node_sync_invalid_reply`,
`node_sync_log_oversized`, `node_sync_payload_oversized`, `node_sync_run_start_failed`, `node_sync_save_failed`, `node_sync_scan_failed`, `node_wake_failed`,
`node_wake_now_failed`, `notice_sent`, `notification`, `notification_failed`, `notification_state_failed`, `pause_timer_failed`,
`pr_untrusted_ignored`, `precheck_fetch_failed`, `precheck_no_base`, `precheck_no_head`, `precheck_unit_failed`, `precheck_worktree_failed`,
`precheck_worktree_left`, `prompt_by_file`, `prompt_file_exclude_failed`, `prompt_file_failed`, `questions_autopilot_bounced`, `questions_bounced`,
`questions_note_cleanup_failed`, `remote_answer_lock_failed`, `remote_answer_queued`, `remote_answer_rejected`, `remote_answer_stale`, `remote_member_cancel_failed`,
`remote_member_cancel_skipped`, `remote_member_grant_failed`, `remote_member_job_failed`, `remote_member_revoke_skipped`, `remote_member_started`, `remote_member_status`,
`remote_questions_cleanup_failed`, `remote_questions_expired`, `remote_questions_lock_failed`, `remote_questions_retry_failed`, `remote_questions_start_failed`, `restart_limit`,
`resume_abandoned`, `resume_failed`, `resume_postponed`, `resume_stash_retry`, `resume_wip_commit_failed`, `resume_worktree_remove_failed`,
`resumed`, `resumed_after_pause`, `review_findings`, `review_fix_pass`, `review_fix_resume_failed`, `review_fixes_exhausted`,
`review_pair_add_failed`, `review_pair_done`, `review_pair_lock_failed`, `review_pair_stale`, `review_pair_start_failed`, `review_pair_waiting`,
`review_precheck_failed`, `review_precheck_resume_failed`, `review_retry_failed`, `review_without_verdict`, `round_bounced`, `round_closed_with_problems`,
`round_pr_superseded`, `round_size_failed`, `run_abandoned`, `run_finished`, `run_interrupted`, `run_interrupted_by_restart`,
`run_started`, `run_stopped`, `runner_died`, `runner_start_failed`, `runner_starting`, `session_id_missing`,
`signing_key_unreadable`, `stash_apply_failed`, `stash_drop_failed`, `status_bounced`, `stop_all`, `task_cleanup_failed`,
`task_create_failed`, `task_created`, `task_discarded`, `task_kicked`, `task_paused`, `task_stopped`,
`team_abandoned`, `team_close_stop_failed`, `team_closed`, `team_finished`, `team_not_settled`, `team_read_failed`,
`team_waiting`, `transcript_bus_unreadable`, `transcript_failed`, `unit_stop_failed`, `verify_member_branch_missing`, `verify_pr_check_failed`,
`waiting_for_investigation`, `wake_by_hand`, `wake_deferred`, `wake_loop_detected`, `wake_now_requested`, `wake_requested`.

**Wrapper-Emitted Events via `try` and `jq_soft` (48 names):**
`activity_parse`, `activity_write_failed`, `answers_watch_reset_failed`, `answers_watch_stop_failed`, `auth_check_parse`,
`board_release_failed`, `cancel_pending_failed`, `checkpoint_copy_failed`, `checkpoint_read_failed`, `cli_result_parse`,
`discard_branch_failed`, `discard_worktree_failed`, `dispatch_failed`, `event_scan_failed`, `handover_comment_failed`,
`helper_finish_failed`, `helper_state_failed`, `inv_list_failed`, `investigation_finish_failed`, `investigation_phase_parse`,
`node_sync_cleanup_failed`, `node_sync_cursor_invalid`, `node_sync_jobs_invalid`, `node_sync_log_malformed`, `node_sync_manifest_invalid`,
`notice_failed`, `opencode_export_failed`, `opencode_export_parse`, `peers_parse`, `peers_unavailable`,
`pr_body_parse`, `pr_lookup_failed`, `pr_lookup_parse`, `pr_ready_failed`, `remote_member_revoke_failed`,
`remote_wake_failed`, `review_followup_failed`, `review_mention_failed`, `round_fetch_failed`, `round_pr_close_failed`, `round_pr_link_failed`,
`submodule_update_failed`, `team_close_failed`, `team_state_failed`, `team_update_failed`, `team_waiting_parse`,
`worktree_prune_failed`, `worktree_remove_failed`.

## Linux Locks

All interprocess locks in `bin/agent` are Linux `flock` locks. Below is the complete inventory of all 17 `flock` call sites in `bin/agent` plus `agentbus` `bus.lock`:

| Call Site | Lock File Path | FD | Arguments | Timeout | Acquisition Scope & Contention Policy |
|---|---|---|---|---|---|
| `bin/agent:471` | `$TASKS/$id/meta.env.lock` | 9 | `flock -w 30 9` | 30s | Exclusive metadata write lock in `meta_set`. Exits with code 1 on timeout after logging `meta_lock_timeout`. |
| `bin/agent:728` | `$AGENTS_HOME/remote-questions/$node/$task/ready.lock` | 7 | `flock -w 30 7` | 30s | Exclusive lock in `cmd__remote_questions_ready` serializing remote questions readiness and answer watch initialization. Returns 1 on timeout after logging `remote_questions_lock_failed`. |
| `bin/agent:779` | `$AGENTS_HOME/remote-questions/$node/$task/answers.lock` | 8 | `flock -w 30 8` | 30s | Exclusive lock in `remote_answers_saved` serializing remote answer validation and queueing to lead. Returns 1 on timeout after logging `remote_answer_lock_failed`. |
| `bin/agent:922` | `$AGENTS_HOME/notify-failures.jsonl.lock` | 9 | `flock -w 10 9` | 10s | Exclusive lock in `notify_failure_keep`. Exits with code 1 on contention timeout. |
| `bin/agent:1917` | `$AGENTS_HOME/reviews/<repo>-<PR-number>/pair.lock` | 9 | `flock -w 60 9` | 60s | Exclusive lock in `review_pair_done` ensuring exactly one reviewer records verdict/decision across slots A and B. Logs `review_pair_lock_failed` on timeout. |
| `bin/agent:2237` | `$TASKS/$id/wake.lock` | 8 | `flock -w 30 8` | 30s | Exclusive lock in `cmd__wake` serializing scheduled or message wake actions. Exits cleanly (0) on timeout. |
| `bin/agent:3329` | `$AGENTS_HOME/dispatch.lock` | `$resume_fd` | `flock "$resume_fd"` | None (blocking) | Blocking exclusive lock in `investigation_resume` held across PR validation and advisory review admission, including external `gh` and `git` subprocesses, until explicitly unlocked at line 3365. |
| `bin/agent:3365` | `$AGENTS_HOME/dispatch.lock` | `$resume_fd` | `flock -u "$resume_fd"` | None (release) | Explicit unlock in `investigation_resume` releasing dispatch lock descriptor. |
| `bin/agent:3916` | `$TASKS/$parent/stop.lock` | `$stop_fd` | `flock -n "$stop_fd"` | 0s (non-blocking) | Non-blocking lock in `investigation_team_stop`. If contended, returns 0 immediately as another cleanup owns cancellation. |
| `bin/agent:3926` | `$AGENTS_HOME/dispatch.lock` | 9 | `flock 9` | None (blocking) | Blocking barrier in subshell in `investigation_team_stop` waiting for any in-flight admission to finish before terminating team. |
| `bin/agent:4141` | `$AGENTS_HOME/dispatch.lock` | 9 | `flock -w 120 9` | 120s | Exclusive lock in `dispatch` serializing investigation admission. Logs `dispatch_lock_timeout` and exits 0 on timeout. |
| `bin/agent:4273` | `$AGENTS_HOME/helpers.lock` | 7 | `flock -w 60 7` | 60s | Exclusive lock in `dispatch_helpers` assigning queued worker helper requests. Exits 0 on timeout. |
| `bin/agent:4952` | `$AGENTS_HOME/task-ids.lock` | 10 | `flock -w 60 10` | 60s | Exclusive lock in `lock_task_ids` serializing task ID allocation across runs. Calls `die` on timeout. |
| `bin/agent:4957` | `$AGENTS_HOME/task-ids.lock` | 10 | `flock -u 10` | None (release) | Explicit unlock in `unlock_task_ids` releasing task ID allocation descriptor. |
| `bin/agent:5787` | `$TASKS/$id/answers.lock` | 8 | `flock -w 30 8` | 30s | Exclusive lock in `cmd__answers_saved` serializing answer validation and task resumption. Exits 0 on timeout. |
| `bin/agent:6628` | `$BUS_DIR/node-daemon.lock` | 8 | `flock -n 8` | 0s (non-blocking) | Non-blocking lock in `node_lock`. Held continuously across the entire node daemon polling loop. Calls `die` immediately if another daemon holds descriptor 8. |
| `bin/agent:7101` | `$TASKS/$tid/meta.env.lock` | 9 | `flock -s 9` | None (blocking shared) | Shared read lock in `node_scan_tasks` (`node_report`) reading `job_transitions.jsonl` safely against concurrent writers. Dies on failure. |
| `agentbus` | `$BUS_DIR/bus.lock` | Go FD | `flock` (exclusive) | None (blocking) | Exclusive interprocess file lock via `syscall.Flock(fd, syscall.LOCK_EX)` guarding `bus/state.json` and `bus/messages.jsonl` under exclusive lock during atomic transactions. |

**Descriptor & Contention Policy:** In Bash, file descriptors are inherited by child processes by default unless explicitly closed (`exec 10>&-` or `3>&- 7>&- 8>&- 9>&- 10>&-` before subprocess launches). While short-lived operations close descriptors before long-running subshells, specific long-lived processes retain locks by design: the node daemon holds `node-daemon.lock` throughout its polling loop, and `investigation_resume` holds `dispatch.lock` across GitHub API and Git subprocesses.

## Bats Test Baseline

- **Test Files:** 120 `.bats` files in `tests/unit/`
- **Test Cases:** 480 `@test` declarations verified at base `940f6fdc067069f92a8023c84ed1b5eb011b95ae` (and HEAD)
- **Classification:** 359 black-box cases (external CLI / side-effect contracts), 121 source-coupled cases (inspecting Bash functions/variables) across 49 files
- **Ledger:** All 480 test cases are inventoried 1:1 in `tests/agent-go-map.tsv` and validated via `tools/check-agent-test-map.sh`
- **Round Allocation:** All 480 cases assigned to implementation rounds R01–R56 under the working cap of 1,500 changed lines per round

---

**Parity Status:** In Round R01, all ledger rows are marked `planned`. No Go behavioral parity is claimed yet; `bin/agent` remains the active production orchestrator and immutable baseline oracle.
