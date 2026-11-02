# Step-by-Step Plan: Rewriting `bin/agent` in Go While Preserving the Bats Baseline

## 1. Executive Summary

`bin/agent` is currently an **8,523-line Bash script** containing **314 functions** that implements the `agent` CLI, the `gh` filter shim (`argv[0] == "gh"`), the transient `systemd --user` task runner and watchdog (`_runner`), team/round/review autopilot supervision (`_supervise`, `_precheck`, `_merge`, `_fixpass`, `_wake`), the 5-phase investigation team state machine (`_investigation-team`), multi-instance HTTP node/lead orchestration (`node`, `lead`, `jobs`, `nodes`, `ack`, `_remote-member`, `_remote-questions`), account authentication checks (`authenticated`, `_authcheck`), and task archiving (`archive`, `discard`).

Its behaviour is verified by **110 Bats test files** (`tests/unit/*.bats`) containing **421 `@test` cases** (plus 2 shared helper libraries: `tests/unit/helpers.bash` and `tests/unit/remote_control_homes.bash`).

This document provides a concrete, round-by-round engineering plan for other models to:
1. Rewrite `bin/agent` into a multi-file, standard-library-only Go package in `agent/` (`agent/team.go`, `agent/archive.go`, `agent/run.go`, `agent/authenticated.go`, etc.) following all `AGENTS.md` rules (`OpError` via `tools/errgen`, `tools/doccheck`, `.golangci.yml`, `staticcheck`, TDD RED → GREEN → REFACTOR, and the ≤ 3,000-line round limit).
2. Port every test case in `tests/unit/*.bats` to a 1-to-1 Go unit test in `agent/*_test.go`.
3. Keep all **110 existing `.bats` files completely unmodified and passing** (`make gate`) at every commit and after every round until the entire rewrite is complete.

---

## 2. Critical Constraints & Architectural Findings

### 2.1 How the Existing 110 Bats Files Interact with `bin/agent`

Before writing a single line of Go, every implementing model must understand **how `tests/unit/*.bats` and `Makefile` invoke `bin/agent`**, because `AGENTS.md` Rule 1 (*"Never modify existing tests"*) and the task prompt (*"Before everything is ready, bats the tests are baseline and they haven't been modified/deleted"*) forbid editing `tests/unit/**`.

An exhaustive scan of all 110 files in `tests/unit/*.bats` reveals **four distinct interaction modes**:

| Mode | Count of `.bats` files | How `bin/agent` is used | Impact if `bin/agent` is prematurely replaced by an ELF binary |
|---|---:|---|---|
| **1. CLI Execution** | **66 files** | Executed as `$HOME/.local/bin/agent <subcommand>` (copied via `install -m755 "$REPO_ROOT/bin/agent" "$HOME/.local/bin/agent"` in `helpers.bash`) or symlinked as `gh` (`ln -sf "$REPO_ROOT/bin/agent" .../gh`). | Works transparently with an ELF binary or a thin exec wrapper. |
| **2. Bash `source`** | **40 files** | Runs `source "$REPO_ROOT/bin/agent"` (or `source "$HOME/.local/bin/agent"`) inside a Bats test or subshell, then directly invokes ~55 internal Bash functions (e.g. `meta_set`, `prepare_job_task`, `fresh_session_due`, `round_open_prs`, `node_handle_reply`, `run_claude`), often overriding shell variables (`NODE_REPOS`, `NODE_STATE`, `AGENT_TYPE`) or mocking shell functions (`bus()`, `notify()`, `start_runner()`, `grep()`, `node_request()`) in that subshell. | `source "$REPO_ROOT/bin/agent"` fails immediately with `bash: bin/agent: cannot execute binary file` if `bin/agent` is a raw ELF binary. |
| **3. `sed` Function Extraction** | **4 files** | Uses `sed -n '/^<func>() {/,/^}/p' "$BATS_TEST_DIRNAME/../../bin/agent"` to extract raw Bash source lines for 5 specific functions (`submit_remote_job`, `github_repo_url`, `node_poll_remote_helpers`, `node_heartbeat_json`, `node_transition`) into a temp `.sh` file and sources it. | `sed` extracts empty files from an ELF binary, failing `job_attribution.bats`, `job_attribution_paths.bats`, `node_agents.bats`, and `node_report_agent.bats`. |
| **4. `make lint-shell`** | **`Makefile`** | `SHELL_SCRIPTS` in `Makefile` includes `bin/agent` and runs `bash -n $(SHELL_SCRIPTS)` and `shellcheck -S warning -x -P SCRIPTDIR $(SHELL_SCRIPTS)`. | `bash -n` and `shellcheck` fail on an ELF binary unless `SHELL_SCRIPTS` is updated when `bin/agent` ceases to be a shell script. |

### 2.2 Complete Inventory of Internal Functions Sourced or Extracted by Bats Tests

Every model working on a round must consult this table to know which `.bats` files call internal functions directly:

| `.bats` File | Mode | Internal `bin/agent` Functions Called or Mocked |
|---|---|---|
| `agent.bats` | `source` | `quota_reset_seconds`, `usage` (also mocks `bus`, `notice`, `notify`) |
| `auto_stage.bats` | `source` | `limit_kind` |
| `autopilot_merge.bats` | `source` | `merge_blockers` |
| `autopilot_review.bats` | `source` | `review_verdict`, `round_test_changes` |
| `findings.bats` | `source` | `findings_count` |
| `fresh_relative.bats` | `source` | `fresh_session_due` |
| `fresh_session.bats` | `source` | `fresh_session_due`, `long_run_end` |
| `github_format.bats` | `source` | `pr_format_issues` |
| `handover_failed.bats` | `source` | `handover_failed`, `meta_set`, `remote_member_task`, `remote_start_check` |
| `investigation_mcp_off.bats` | `source` | `codex_mcp_args`, `opencode_config` (mocks `is_independent_investigation`, `bus_args_json`) |
| `investigation_team_admission.bats` | `source` | `free_investigator_list` |
| `investigation_team_advisory_resume.bats` | `source` | `investigation_resume_tick`, `investigation_team_tick`, `meta_set` |
| `investigation_team_cli.bats` | `source` | `meta_set`, `task_field` |
| `investigation_team_handoff.bats` | `source` | `investigation_handoff`, `investigation_team_tick`, `meta_set` |
| `investigation_team_isolation.bats` | `source` | `meta_load`, `opencode_config`, `run_claude`, `run_codex`, `run_gemini` (mocks `watch_run`) |
| `investigation_team_opencode_schema.bats` | `source` | `meta_load`, `opencode_config` |
| `investigation_team_phase_queue.bats` | `source` | `investigation_pipeline_tick`, `investigation_team_tick`, `meta_set` |
| `investigation_team_phase_selection.bats` | `source` | `investigation_phase_choose`, `investigation_phase_pick`, `meta_set` |
| `investigation_team_publication_retry.bats` | `source` | `investigation_phase1_recover`, `investigation_published`, `meta_set` |
| `investigation_team_reboot_regression.bats` | `source` | `restart_interrupted`, `meta_set` |
| `investigation_team_recovery_regressions.bats` | `source` | `free_investigator_list`, `investigation_team_tick`, `meta_set` |
| `investigation_team_regressions.bats` | `source` | `investigation_team_tick`, `meta_set` |
| `investigation_team_resume_boundaries.bats` | `source` | `investigation_resume`, `investigation_resume_admit`, `meta_set` |
| `investigation_team_resume_recovery.bats` | `source` | `investigation_resume_tick`, `meta_set` |
| `investigation_team_runtime.bats` | `source` | `free_investigator`, `free_investigator_list`, `meta_set`, `start_review_pair` |
| `investigation_team_stdin.bats` | `source` | `validate_investigation_topic` |
| `investigation_team_supervision.bats` | `source` | `investigation_teams_tick`, `meta_set` |
| `investigation_team_validation.bats` | `source` | `validate_investigation_team`, `validate_investigation_topic` |
| `job_attribution.bats` | **`sed`** | Extracts `submit_remote_job`, `github_repo_url`; mocks `lead_url`, `bus`, `log_warn`, `die` |
| `job_attribution_paths.bats` | **`sed`** | Extracts `github_repo_url`, `node_poll_remote_helpers`; mocks `node_request`, `bus`, `log_warn` |
| `long_run_cut.bats` | `source` | `watch_run` |
| `meta.bats` | `source` | `meta_set` |
| `network.bats` | `source` | `limit_kind`, `quota_reset_seconds` |
| `node_activity.bats` | `source` | `node_heartbeat_json`, `node_job_result` |
| `node_agents.bats` | **`sed`** | Extracts `node_heartbeat_json`; mocks `is_investigator`, `agent_tier`, `agent_cli_present`, `agent_running_task`, `node_activity_json` |
| `node_cancel_exit.bats` | `source` | `meta_load`, `meta_set`, `node_handle_stop`, `prepare_job_task`, `runner_exit`, `task_field` |
| `node_checkout.bats` | `source` | `create_task`, `meta_set`, `prepare_job_task` (one test mocks `grep` to test `meta_set` read-fault) |
| `node_daemon.bats` | `source` | `node_jobs_load`, `prepare_job_task`, `task_field` |
| `node_prelaunch_cancel.bats` | `source` | `meta_set`, `node_jobs_save`, `node_process_pending_launches`, `prepare_job_task`, `task_field` (mocks `start_runner`) |
| `node_recovery_fence.bats` | `source` | `cmd_stop`, `meta_file`, `meta_set`, `node_jobs_load`, `node_jobs_save`, `node_process_pending_launches`, `node_recover_tasks`, `node_save`, `node_scan_tasks`, `node_start_ack`, `prepare_job_task`, `task_field` (mocks `start_runner`, `unit_active`) |
| `node_report_agent.bats` | **`sed`** | Extracts `node_transition`; mocks `node_time`, `node_save` |
| `node_review_boundaries.bats` | `source` | `agent_running_task`, `cmd_stop`, `meta_file`, `meta_set`, `node_jobs_file`, `node_jobs_save`, `node_process_pending_launches`, `node_recover_tasks`, `node_scan_tasks`, `prepare_job_task`, `stopping`, `task_field` (mocks `start_runner`, `unit_active`) |
| `node_safety.bats` | `source` | `agent_running_task`, `node_jobs_load`, `node_lock`, `prepare_job_task`, `task_field` |
| `one_pr_per_round.bats` | `source` | `round_open_prs` |
| `prompt_checkpoints.bats` | `source` | `checkpoints_latest` |
| `remote_control_cancel_order.bats` | `source` | `node_handle_reply` (mocks `node_handle_stop`, `node_apply_answer`) |
| `remote_control_heartbeat_limits.bats` | `source` | `node_apply_answer`, `node_clear_control_acks`, `node_flush_reports`, `task_field` (mocks `node_heartbeat_json`, `node_request`) |
| `remote_control_kick_exit.bats` | `source` | `cmd_kick`, `meta_set`, `runner_exit`, `task_field` (mocks `notify`, `notice`, `team_lead`, `team_after_run`, `dispatch`, `dispatch_helpers`) |
| `remote_control_notifications.bats` | `source` | `notify` (mocks `notify_send`) |
| `remote_control_review.bats` | `source` | `cmd_answer`, `cmd_kick`, `handover_failed`, `meta_set`, `node_apply_kick`, `runner_exit`, `task_field` (mocks `stop_answers_watch`) |
| `remote_control_validation.bats` | `source` | `meta_set`, `node_apply_answer`, `node_apply_kick`, `node_clear_control_acks`, `node_handle_controls`, `stopping`, `supervise_remote_questions`, `task_field` |
| `remote_fixes.bats` | `source` | `wake_loop_applies` |
| `review_byline.bats` | `source` | `review_byline` (and symlinks `bin/agent` as `gh`) |
| `review_pair_random.bats` | `source` | `start_review_pair` (mocks `free_investigator_list`, `start_review_slot`) |
| `review_resilience.bats` | `source` | `free_investigator_list` |
| `rotation.bats` | `source` | `free_investigator`, `pick_investigator`, `remember_investigator` |
| `round_handover.bats` | `source` | `merge_blockers`, `meta_set`, `round_pr_supersede` |
| `round_only.bats` | `source` | `round_text_strip` |
| `run_notes.bats` | `source` | `limit_kind`, `node_job_result`, `quota_reset_seconds` |
| `skips.bats` | `source` | `skips_in_repo` |
| `wake_cmd.bats` | `source` | `node_handle_reply` |

### 2.3 The Strangler-Fig Migration Mechanism (Keeping Bats 100% Green Across Rounds)

Because a PR can touch at most **3,000 lines** (aiming for **2,000 lines**, per `AGENTS.md` "Rounds") and `bin/agent` is 8,523 lines long, the rewrite must be executed across multiple self-contained rounds where **`make gate` (`make lint test`) passes after every round**.

Here is the exact mechanism to migrate incrementally while keeping all 110 unmodified `.bats` tests passing:

1. **Build `agent/` to `build/agent-go` and expose it on `PATH` in `Makefile`:**
   - Create the new Go module in `agent/go.mod` (`module agent`, `go 1.22`, standard library only).
   - In `Makefile`:
     - Extend `build` to compile `cd agent && go build -o ../build/agent-go .` alongside `build/agentbus` and `build/agentlive`.
     - In `unit-bats`, pass `PATH="$(CURDIR)/build:$$PATH" AGENT_GO_BIN="$(CURDIR)/build/agent-go" bats tests/unit`.
     - **Why this works without modifying `tests/unit/helpers.bash`:** `helpers.bash` and `remote_control_homes.bash` both set `PATH="$HOME/.local/bin:$STUBS_DIR:$PATH"`, preserving the parent `PATH`! Putting `$(CURDIR)/build` on `PATH` in `Makefile` makes `agent-go` automatically discoverable inside every isolated test `$HOME` (including multi-home tests like `node_two_homes.bats` and `delegate_jobs.bats`) with zero edits to `tests/unit/**`.
2. **Subcommand & `gh` Delegation in `bin/agent` During Incremental Rounds:**
   - When `bin/agent` is invoked as `gh` (`[[ $(basename "$0") == "gh" ]]`), once Round 2 (`agent/gh.go`) lands, `bin/agent` immediately runs:
     ```bash
     if [[ $(basename "$0") == "gh" ]]; then
         if _go_bin=$(command -v "${AGENT_GO_BIN:-agent-go}"); then
             exec "$_go_bin" _gh "$@"
         fi
         agent_gh "$@"
     fi
     ```
   - In `main()` at the bottom of `bin/agent`, maintain an explicit allowlist of subcommands already ported to Go in completed rounds:
     ```bash
     main() {
         local cmd=${1:-status}
         case $cmd in
             # Add each subcommand here in the round that implements it in Go:
             authenticated|_authcheck|tokens|notes|helpers|transcript|bus|say|discard|archive|...)
                 if _go_bin=$(command -v "${AGENT_GO_BIN:-agent-go}"); then
                     exec "$_go_bin" "$@"
                 fi
                 ;;
         esac
         # ... existing bash case dispatch for unported subcommands ...
     }
     ```
   - Whenever `agent-go` spawns background systemd units (`systemd-run ... "$SELF" _runner ...`) or links `gh` (`ln -sf "$SELF" "$gh_dir/gh"`), `SELF` resolves to the `agent` binary/script in `$HOME/.local/bin/agent` (or `agent-go` directly once all subcommands are ported), so all Bats stubs (`tests/unit/stubs/systemd-run`, `systemctl`, `claude`, `codex`, `gemini`, `opencode`, `gh`) intercept calls identically to how they intercept `bin/agent` today.
3. **Handling `source "$REPO_ROOT/bin/agent"` and `sed` During Incremental Rounds and Final Cutover:**
   - While rounds are in progress, `bin/agent` continues to guard its entrypoint with:
     ```bash
     if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
         main "$@"
     fi
     ```
   - Every round writes **native Go unit tests (`agent/<domain>_test.go`)** that test the Go functions directly in Go (mirroring the `.bats` tests 1-to-1, including all edge cases previously tested via `source bin/agent` or `sed`), while the CLI subcommands in `.bats` are executed through `agent-go` via `main()`.
   - For pure internal functions called via `source bin/agent` that do not depend on caller-redefined Bash mocks (e.g., `round_text_strip`, `quota_reset_seconds`, `limit_kind`, `findings_count`, `pr_format_issues`, `review_byline`, `skips_in_repo`, `validate_investigation_team`, `validate_investigation_topic`), their Bash bodies in `bin/agent` can be replaced with one-line bridges:
     ```bash
     round_text_strip() { "${AGENT_GO_BIN:-agent-go}" _fn round_text_strip "$@"; }
     ```
   - For functions that `.bats` tests extract via `sed` (`submit_remote_job`, `github_repo_url`, `node_poll_remote_helpers`, `node_heartbeat_json`, `node_transition`) or call after mocking internal Bash functions in the same subshell, keep those Bash function definitions in the sourced section of `bin/agent` while `tests/unit/*.bats` remains unedited.
   - **Final Cutover (Round 14 — Two Options Depending on Human Sign-Off):**
     - **Option 14A (Zero-Test-Edit Cutover — Default under Rule 1):** `agent/` compiles to `build/agent-go` (installed to `~/.local/bin/agent-go` by `install.sh` and `Makefile`). When `bin/agent` is executed (`[[ "${BASH_SOURCE[0]}" == "$0" ]]`), it unconditionally `exec`s `agent-go "$@"` for 100% of CLI commands and `gh`. All 8,523 lines of CLI execution run in Go, all 110 `.bats` files have 1-to-1 Go unit tests in `agent/*_test.go`, and the 44 `source`/`sed` `.bats` files still pass without modifying `tests/unit/**`.
     - **Option 14B (Pure Compiled ELF Binary at `bin/agent` — Requires Human Approval per Rule 1):** `Makefile` builds `cd agent && go build -o ../bin/agent .` and removes `bin/agent` from `SHELL_SCRIPTS` and git tracking. Because Bash cannot `source` or `sed`-extract functions from an ELF binary, executing Option 14B requires asking the human (in the questions file or PR description) for permission to either (i) retire the 44 `source`/`sed` `.bats` files now covered by `agent/*_test.go` while keeping the 66 CLI `.bats` files, or (ii) point those 44 `.bats` files at a test-only compatibility shim. **Do not modify `tests/unit/**` for Option 14B without explicit human approval.**

### 2.4 Pre-Existing Baseline Conflict in `tests/unit/` on `main` (`f6a6024`)

> [!WARNING]
> Running the full `make gate` suite against the unmodified base commit `f6a602465a7d30e4b6a6737515e67e056f757a05` passes `check-generated`, `lint-shell`, `lint-go`, `lint-docs`, `unit-go`, and **419 of 421** Bats tests, but fails on **2 Bats tests** due to a direct contradiction between commit `f6a6024` (`tests/unit/investigation_mcp_off.bats`) and two earlier test files:
> 1. **`tests/unit/investigation_team_isolation.bats` (line 91, test 122):** Asserts that `run_codex` for an investigation team task does **not** pass `mcp_servers.agentbus.command` (`run grep -q "mcp_servers\.agentbus\.command" "$STUB_CLI_LOG"; [ "$status" -ne 0 ]`), whereas `tests/unit/investigation_mcp_off.bats` (line 13, added in `213db63`/`f6a6024`) asserts that `codex_mcp_args` (called by `run_codex`) **must** emit `mcp_servers.agentbus.command=...` alongside `mcp_servers.agentbus.enabled=false`.
> 2. **`tests/unit/investigation_team_opencode_schema.bats` (line 45, test 125):** Asserts that `opencode_config` for an independent investigation outputs `.mcp.agentbus | keys == ["enabled"] and .enabled == false` (no `type` or `command`), whereas `tests/unit/investigation_mcp_off.bats` (lines 27–29, added in `213db63`/`f6a6024`) asserts that `.mcp.agentbus` **must** include `.type == "local"` and `.command | length >= 1` alongside `.enabled == false`.
>
> **Action Required Before Round 5 (`agent/cli.go`):** Per `AGENTS.md` Rule 1, implementing models must **not** modify either test file on their own. The human should clarify whether `investigation_team_isolation.bats:91` and `investigation_team_opencode_schema.bats:45` should be updated to match `f6a6024` (`investigation_mcp_off.bats`) or vice versa.

---

## 3. Go Package Architecture & Repository Conventions

### 3.1 Directory Layout (`agent/`)

Create a new Go module in `agent/` (`module agent`, `go 1.22`, **standard library only** — Rule 8 forbids adding third-party Go dependencies without human approval). Split `agent/` into focused domain files matching the prompt's design:

| File | Responsibility | Lines in `bin/agent` | Key Types & Functions |
|---|---|---:|---|
| `agent/go.mod` | Go module definition (`module agent`, `go 1.22`) | — | Standard library only |
| `agent/errors.json` | Error specification for `tools/errgen` | — | Per-operation error causes for every `agent` subcommand and internal operation |
| `agent/errors.go` | Error model (`OpError[C]`, `ErrorClass`, `*StorageError`, `*ValueError`, `*ProcessError`) | — | Identical pattern to `agentbus/errors.go`; `//go:generate` directive invoking `../tools/errgen` |
| `agent/errors_gen.go` | Generated per-operation error types (`RunError`, `ArchiveError`, etc.) | — | Generated by `make generate` (never edit by hand — Rule 5) |
| `agent/types.go` | Strongly typed enums and value types | — | `TaskStatus`, `RunMode`, `AutoStage`, `PairStage`, `LimitKind`, `Provider`, `Tier`, `LogEvent`, `Timestamp` with `Valid()`, `parseEnum`, `UnmarshalJSON` |
| `agent/logging.go` | Structured JSONL logging & `try` side-effect logging | 202–285 | `logInfo`, `logWarn`, `logError`, `warnSideEffect`, `cliWriter`; writes `{ts, level, component: "agent", event, ...}` to `logs/system.jsonl` |
| `agent/config.go` | Configuration (`agents.conf`), `secrets.env`, paths, and runtime environment seam (`Env`) | 286–462 | `Config`, `Env`, `LoadConfig` (parses Bash `VAR=val`, `declare -A AGENT_TYPE=([k]=v)`, `NORMAL_AGENTS=(a b)`), `agentProvider`, `agentTier`, `investigatorNames`, `isInvestigator`, `stopping` |
| `agent/bus.go` | Typed wrapper around `agentbus` CLI subcommands and exit codes | 436–453, 886–896 | `BusClient`, `BusExitCode` (`BusInvalidArg=2` .. `BusProcess=10`), typed calls for `team-get`, `inv-next`, `helper-next`, `post`, `status`, `checkpoints`, etc. |
| `agent/meta.go` | Task `meta.env` persistence under `flock`, ID resolution, `systemd` unit names/activity | 463–565 | `TaskMeta`, `metaFile`, `metaSet` (locks `meta.env.lock`, writes Bash-compatible `KEY=VALUE`, appends `job_transitions.jsonl`), `metaLoad`, `taskField`, `resolveID`, `unitName`, `unitActive`, `agentRunningTask` |
| `agent/gh.go` | Trusted `gh` CLI wrapper (`argv[0] == "gh"`) and PR/issue filtering | 9–200, 1189–1236 | `runAgentGH`, `ghAPIFilter`, `injectReviewByline`, `reviewByline`, `agentsGHEnv`, `githubRepoURL`, `githubOwner`, `githubTrusted` |
| `agent/questions.go` | Question file parsing, answer validation, editor watching, remote question staging | 566–853 | `countQuestions`, `validateAnswers`, `questionsMissingRecommendations`, `expandAnswers`, `answersMarked`, `questionsNote`, `questionsReady`, `cmdRemoteQuestions`, `cmdRemoteQuestionsReady`, `superviseRemoteQuestions`, `watchAnswers`, `stopAnswersWatch`, `cmdAnswer`, `cmdAnswersSaved` |
| `agent/prompt.go` | Prompt template rendering (`{{PLACEHOLDERS}}`), git identity, commit trailers, checkpoints | 854–885, 2494–2779, 6340–6410 | `renderPrompt`, `buildPrompt`, `gitIdentity`, `commitTrailers`, `busArgsJSON`, `checkpointsLatest`, `roundTextStrip` |
| `agent/notify.go` | Desktop (`notify-send`), mail/push, and bus `notice` notifications | 946–981, 3055–3154 | `notify`, `notifySend`, `notifyFailureKeep`, `notice`, `pushText` |
| `agent/limits.go` | Provider rate-limit/quota detection, reset duration parsing, wake-loop guard, pause/resume | 2273–2493 | `limitKind`, `limitLine`, `quotaResetSeconds`, `durationSeconds`, `wakeLoopApplies`, `wakeLoopCheck`, `scheduleWake`, `pauseTask`, `resumePaused` |
| `agent/cli.go` | Headless CLI launchers (`claude`, `codex`, `gemini`, `opencode`) & MCP isolation | 2905–3054 | `runClaude`, `codexMCPArgs`, `runCodex`, `runGemini`, `opencodeConfig`, `runOpencode`, `isIndependentInvestigation`, `cliPrompt` |
| `agent/runner.go` | Background task runner (`_runner`), process-group watchdog (`watch_run`), verification loop, final status extraction | 2780–2904, 4299–4889 | `cmdRunner`, `runnerExit`, `startRunner`, `watchRun`, `activityProgress`, `killTree`, `plCommits`, `verifyGaps`, `finalStatus`, `questionsFromNoStatus`, `questionsFromFinal`, `freshSessionDue`, `longRunEnd` |
| `agent/rounds.go` | Round diff-size calculation, skipped-test enforcement, PR format checks, PR superseding | 982–1340, 1450–1560 | `roundLimit`, `roundLimitText`, `roundSoft`, `roundBaseRef`, `skipsInRepo`, `skipsAdded`, `prSkippedSection`, `roundCheck`, `cmdRoundSize`, `prFormatIssues`, `roundOpenPRs`, `roundPROfTeam`, `roundPRSupersede` |
| `agent/autopilot.go` | Autopilot review pairs, prechecks, fix passes, merge blockers, and `_merge`/`_precheck`/`_fixpass` | 1237–2099 | `mergeBlockers`, `findingsCount`, `reviewVerdict`, `reviewTaskVerdict`, `roundTestChanges`, `cmdMerge`, `cmdAutopilot`, `superviseReviewWaits`, `startPrecheck`, `cmdPrecheck`, `reviewFixPass`, `cmdFixpass`, `freeInvestigatorList`, `startReviewPairs`, `startReviewPair`, `startReviewSlot`, `requestReview`, `handoverFailed` |
| `agent/team.go` | Team lifecycle, post-run hooks (`team_after_run`), `_wake`, `team` CLI, `team close`, remote member spawning | 897–945, 2100–2271, 6100–6276, 8153–8265 | `teamJSON`, `teamLead`, `teamDoneFlag`, `teamTasks`, `teamSummary`, `finishTeam`, `teamAfterRun`, `cmdWakeInternal`, `cmdTeam`, `teamClose`, `remoteMemberTask`, `cmdRemoteMember`, `remoteMemberStart`, `memberOut` |
| `agent/investigate.go` | Single-agent investigations, investigator rotation (`last-investigator`), `inv` and `dispatch` CLIs | 3155–3230, 4050–4208, 5970–6020 | `pickInvestigator`, `rememberInvestigator`, `freeInvestigator`, `dispatchInvestigations`, `startInvestigation`, `invFinish`, `maybeResumeWorker`, `cmdInv`, `cmdDispatch` |
| `agent/investigation_team.go` | 5-phase investigation team state machine (`_investigation-team`), admission, leases, handoff, recovery | 3231–4049, 5480–5545 | `cmdInvestigationTeam`, `investigationTeamsTick`, `investigationTeamTick`, `investigationPhaseStart`, `investigationPhaseChoose`, `investigationPhasePick`, `investigationHandoff`, `investigationResumeTick`, `investigationPhase1Recover`, `validateInvestigationTeam`, `validateInvestigationTopic` |
| `agent/helpers.go` | Worker helper requests (`request_worker`), `dispatch_helpers`, `start_helper`, `helper_finish`, `helpers` CLI | 4209–4298, 8280–8310 | `freeWorker`, `dispatchHelpers`, `startHelper`, `helperFinish`, `cmdHelpers` |
| `agent/run.go` | Task ID allocation under lock, worktree creation, remote job preparation (`prepare_job_task`), `run` CLI | 4890–5330, 5546–5685 | `lockTaskIDs`, `allocateRunIDs`, `createTask`, `dropUnstartedTask`, `jobRefusal`, `prepareJobTask`, `discardTask`, `cmdRun` |
| `agent/resume.go` | Task & team resumption (`resume_run`), stash handling, `resume`, `wake`, `kick` CLIs | 5331–5479, 5740–5815, 6460–6515 | `abandonRun`, `resumeHistory`, `resumeRun`, `cmdResume`, `cmdWake`, `cmdKick` |
| `agent/status.go` | User-facing inspection & lifecycle CLIs: `status`, `show`, `log`, `stop`, `clean`, `agents`, `notes`, `bus`, `say`, `live`, `web` | 5816–6045, 6516–6534, 8266–8505 | `cmdStatus`, `cmdShow`, `renderLog`, `cmdLog`, `stopAll`, `cmdStop`, `cmdClean`, `cmdAgents`, `cmdNotes`, `cmdBus`, `cmdSay`, `cmdLive`, `cmdWeb` |
| `agent/supervise.go` | Central supervisor (`_supervise`), boot-ID interrupted-run recovery (`restart_interrupted`) | 6046–6210 | `cmdSupervise`, `superviseMemberJobs`, `superviseTeams`, `bootID`, `restartInterrupted` |
| `agent/tokens.go` | Token usage aggregation and `tokens` CLI | 6277–6339, 6411–6459 | `cmdTokens`, `parseTokenTotals` |
| `agent/transcript.go` | Session transcript rendering (`transcript` CLI) | 8311–8365 | `cmdTranscript` |
| `agent/archive.go` | Task archiving (`7z`), `archive.jsonl` indexing, `discard` and `archive` CLIs | 8366–8485 | `archiveIndex`, `archiveGroup`, `sevenZip`, `archiveOne`, `cmdDiscard`, `cmdArchive` |
| `agent/authenticated.go` | Provider authentication probing (`authenticated` and `_authcheck` CLIs) | 7958–8151 | `authClassify`, `apiErrorText`, `agentAuthTarget`, `cmdAuthcheck`, `authProbe`, `opencodeModelFor`, `cmdAuthenticated`, `authProviders` |
| `agent/node.go` | Multi-instance node daemon (`node`), `lead` HTTP proxy/launcher, `jobs`, `nodes`, `_agents-json`, `ack` CLIs | 6536–7350, 7700–7957 | `cmdLead`, `cmdJobs`, `cmdNodes`, `cmdNode`, `cmdAgentsJSON`, `cmdAck`, `submitRemoteJob`, `nodeLock`, `nodePoll`, `nodeHeartbeatJSON`, `nodeTransition`, `nodeRecoverTasks`, `nodeProcessPendingLaunches`, `nodeHandleStop`, `nodeJobResult` |
| `agent/sync.go` | Node-to-lead artifact, log tail, and state synchronization | 7351–7550 | `nodeSyncTasks`, `nodeSyncTaskPayload`, `nodeFlushReports`, `nodeScanTasks` |
| `agent/remote_control.go` | Remote answers, kicks, wakes, and control ACKs on worker nodes | 7551–7699 | `nodeHandleReply`, `nodeHandleControls`, `nodeApplyAnswer`, `nodeApplyKick`, `nodeClearControlAcks` |
| `agent/main.go` | Entrypoint (`main`), `argv[0] == "gh"` check, CLI subcommand & `_fn` dispatcher, `usage` | 8506–8523 | `main`, `dispatchCLI`, `dispatchInternalFn`, `usage` |

### 3.2 Dependency-Injected Runtime Seam (`Env`) for Fast, Deterministic Go Unit Tests

In `agent/config.go`, define an `Env` struct passed to all operations instead of reading global process state directly:

```go
// Env holds the filesystem paths, configuration, clock and external process runner for one agent invocation.
type Env struct {
	Home       string            // user home directory ($HOME)
	AgentsHome string            // root state directory ($AGENTS_HOME, default $HOME/agents)
	ConfPath   string            // path to agents.conf ($CONF)
	Secrets    string            // path to secrets.env ($SECRETS)
	RepoRoot   string            // repository root when running from a checkout
	SelfBin    string            // path to the current agent executable ($0)
	Environ    map[string]string // process environment variables
	Cfg        *Config           // parsed configuration and defaults
	Now        func() time.Time  // clock (time.Now in production, deterministic clock in tests)
	Exec       Runner            // executes external binaries (git, systemd-run, systemctl, agentbus, 7z, gh, CLIs)
	HTTP       *http.Client      // HTTP client for node/lead requests
	Stdout     io.Writer         // standard output stream
	Stderr     io.Writer         // standard error stream
	Stdin      io.Reader         // standard input stream
}
```

This single design choice gives **two testing modes** in `agent/*_test.go`:
1. **In-Process Unit Tests (Microseconds):** Inject a `FakeRunner` or temp directory with stub state (`meta.env`, `state.json`) and a fake clock (`Now`), testing every branch and `OpError` cause (`failsWith(t, RunInvalidAgent)(...)`) without spawning external processes or sleeping.
2. **Stub-CLI Integration Tests in Go (Milliseconds):** Point `Env.Home` at `t.TempDir()`, prepend `../tests/unit/stubs` and `../build` to `PATH` (just like `tests/unit/helpers.bash` does!), and run the real `os/exec` `Runner` against the exact same stub CLIs (`tests/unit/stubs/claude`, `codex`, `gemini`, `opencode`, `gh`, `systemctl`, `systemd-run`, `7z`, `notify-send`) that the Bats suite uses.

### 3.3 Native `agents.conf` & `meta.env` Compatibility (Rule 9)

Existing installations and all 110 `.bats` tests write `agents.conf` and `meta.env` using Bash syntax:
- `agents.conf` contains:
  - Simple assignments: `MAX_RUNS=25`, `VERIFY_CMD="make gate"`, `LEAD_LISTEN=${LEAD_LISTEN:-127.0.0.1:0}`
  - Indexed arrays: `NORMAL_AGENTS=(claude-sonnet codex)`, `INVESTIGATORS=(inv-a inv-b)`
  - Associative arrays: `declare -A AGENT_TYPE=([claude-sonnet]=claude [inv-a]=gemini)` and `declare -A NODE_REPOS=([team-agents]="$HOME/repo")`
- `meta.env` contains `KEY=VALUE` lines written by `printf '%s=%q\n'` (Bash `$'...'` or backslash-escaped strings) or unquoted/single-quoted/double-quoted values written by test helpers.

**Rule for `agent/config.go` and `agent/meta.go`:**
1. Implement a dedicated Bash-compatible parser in `agent/config.go` (`parseBashConfig`) and `agent/meta.go` (`parseMetaEnv` / `formatBashQuoted`) that handles:
   - Single-quoted `'...'`, double-quoted `"..."` (with `\$`, `\"`, `\\`, and `$HOME` / `${VAR:-default}` variable expansion), ANSI-C `$'...'` quoting (`\n`, `\t`, `\'`, `\\`, `\xNN`), and backslash-escaped unquoted tokens (output of Bash `printf '%q'`).
   - Indexed arrays `NAME=(elem1 elem2 ...)` across single or multiple lines.
   - Associative arrays `declare -A NAME=([k1]=v1 [k2]=v2 ...)` and `NAME[k]=v`.
   - Comments (`# ...`) and blank lines.
2. When a `.bats` test uses arbitrary dynamic Bash code inside `agents.conf` (if any), fallback cleanly or support all constructs used across `config/agents.conf` and `tests/unit/*.bats`. (Verify in Round 1 against every `agents.conf` fixture in `tests/unit/*.bats`!)
3. `metaSet` in `agent/meta.go` **must**:
   - Acquire an exclusive `syscall.Flock(fd, syscall.LOCK_EX)` on `<task_dir>/meta.env.lock` before reading/modifying `meta.env`.
   - Write to `<task_dir>/meta.env.tmp.<pid>` and `os.Rename` atomically to `<task_dir>/meta.env`.
   - If `STATUS` is being updated, `JOB` is non-empty in `meta.env`, and the previous `STATUS` differs from the new `STATUS`, atomically append a JSON line `{"job":...,"status":...,"at":...,"task_id":...,"agent":...}` to `$AGENTS_HOME/bus/job_transitions.jsonl` under `meta.env.lock` — matching lines 476–506 of `bin/agent`.

---

## 4. Step-by-Step Round Plan (14 Rounds)

Each round below is sized between **1,200 and 2,200 lines** (code + tests combined, excluding generated `errors_gen.go`), keeping every PR well within the 3,000-line hard limit (`agent round-size <task>`).

Every round follows the strict TDD workflow required by `AGENTS.md` and `docs/tdd.md`:
1. **Update `agent/errors.json` and run `make generate`** to generate the round's `OpError` cause enums in `agent/errors_gen.go`.
2. **RED Commit:** Write the stub signatures in `agent/<file>.go` returning a typed error or empty result, and write the comprehensive Go unit tests in `agent/<file>_test.go` (porting the corresponding `.bats` tests 1-to-1). Run `cd agent && go test ./...` to confirm the new tests fail as expected, and commit with the failing test output in the commit body.
3. **GREEN Commit:** Implement the functions in `agent/<file>.go` and wire the subcommand(s) in `agent/main.go` and `bin/agent`. Run `cd agent && go test -race ./...` and `make gate` (which runs all 110 `.bats` tests!).
4. **REFACTOR Commit (if needed):** Clean up duplication while keeping all Go and Bats tests green.

---

### Round 1: Foundation — Go Module, Error Model, Types, Logging, Config, Bus Client, and `meta.env`

- **Target Files to Create/Modify:**
  - `agent/go.mod`
  - `agent/errors.json`, `agent/errors.go`, `agent/errors_gen.go`
  - `agent/types.go`
  - `agent/logging.go`
  - `agent/config.go`
  - `agent/bus.go`
  - `agent/meta.go`
  - `agent/main.go` (minimal CLI + `_fn` dispatcher for `meta_set`, `meta_load`, `task_field`)
  - `agent/testhelpers_test.go`, `agent/config_test.go`, `agent/meta_test.go`
  - `Makefile` (add `agent` to `generate`, `build`, `lint-go`, `lint-docs`, `unit-go`, and export `PATH="$(CURDIR)/build:$$PATH" AGENT_GO_BIN="$(CURDIR)/build/agent-go"` in `unit-bats`)
- **Functions Ported from `bin/agent` (Lines 202–565):**
  - Logging: `log_event`, `log_info`, `log_warn`, `log_error`, `try`, `jq_soft`, `die`
  - Config: `agents.conf` and `secrets.env` loader, `agent_provider`, `agent_tier`, `investigator_names`, `is_investigator`, `stopping`, `bus`
  - Metadata: `meta_file`, `meta_set`, `meta_load`, `task_field`, `resolve_id`, `unit_name`, `unit_active`, `agent_running_task`
- **Bats Tests Ported to Go (`agent/meta_test.go`, `agent/config_test.go`):**
  - `tests/unit/meta.bats` (concurrent `meta_set` under `flock` without lost updates, `job_transitions.jsonl` emission, read-fault handling from `node_checkout.bats`).
  - All `agents.conf` parsing patterns across `tests/unit/*.bats` (`AGENT_TYPE`, `NORMAL_AGENTS`, `INVESTIGATORS`, `WORKER_AGENTS`, `NODE_REPOS`, `printf '%q'` round-tripping).
- **What to Check / How to Run Tests:**
  - `make generate` — verifies `tools/errgen` generates `agent/errors_gen.go` cleanly.
  - `cd agent && go test -race -v -run 'TestMeta|TestConfig' ./...`
  - `bats tests/unit/meta.bats tests/unit/gate.bats`
  - `make gate`

---

### Round 2: Trusted GitHub Filter (`agent/gh.go`) and Provider Auth (`agent/authenticated.go`)

- **Target Files to Create/Modify:**
  - `agent/gh.go`, `agent/gh_test.go`
  - `agent/authenticated.go`, `agent/authenticated_test.go`
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 9–200, 1189–1236, 7958–8151):**
  - `agent_gh` (invoked when `filepath.Base(os.Args[0]) == "gh"` or `agent-go _gh`): `gh api` endpoint & method allowlist, trusted author filtering (`TRUSTED_GITHUB_ACTORS`, `github_owner`, bot `[bot]` stripping), `gh pr list|view|diff|checkout`, `gh issue list|view`, `reviews` body byline injection (`review_byline`), `agents_gh_env`, `github_repo_url`, `github_trusted`, `trusted_prs`.
  - Auth: `auth_classify`, `api_error_text`, `agent_auth_target`, `cmd__authcheck`, `auth_probe`, `opencode_model_for`, `cmd_authenticated`, `auth_providers`.
  - Wire `bin/agent` to `exec "$AGENT_GO_BIN"` when `basename "$0" == "gh"` and for subcommands `authenticated` and `_authcheck`.
- **Bats Tests Ported to Go (`agent/gh_test.go`, `agent/authenticated_test.go`):**
  - `tests/unit/agent_gh.bats` (7 tests: allowlisted endpoints, blocked mutations, trusted vs untrusted PR/issue authors, diff/checkout blocks on untrusted PRs).
  - `tests/unit/github_trust.bats` (4 tests: trusted actors, owner trust, `[bot]` suffix handling).
  - `tests/unit/review_byline.bats` (3 tests: byline prepended, duplicate byline avoided, `review_byline` formatting).
  - `tests/unit/authenticated.bats` (3 tests: valid account, expired/unauthenticated classification, provider filtering).
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestGH|TestReviewByline|TestAuthenticated' ./...`
  - `bats tests/unit/agent_gh.bats tests/unit/github_trust.bats tests/unit/review_byline.bats tests/unit/authenticated.bats`
  - `make gate`

---

### Round 3: Archiving, Tokens, Transcripts, and Simple Inspection CLIs (`archive.go`, `tokens.go`, `transcript.go`, `status.go` Part 1)

- **Target Files to Create/Modify:**
  - `agent/archive.go`, `agent/archive_test.go`
  - `agent/tokens.go`, `agent/tokens_test.go`
  - `agent/transcript.go`, `agent/transcript_test.go`
  - `agent/status.go`, `agent/status_test.go`
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 5816–6045, 6277–6339, 6411–6459, 6516–6534, 8266–8505):**
  - `archive_index`, `archive_group`, `sevenzip`, `archive_one`, `cmd_discard`, `cmd_archive`.
  - `cmd_tokens` (reading `activity.jsonl` token events, per-task and `--all` totals).
  - `cmd_transcript` (formatting `activity.jsonl` and `prompt-*.md` / `final-*.md`).
  - `cmd_status`, `cmd_show`, `render_log`, `cmd_log`, `cmd_clean`, `cmd_agents`, `cmd_notes`, `cmd_helpers`, `cmd_bus`, `cmd_say`, `cmd_live`, `cmd_web`, `usage`.
  - Wire `bin/agent` to delegate `archive`, `discard`, `tokens`, `transcript`, `status`, `show`, `log`, `clean`, `agents`, `notes`, `helpers`, `bus`, `say`, `live`, `web`, `help`, `--help`, `-h`.
- **Bats Tests Ported to Go (`agent/archive_test.go`, `agent/tokens_test.go`, `agent/status_test.go`):**
  - `tests/unit/tokens.bats` (5 tests).
  - `tests/unit/web.bats` (6 tests) and `tests/unit/web_jobs.bats` (1 test) for `agent web` flag/env wiring.
  - Archive, discard, status, show, log, clean, agents, notes, bus, say tests from `tests/unit/agent.bats`.
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestArchive|TestDiscard|TestTokens|TestTranscript|TestStatus' ./...`
  - `bats tests/unit/tokens.bats tests/unit/web.bats tests/unit/web_jobs.bats tests/unit/agent.bats`
  - `make gate`

---

### Round 4: Questions, Remote Questions Staging, Notifications, and Prompt Rendering (`questions.go`, `notify.go`, `prompt.go`)

- **Target Files to Create/Modify:**
  - `agent/questions.go`, `agent/questions_test.go`
  - `agent/notify.go`, `agent/notify_test.go`
  - `agent/prompt.go`, `agent/prompt_test.go`
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 566–981, 2494–2779, 3055–3154, 5687–5739, 6340–6410):**
  - Questions: `count_questions`, `validate_answers`, `questions_missing_recommendations`, `expand_answers`, `answers_marked`, `questions_note`, `file_mtime`, `questions_ready`, `remote_identity`, `remote_question_context`, `cmd__remote_questions`, `cmd__remote_questions_ready`, `supervise_remote_questions`, `remote_answers_saved`, `watch_answers`, `stop_answers_watch`, `open_in_editor`, `cmd_answer`, `cmd__answers_saved`.
  - Notifications: `notify_send`, `notify_failure_keep`, `notice`, `push_text`, `notify`.
  - Prompts & Identity: `render`, `team_members_list`, `git_identity` (setting `user.name` / `user.email`), `commit_trailers`, `build_prompt`, `bus_args_json`, `checkpoints_latest`, `round_text_strip`.
  - Wire `bin/agent` to delegate `answer`, `_answers-saved`, `_remote-questions`, `_remote-questions-ready`.
- **Bats Tests Ported to Go (`agent/questions_test.go`, `agent/notify_test.go`, `agent/prompt_test.go`):**
  - `tests/unit/identity.bats` (3 tests), `tests/unit/prompt_file.bats` (2 tests), `tests/unit/prompt_checkpoints.bats` (1 test), `tests/unit/round_only.bats` (2 tests), `tests/unit/context_hygiene.bats` (2 tests).
  - `tests/unit/autopilot_questions.bats` (2 tests), `tests/unit/remote_questions_atomic.bats` (1 test), `tests/unit/remote_questions_preparation_retry.bats` (3 tests), `tests/unit/remote_answer_round.bats` (1 test).
  - `tests/unit/pause_notify.bats` (3 tests), `tests/unit/remote_control_notifications.bats` (1 test).
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestQuestion|TestNotify|TestPrompt|TestIdentity|TestRoundTextStrip' ./...`
  - `bats tests/unit/identity.bats tests/unit/prompt_file.bats tests/unit/prompt_checkpoints.bats tests/unit/round_only.bats tests/unit/autopilot_questions.bats tests/unit/remote_questions_atomic.bats tests/unit/remote_questions_preparation_retry.bats tests/unit/pause_notify.bats tests/unit/remote_control_notifications.bats`
  - `make gate`

---

### Round 5: Provider Limits, Quota Reset Parsing, Wake-Loop Guard, and CLI Launchers (`limits.go`, `cli.go`)

- **Target Files to Create/Modify:**
  - `agent/limits.go`, `agent/limits_test.go`
  - `agent/cli.go`, `agent/cli_test.go`
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 2273–2493, 2905–3054):**
  - Limits & Quota: `limit_keep`, `limit_lines`, `last_error_line`, `limit_kind` (`quota`, `rate`, `network`, `auth`, `overloaded`), `limit_line`, `quota_reset_seconds`, `duration_seconds`, `bus_messages_from`, `recent_run_starts`, `wake_loop_applies`, `wake_loop_check`, `schedule_wake`, `pause_task`, `resume_paused`.
  - CLI Launchers: `cli_prompt`, `is_independent_investigation`, `run_claude`, `codex_mcp_args`, `run_codex`, `run_gemini`, `opencode_config`, `run_opencode`.
- **Bats Tests Ported to Go (`agent/limits_test.go`, `agent/cli_test.go`):**
  - `tests/unit/limits.bats` (3 tests), `tests/unit/network.bats` (3 tests), `tests/unit/auto_stage.bats` (2 tests), `tests/unit/run_notes.bats` (3 tests), `tests/unit/remote_fixes.bats` (3 tests).
  - `tests/unit/investigation_mcp_off.bats` (2 tests), `tests/unit/investigation_team_isolation.bats` (4 tests), `tests/unit/investigation_team_opencode_schema.bats` (1 test).
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestLimit|TestQuotaReset|TestWakeLoop|TestCLI|TestMCP' ./...`
  - `bats tests/unit/limits.bats tests/unit/network.bats tests/unit/auto_stage.bats tests/unit/run_notes.bats tests/unit/remote_fixes.bats tests/unit/investigation_mcp_off.bats tests/unit/investigation_team_isolation.bats tests/unit/investigation_team_opencode_schema.bats`
  - `make gate`

---

### Round 6: Background Task Runner, Watchdog, Verification Loop, and Fresh-Session Logic (`runner.go`)

- **Target Files to Create/Modify:**
  - `agent/runner.go`, `agent/runner_test.go`
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 2780–2904, 4299–4889, 6340–6410):**
  - Watchdog & Process Group Management: `act`, `last_event`, `killtree`, `watch_run`, `activity_progress`.
  - Verification & Final Status: `pl_commits`, `verify_gaps`, `final_status`, `questions_from_nostatus`, `questions_from_final`.
  - Fresh-Session / Stash / Checkpoints: `long_run_end`, `fresh_session_due`, `stash_worktree`.
  - Runner Lifecycle: `runner_exit`, `cmd__runner`, `start_runner`.
  - Wire `bin/agent` to delegate `_runner`.
- **Bats Tests Ported to Go (`agent/runner_test.go`):**
  - `tests/unit/long_run_cut.bats` (2 tests), `tests/unit/fresh_session.bats` (3 tests), `tests/unit/fresh_relative.bats` (1 test), `tests/unit/verify_loop.bats` (3 tests), `tests/unit/resilience.bats` (3 tests).
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestWatchRun|TestFreshSession|TestVerifyLoop|TestRunner' ./...`
  - `bats tests/unit/long_run_cut.bats tests/unit/fresh_session.bats tests/unit/fresh_relative.bats tests/unit/verify_loop.bats tests/unit/resilience.bats`
  - `make gate`

---

### Round 7: Task Allocation, `run`, `resume`, `stop`, `wake`, and `kick` (`run.go`, `resume.go`)

- **Target Files to Create/Modify:**
  - `agent/run.go`, `agent/run_test.go`
  - `agent/resume.go`, `agent/resume_test.go`
  - `agent/status.go` (`stop_all`, `cmd_stop`)
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 4890–5685, 5740–5815, 5915–5969, 6460–6515):**
  - Task Creation & Allocation: `lock_task_ids`, `unlock_task_ids`, `allocate_run_ids`, `create_task`, `drop_unstarted_task`, `job_refusal`, `prepare_job_task`, `discard_task`.
  - Run & Resume: `abandon_run`, `resume_history`, `resume_run`, `cmd_run`, `cmd_resume`, `cmd_wake`, `cmd_kick`, `stop_all`, `cmd_stop`.
  - Wire `bin/agent` to delegate `run`, `resume`, `stop`, `wake`, `kick`.
- **Bats Tests Ported to Go (`agent/run_test.go`, `agent/resume_test.go`):**
  - `tests/unit/resume_alloc.bats` (5 tests), `tests/unit/resume_members.bats` (2 tests), `tests/unit/resume_stash.bats` (1 test), `tests/unit/kick.bats` (3 tests), `tests/unit/wake_cmd.bats` (4 tests), `tests/unit/agent.bats` (run/resume/stop tests).
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestAllocate|TestCreateTask|TestRun|TestResume|TestStop|TestKick|TestWake' ./...`
  - `bats tests/unit/resume_alloc.bats tests/unit/resume_members.bats tests/unit/resume_stash.bats tests/unit/kick.bats tests/unit/wake_cmd.bats tests/unit/agent.bats`
  - `make gate`

---

### Round 8: Rounds, Skipped Tests, PR Format Validation, and Autopilot Merge/Review (`rounds.go`, `autopilot.go`)

- **Target Files to Create/Modify:**
  - `agent/rounds.go`, `agent/rounds_test.go`
  - `agent/autopilot.go`, `agent/autopilot_test.go`
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 982–2099):**
  - Rounds & Skips: `team_waits_on_others`, `round_limit`, `round_limit_text`, `round_soft`, `round_base_ref`, `skips_in_repo`, `skips_added`, `pr_skipped_section`, `skip_note`, `agent_desc`, `attribution_block`, `round_check`, `round_pr_url`, `cmd_round_size`, `last_team_id`, `round_open_prs`, `round_pr_note`, `round_pr_of_team`, `round_pr_supersede`.
  - Autopilot & Reviews: `count_of`, `findings_count`, `review_verdict`, `review_task_verdict`, `round_test_changes`, `pr_format_issues`, `merge_blockers`, `gate_at`, `start_task_unit`, `cmd__merge`, `cmd_autopilot`, `supervise_review_waits`, `review_gate_cmd`, `auto_stage`, `human_on_pr`, `gate_progress`, `pair_stage`, `start_precheck`, `team_wait_settled`, `cmd__precheck`, `review_precheck`, `review_followup`, `review_fix_pass`, `cmd__fixpass`, `do_fix_pass`, `handover_failed`, `review_without_verdict`, `retry_review_slot`, `review_pair_done`, `free_investigator_list`, `start_review_pairs`, `start_review_pair`, `start_review_slot`, `request_review`.
  - Wire `bin/agent` to delegate `round-size`, `autopilot`, `_merge`, `_precheck`, `_fixpass`.
- **Bats Tests Ported to Go (`agent/rounds_test.go`, `agent/autopilot_test.go`):**
  - `tests/unit/rounds.bats` (8 tests), `tests/unit/round_handover.bats` (4 tests), `tests/unit/round_reports.bats` (1 test), `tests/unit/one_pr_per_round.bats` (3 tests), `tests/unit/skips.bats` (7 tests), `tests/unit/github_format.bats` (2 tests), `tests/unit/findings.bats` (1 test).
  - `tests/unit/autopilot_merge.bats` (5 tests), `tests/unit/autopilot_review.bats` (8 tests), `tests/unit/review_pair_random.bats` (4 tests), `tests/unit/review_resilience.bats` (7 tests), `tests/unit/handover_failed.bats` (3 tests).
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestRound|TestSkip|TestPRFormat|TestFindings|TestAutopilot|TestReviewPair|TestMergeBlockers' ./...`
  - `bats tests/unit/rounds.bats tests/unit/round_handover.bats tests/unit/round_reports.bats tests/unit/one_pr_per_round.bats tests/unit/skips.bats tests/unit/github_format.bats tests/unit/findings.bats tests/unit/autopilot_merge.bats tests/unit/autopilot_review.bats tests/unit/review_pair_random.bats tests/unit/review_resilience.bats tests/unit/handover_failed.bats`
  - `make gate`

---

### Round 9: Teams, Workers/Helpers, Single Investigations, and Supervisor (`team.go`, `helpers.go`, `investigate.go`, `supervise.go`)

- **Target Files to Create/Modify:**
  - `agent/team.go`, `agent/team_test.go`
  - `agent/helpers.go`, `agent/helpers_test.go`
  - `agent/investigate.go`, `agent/investigate_test.go`
  - `agent/supervise.go`, `agent/supervise_test.go`
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 897–945, 2100–2271, 3155–3230, 4050–4298, 5970–6276, 8153–8265):**
  - Team: `team_json`, `team_lead`, `team_done_flag`, `team_tasks`, `team_summary`, `finish_team`, `team_after_run`, `cmd__wake`, `cmd_team`, `team_close`, `member_out`, `remote_member_task`, `cmd__remote_member`, `remote_agent_wait_free`, `resume_abandon`, `remote_member_start`.
  - Investigations & Rotation: `pick_investigator`, `is_one_of`, `remember_investigator`, `free_investigator`, `dispatch`, `start_investigation`, `inv_finish`, `maybe_resume_worker`, `cmd_inv`, `cmd_dispatch`.
  - Helpers: `free_worker`, `dispatch_helpers`, `start_helper`, `helper_finish`.
  - Supervisor & Boot Recovery: `cmd__supervise`, `supervise_member_jobs`, `supervise_teams`, `node_stop_doorbell`, `boot_id`, `restart_interrupted`.
  - Wire `bin/agent` to delegate `team`, `inv`, `dispatch`, `_wake`, `_supervise`, `_remote-member`.
- **Bats Tests Ported to Go (`agent/team_test.go`, `agent/investigate_test.go`, `agent/supervise_test.go`):**
  - `tests/unit/coordination.bats` (1 test), `tests/unit/team_close.bats` (3 tests), `tests/unit/member_out.bats` (6 tests), `tests/unit/rotation.bats` (2 tests), `tests/unit/restart.bats` (4 tests).
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestTeam|TestMemberOut|TestRotation|TestRestart|TestSupervise|TestHelper' ./...`
  - `bats tests/unit/coordination.bats tests/unit/team_close.bats tests/unit/member_out.bats tests/unit/rotation.bats tests/unit/restart.bats`
  - `make gate`

---

### Round 10: Investigation Teams — Validation, CLI, Admission, Phase Selection, and Phase Queue (`investigation_team.go` Part 1)

- **Target Files to Create/Modify:**
  - `agent/investigation_team.go`, `agent/investigation_team_test.go`
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 3500–4049, 5480–5545):**
  - Validation & CLI: `validate_investigation_team`, `validate_investigation_topic`, `cmd_run --investigation-team`.
  - Phase Selection & Queueing: `investigation_field`, `investigation_phase_choose`, `investigation_phase_pick`, `_investigator_is_busy`, `investigation_phase_start`, `investigation_phase_failed`, `investigation_pipeline_tick`, `investigation_team_admit`, `investigation_teams_admit`, `investigation_team_stop`, `investigation_teams_cleanup`.
- **Bats Tests Ported to Go (`agent/investigation_team_test.go`):**
  - `tests/unit/investigation_team_validation.bats` (13 tests)
  - `tests/unit/investigation_team_stdin.bats` (2 tests)
  - `tests/unit/investigation_team_cli.bats` (12 tests)
  - `tests/unit/investigation_team_admission.bats` (8 tests)
  - `tests/unit/investigation_team_phase_selection.bats` (11 tests)
  - `tests/unit/investigation_team_phase_queue.bats` (4 tests)
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestInvestigationTeam(Validation|Stdin|CLI|Admission|PhaseSelection|PhaseQueue)' ./...`
  - `bats tests/unit/investigation_team_validation.bats tests/unit/investigation_team_stdin.bats tests/unit/investigation_team_cli.bats tests/unit/investigation_team_admission.bats tests/unit/investigation_team_phase_selection.bats tests/unit/investigation_team_phase_queue.bats`
  - `make gate`

---

### Round 11: Investigation Teams — Handoff, Phase 1 Publication Recovery, Advisory Resume, and Reboot/Recovery Regressions (`investigation_team.go` Part 2)

- **Target Files to Create/Modify:**
  - `agent/investigation_team.go`, `agent/investigation_team_recovery_test.go`
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 3231–3499, 3600–4049):**
  - Advisory Resume & Recovery: `investigation_resume_write`, `investigation_resume_tick`, `investigation_resume_admit`, `investigation_resume`.
  - Phase 1 Publication & Handoff: `investigation_phase1_write`, `investigation_published`, `investigation_phase1_recover`, `investigation_handoff`, `investigation_team_tick`, `investigation_teams_tick`, `cmd__investigation_team`.
  - Wire `bin/agent` to delegate `_investigation-team`.
- **Bats Tests Ported to Go (`agent/investigation_team_recovery_test.go`):**
  - `tests/unit/investigation_team_handoff.bats` (8 tests)
  - `tests/unit/investigation_team_publication_retry.bats` (2 tests)
  - `tests/unit/investigation_team_advisory_resume.bats` (4 tests)
  - `tests/unit/investigation_team_resume_boundaries.bats` (3 tests)
  - `tests/unit/investigation_team_resume_recovery.bats` (4 tests)
  - `tests/unit/investigation_team_runtime.bats` (8 tests)
  - `tests/unit/investigation_team_supervision.bats` (4 tests)
  - `tests/unit/investigation_team_reboot_regression.bats` (2 tests)
  - `tests/unit/investigation_team_recovery_regressions.bats` (8 tests)
  - `tests/unit/investigation_team_regressions.bats` (2 tests)
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestInvestigationTeam' ./...`
  - `bats tests/unit/investigation_team_*.bats`
  - `make gate`

---

### Round 12: Multi-Instance Lead & Node Core — `lead`, `node`, `jobs`, `nodes`, `ack`, Checkout, and Recovery Fence (`node.go`)

- **Target Files to Create/Modify:**
  - `agent/node.go`, `agent/node_test.go`
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 6480–7350, 7700–7957):**
  - Lead & Job Submission: `cmd_lead`, `lead_url`, `submit_remote_job`, `cmd_jobs`, `cmd_nodes`, `cmd_ack`, `cmd__agents_json`.
  - Node Daemon Core: `node_lock`, `node_jobs_file`, `node_jobs_load`, `node_jobs_save`, `node_save`, `node_time`, `node_transition`, `node_heartbeat_json`, `node_activity_json`, `agent_cli_present`, `node_poll`, `node_poll_remote_helpers`, `node_request`, `node_start_ack`, `node_process_pending_launches`, `node_recover_tasks`, `node_handle_stop`, `node_job_result`, `cmd_node`.
  - Wire `bin/agent` to delegate `lead`, `jobs`, `nodes`, `node`, `_agents-json`, `ack`.
- **Bats Tests Ported to Go (`agent/node_test.go`):**
  - `tests/unit/lead.bats` (3 tests), `tests/unit/lead_cli.bats` (4 tests), `tests/unit/lead_cli_context.bats` (5 tests), `tests/unit/lead_install.bats` (2 tests), `tests/unit/node_install.bats` (4 tests).
  - `tests/unit/delegate_jobs.bats` (7 tests), `tests/unit/job_attribution.bats` (2 tests), `tests/unit/job_attribution_paths.bats` (2 tests).
  - `tests/unit/node_agents.bats` (1 test), `tests/unit/node_activity.bats` (3 tests), `tests/unit/node_report_agent.bats` (1 test), `tests/unit/node_safety.bats` (4 tests), `tests/unit/node_large_assignment.bats` (1 test).
  - `tests/unit/node_checkout.bats` (18 tests), `tests/unit/node_daemon.bats` (15 tests), `tests/unit/node_cancel_exit.bats` (4 tests), `tests/unit/node_prelaunch_cancel.bats` (2 tests), `tests/unit/node_recovery_fence.bats` (6 tests), `tests/unit/node_review_boundaries.bats` (7 tests), `tests/unit/node_helper_hooks.bats` (12 tests).
- **Note on Size:** Because Round 12 covers 20 `.bats` files, if the combined implementation + test lines approach 2,500 lines, split Round 12 into **Round 12a** (`lead`, `jobs`, `nodes`, `ack`, `_agents-json`, `node_checkout`, `node_safety`, `job_attribution`) and **Round 12b** (`node` daemon loop, `node_daemon`, `node_recovery_fence`, `node_review_boundaries`, `node_cancel_exit`, `node_prelaunch_cancel`, `node_helper_hooks`).
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestLead|TestNode|TestJob' ./...`
  - `bats tests/unit/lead*.bats tests/unit/delegate_jobs.bats tests/unit/job_attribution*.bats tests/unit/node_*.bats`
  - `make gate`

---

### Round 13: Node Synchronization, Remote Controls, and Multi-Home End-to-End Tests (`sync.go`, `remote_control.go`)

- **Target Files to Create/Modify:**
  - `agent/sync.go`, `agent/sync_test.go`
  - `agent/remote_control.go`, `agent/remote_control_test.go`
  - `agent/errors.json`, `agent/errors_gen.go`, `agent/main.go`, `bin/agent`
- **Functions Ported from `bin/agent` (Lines 7351–7699):**
  - Sync: `node_scan_tasks`, `node_sync_tasks`, `node_sync_task_payload`, `node_flush_reports`.
  - Remote Controls: `node_handle_reply`, `node_handle_controls`, `node_apply_answer`, `node_apply_kick`, `node_clear_control_acks`.
- **Bats Tests Ported to Go (`agent/sync_test.go`, `agent/remote_control_test.go`):**
  - `tests/unit/node_sync.bats` (6 tests), `tests/unit/node_sync_freshness.bats` (3 tests), `tests/unit/node_sync_limits.bats` (4 tests), `tests/unit/node_sync_review.bats` (8 tests), `tests/unit/node_sync_state.bats` (1 test), `tests/unit/node_sync_tails.bats` (2 tests), `tests/unit/node_sync_two_homes.bats` (1 test), `tests/unit/node_two_homes.bats` (11 tests).
  - `tests/unit/remote_controls.bats` (3 tests), `tests/unit/remote_control_cancel_order.bats` (1 test), `tests/unit/remote_control_heartbeat_limits.bats` (3 tests), `tests/unit/remote_control_kick_exit.bats` (3 tests), `tests/unit/remote_control_proxy.bats` (1 test), `tests/unit/remote_control_review.bats` (7 tests), `tests/unit/remote_control_validation.bats` (7 tests), `tests/unit/remote_control_web.bats` (1 test), `tests/unit/remote_proxy.bats` (2 tests).
- **What to Check / How to Run Tests:**
  - `cd agent && go test -race -v -run 'TestNodeSync|TestRemoteControl|TestTwoHomes' ./...`
  - `bats tests/unit/node_sync*.bats tests/unit/node_two_homes.bats tests/unit/remote_*.bats`
  - `make gate`

---

### Round 14: Final Cutover, `install.sh` & Documentation Updates, and Full Parity Audit

- **Target Files to Create/Modify:**
  - `bin/agent` (reduced to thin exec + bats-source shim in Option 14A, or replaced by compiled `build/agent` binary in Option 14B once approved by the human)
  - `Makefile`, `install.sh`, `README.md`
  - `agent/parity_test.go` (verifies every CLI subcommand, exit code, and `_fn` entry point)
- **What to Do in Round 14:**
  1. Verify that **every single subcommand** in `agent/main.go` (`run`, `resume`, `answer`, `wake`, `kick`, `status`, `show`, `log`, `tokens`, `stop`, `clean`, `discard`, `archive`, `agents`, `authenticated`, `inv`, `team`, `notes`, `helpers`, `transcript`, `round-size`, `autopilot`, `dispatch`, `bus`, `say`, `live`, `web`, `lead`, `jobs`, `nodes`, `node`, `ack`, `_runner`, `_supervise`, `_wake`, `_merge`, `_precheck`, `_fixpass`, `_answers-saved`, `_remote-member`, `_remote-questions`, `_remote-questions-ready`, `_investigation-team`, `_agents-json`, `_authcheck`, and `argv[0] == "gh"`) executes in Go without falling back to Bash.
  2. Update `install.sh` so `agent-go` (or compiled `bin/agent`) is built and installed into `~/.local/bin/` alongside `agentbus` and `agentlive` (note Rule 3: **never run `install.sh` against your own home**; test it only via `bats tests/unit/lead_install.bats tests/unit/node_install.bats`).
  3. Update `README.md` architecture table to document `agent/` (Go, standard library only).
  4. Audit test parity: verify that all 110 `.bats` files have corresponding `agent/*_test.go` tests and that both `cd agent && go test -race ./...` and `bats tests/unit` pass with zero findings.
- **What to Check / How to Run Tests:**
  - `make gate`
  - `bats tests/unit/lead_install.bats tests/unit/node_install.bats`
  - `agent round-size <task>`

---

## 5. Complete 1-to-1 Mapping Table: `tests/unit/*.bats` → `agent/*_test.go`

Use this checklist to verify that no `.bats` file is missed when porting tests to Go:

| Bats Test File (`tests/unit/`) | `@test` Count | Target Go Source File | Target Go Test File | Round |
|---|---:|---|---|---:|
| `meta.bats` | 1 | `agent/meta.go` | `agent/meta_test.go` | 1 |
| `gate.bats` | 4 | `tools/gate.sh` (existing) | `agent/config_test.go` | 1 |
| `lint_shell_memory.bats` | 1 | `Makefile` (existing) | `agent/config_test.go` | 1 |
| `agent_gh.bats` | 7 | `agent/gh.go` | `agent/gh_test.go` | 2 |
| `github_trust.bats` | 4 | `agent/gh.go` | `agent/gh_test.go` | 2 |
| `review_byline.bats` | 3 | `agent/gh.go` | `agent/gh_test.go` | 2 |
| `authenticated.bats` | 3 | `agent/authenticated.go` | `agent/authenticated_test.go` | 2 |
| `tokens.bats` | 5 | `agent/tokens.go` | `agent/tokens_test.go` | 3 |
| `web.bats` | 6 | `agent/status.go` | `agent/status_test.go` | 3 |
| `web_jobs.bats` | 1 | `agent/status.go` | `agent/status_test.go` | 3 |
| `identity.bats` | 3 | `agent/prompt.go` | `agent/prompt_test.go` | 4 |
| `prompt_file.bats` | 2 | `agent/prompt.go` | `agent/prompt_test.go` | 4 |
| `prompt_checkpoints.bats` | 1 | `agent/prompt.go` | `agent/prompt_test.go` | 4 |
| `round_only.bats` | 2 | `agent/prompt.go` | `agent/prompt_test.go` | 4 |
| `context_hygiene.bats` | 2 | `agent/prompt.go` | `agent/prompt_test.go` | 4 |
| `autopilot_questions.bats` | 2 | `agent/questions.go` | `agent/questions_test.go` | 4 |
| `remote_questions_atomic.bats` | 1 | `agent/questions.go` | `agent/questions_test.go` | 4 |
| `remote_questions_preparation_retry.bats` | 3 | `agent/questions.go` | `agent/questions_test.go` | 4 |
| `remote_answer_round.bats` | 1 | `agent/questions.go` | `agent/questions_test.go` | 4 |
| `pause_notify.bats` | 3 | `agent/notify.go` | `agent/notify_test.go` | 4 |
| `remote_control_notifications.bats` | 1 | `agent/notify.go` | `agent/notify_test.go` | 4 |
| `limits.bats` | 3 | `agent/limits.go` | `agent/limits_test.go` | 5 |
| `network.bats` | 3 | `agent/limits.go` | `agent/limits_test.go` | 5 |
| `auto_stage.bats` | 2 | `agent/limits.go` | `agent/limits_test.go` | 5 |
| `run_notes.bats` | 3 | `agent/limits.go` | `agent/limits_test.go` | 5 |
| `remote_fixes.bats` | 3 | `agent/limits.go` | `agent/limits_test.go` | 5 |
| `investigation_mcp_off.bats` | 2 | `agent/cli.go` | `agent/cli_test.go` | 5 |
| `investigation_team_isolation.bats` | 4 | `agent/cli.go` | `agent/cli_test.go` | 5 |
| `investigation_team_opencode_schema.bats` | 1 | `agent/cli.go` | `agent/cli_test.go` | 5 |
| `long_run_cut.bats` | 2 | `agent/runner.go` | `agent/runner_test.go` | 6 |
| `fresh_session.bats` | 3 | `agent/runner.go` | `agent/runner_test.go` | 6 |
| `fresh_relative.bats` | 1 | `agent/runner.go` | `agent/runner_test.go` | 6 |
| `verify_loop.bats` | 3 | `agent/runner.go` | `agent/runner_test.go` | 6 |
| `resilience.bats` | 3 | `agent/runner.go` | `agent/runner_test.go` | 6 |
| `agent.bats` | 30 | `agent/run.go`, `status.go`, `archive.go` | `agent/run_test.go`, `status_test.go`, `archive_test.go` | 3, 7 |
| `resume_alloc.bats` | 5 | `agent/resume.go` | `agent/resume_test.go` | 7 |
| `resume_members.bats` | 2 | `agent/resume.go` | `agent/resume_test.go` | 7 |
| `resume_stash.bats` | 1 | `agent/resume.go` | `agent/resume_test.go` | 7 |
| `kick.bats` | 3 | `agent/resume.go` | `agent/resume_test.go` | 7 |
| `wake_cmd.bats` | 4 | `agent/resume.go` | `agent/resume_test.go` | 7 |
| `rounds.bats` | 8 | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `round_handover.bats` | 4 | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `round_reports.bats` | 1 | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `one_pr_per_round.bats` | 3 | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `skips.bats` | 7 | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `github_format.bats` | 2 | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `findings.bats` | 1 | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `autopilot_merge.bats` | 5 | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `autopilot_review.bats` | 8 | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `review_pair_random.bats` | 4 | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `review_resilience.bats` | 7 | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `handover_failed.bats` | 3 | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `coordination.bats` | 1 | `agent/team.go` | `agent/team_test.go` | 9 |
| `team_close.bats` | 3 | `agent/team.go` | `agent/team_test.go` | 9 |
| `member_out.bats` | 6 | `agent/team.go` | `agent/team_test.go` | 9 |
| `rotation.bats` | 2 | `agent/investigate.go` | `agent/investigate_test.go` | 9 |
| `restart.bats` | 4 | `agent/supervise.go` | `agent/supervise_test.go` | 9 |
| `investigation_team_validation.bats` | 13 | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_stdin.bats` | 2 | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_cli.bats` | 12 | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_admission.bats` | 8 | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_phase_selection.bats` | 11 | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_phase_queue.bats` | 4 | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_handoff.bats` | 8 | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_publication_retry.bats` | 2 | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_advisory_resume.bats` | 4 | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_resume_boundaries.bats` | 3 | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_resume_recovery.bats` | 4 | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_runtime.bats` | 8 | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_supervision.bats` | 4 | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_reboot_regression.bats` | 2 | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_recovery_regressions.bats` | 8 | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_regressions.bats` | 2 | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `lead.bats` | 3 | `agent/node.go` | `agent/node_test.go` | 12 |
| `lead_cli.bats` | 4 | `agent/node.go` | `agent/node_test.go` | 12 |
| `lead_cli_context.bats` | 5 | `agent/node.go` | `agent/node_test.go` | 12 |
| `lead_install.bats` | 2 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_install.bats` | 4 | `agent/node.go` | `agent/node_test.go` | 12 |
| `delegate_jobs.bats` | 7 | `agent/node.go` | `agent/node_test.go` | 12 |
| `job_attribution.bats` | 2 | `agent/node.go` | `agent/node_test.go` | 12 |
| `job_attribution_paths.bats` | 2 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_agents.bats` | 1 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_activity.bats` | 3 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_report_agent.bats` | 1 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_safety.bats` | 4 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_large_assignment.bats` | 1 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_checkout.bats` | 18 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_daemon.bats` | 15 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_cancel_exit.bats` | 4 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_prelaunch_cancel.bats` | 2 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_recovery_fence.bats` | 6 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_review_boundaries.bats` | 7 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_helper_hooks.bats` | 12 | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_sync.bats` | 6 | `agent/sync.go` | `agent/sync_test.go` | 13 |
| `node_sync_freshness.bats` | 3 | `agent/sync.go` | `agent/sync_test.go` | 13 |
| `node_sync_limits.bats` | 4 | `agent/sync.go` | `agent/sync_test.go` | 13 |
| `node_sync_review.bats` | 8 | `agent/sync.go` | `agent/sync_test.go` | 13 |
| `node_sync_state.bats` | 1 | `agent/sync.go` | `agent/sync_test.go` | 13 |
| `node_sync_tails.bats` | 2 | `agent/sync.go` | `agent/sync_test.go` | 13 |
| `node_sync_two_homes.bats` | 1 | `agent/sync.go` | `agent/sync_test.go` | 13 |
| `node_two_homes.bats` | 11 | `agent/sync.go`, `node.go` | `agent/sync_test.go` | 13 |
| `remote_controls.bats` | 3 | `agent/remote_control.go` | `agent/remote_control_test.go` | 13 |
| `remote_control_cancel_order.bats` | 1 | `agent/remote_control.go` | `agent/remote_control_test.go` | 13 |
| `remote_control_heartbeat_limits.bats` | 3 | `agent/remote_control.go` | `agent/remote_control_test.go` | 13 |
| `remote_control_kick_exit.bats` | 3 | `agent/remote_control.go` | `agent/remote_control_test.go` | 13 |
| `remote_control_proxy.bats` | 1 | `agent/remote_control.go` | `agent/remote_control_test.go` | 13 |
| `remote_control_review.bats` | 7 | `agent/remote_control.go` | `agent/remote_control_test.go` | 13 |
| `remote_control_validation.bats` | 7 | `agent/remote_control.go` | `agent/remote_control_test.go` | 13 |
| `remote_control_web.bats` | 1 | `agent/remote_control.go` | `agent/remote_control_test.go` | 13 |
| `remote_proxy.bats` | 2 | `agent/remote_control.go` | `agent/remote_control_test.go` | 13 |

---

## 6. How to Translate Bats Tests to Go Unit Tests (`agent/*_test.go`)

### 6.1 Shared Go Test Harness (`agent/testhelpers_test.go`)

Create `agent/testhelpers_test.go` in Round 1 to replicate `tests/unit/helpers.bash` cleanly in Go:

1. **`newTestEnv(t *testing.T) *Env`:**
   - Creates `home := t.TempDir()` and `agentsHome := filepath.Join(home, "agents")` with subdirectories `tasks`, `worktrees`, `bus`, `logs`, `archive`, `locks`, `prompts`.
   - Copies `prompts/*.md` from `../prompts` into `agentsHome/prompts`.
   - Writes a default `agents.conf` and `0600` `secrets.env` matching `tests/unit/helpers.bash`.
   - Initializes a local git repository at `filepath.Join(home, "repo")` with an `origin` bare remote at `filepath.Join(home, "remote.git")` on branch `main`.
   - Sets `PATH` for external subprocesses to `<home>/.local/bin:<repo_root>/tests/unit/stubs:<repo_root>/build:$PATH` so Go tests that exercise stub CLIs (`claude`, `codex`, `gemini`, `opencode`, `gh`, `systemctl`, `systemd-run`) work identically to Bats tests.
   - Registers `t.Cleanup(...)` to kill any background stub runner processes started under that `home` (matching `teardown()` in `helpers.bash`).
2. **`failsWith[C opCause](t *testing.T, want C)`:**
   - Identical to `agentbus/bus_test.go`: asserts that an operation returned a non-nil `*OpError[C]` with `err.Cause == want`, never asserting on brittle error message strings.
3. **`scenario(t *testing.T, env *Env, jsonSpec string)` and `waitStatus(t *testing.T, env *Env, taskID string, want TaskStatus)`:**
   - Writes `$HOME/stub-scenario.json` for `tests/unit/stubs/*` when testing full runner flows, or configures the in-process `FakeRunner` for pure unit tests.

### 6.2 Translating the Three Types of Bats Tests

1. **Translating `source bin/agent` + Function Call Tests:**
   - *Bats example (`round_only.bats`):*
     ```bash
     out=$(source "$REPO_ROOT/bin/agent"; round_text_strip < "$BATS_TEST_TMPDIR/task.md")
     ```
   - *Go equivalent (`agent/prompt_test.go`):*
     Call `got := roundTextStrip(input)` directly in a table-driven Go test (`TestRoundTextStrip`). No subprocess or shell needed.
2. **Translating `sed`-Extracted or Shell-Mocked Function Tests:**
   - *Bats example (`node_agents.bats`):*
     Extracts `node_heartbeat_json` with `sed`, mocks `agent_cli_present`, `agent_running_task`, `node_activity_json`, and inspects the JSON output.
   - *Go equivalent (`agent/node_test.go`):*
     Call `env.nodeHeartbeatJSON()` with a `FakeRunner` (where `LookPath("missing")` returns `exec.ErrNotFound`) and a task directory in `env.AgentsHome/tasks/` with `STATUS=running` and `AGENT=busy`, then unmarshal and assert on the resulting struct/JSON.
3. **Translating Multi-Home Node/Lead Tests (`node_two_homes.bats`, `remote_control_*.bats`):**
   - Create two independent `*Env` instances (`leadEnv := newTestEnv(t)`, `nodeEnv := newTestEnv(t)`), start the lead HTTP server in-process via `httptest.NewServer` (or `agentbus lead-serve`), and run `nodeEnv.nodePoll()` / `nodeEnv.nodeSyncTasks()` directly against `leadServer.URL`. This runs in < 50 ms per test with full `-race` detector coverage.

---

## 7. Step-by-Step Verification Checklist for Every Round

Before committing and pushing any round:

1. **Generate and verify error types:**
   ```sh
   make generate
   git diff --exit-code agent/errors_gen.go
   ```
2. **Run Go unit tests with the race detector:**
   ```sh
   cd agent && go vet ./... && go test -race ./...
   ```
3. **Run documentation check (`doccheck`) and Go linters (`golangci-lint`, `staticcheck`):**
   ```sh
   make lint-go lint-docs
   ```
   Every constant, variable, type, and struct field in `agent/*.go` (non-test files) must have a comment (`tools/doccheck`).
4. **Run the unmodified Bats suite against the updated build:**
   ```sh
   bats tests/unit/<round_specific>.bats
   ```
5. **Run the full repository gate:**
   ```sh
   make gate
   ```
   Remember: never print a full log file from `build/gate/`; inspect only with `grep -n` or `tail` if a step fails.
6. **Verify zero modifications to existing tests and check round size:**
   ```sh
   git diff --name-only origin/main -- 'tests/**' '*_test.go'
   ```
   Confirm that no existing test file in `tests/**`, `agentbus/*_test.go`, or `live/*_test.go` was modified, and verify that the total diff (`agent round-size <task>`) is under the 3,000-line hard limit (aiming for ≤ 2,000 lines).
