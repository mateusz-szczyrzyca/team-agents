# Step-by-Step Plan: Rewriting `bin/agent` to Go (`agent/*.go`) with TDD and Bats Verification

## 1. Executive Summary and Baseline Inventory

`bin/agent` is currently an 8,583-line Bash script containing 246 functions, 33 public CLI subcommands, and 15 internal (`_`-prefixed or `argv[0]`) entry points. It is verified by 120 Bats test files (`tests/unit/*.bats`) containing 483 `@test` cases (13,182 lines of test code).

The goal is to rewrite `bin/agent` into a modular Go program under `agent/` (split across focused files such as `agent/team.go`, `agent/archive.go`, `agent/run.go`, `agent/authenticated.go`, etc.) with 1-to-1 Go unit tests (`agent/*_test.go`) covering the same behaviours and edge cases as the Bats suite, while:

1. **Keeping all existing Bats tests (`tests/unit/**`) untouched and passing at every step** (`AGENTS.md` Rule 1 and `AUTO_MERGE_HOLD_TEST_CHANGES=yes`).
2. **Following strict Test-First / TDD** (`docs/tdd.md`: $\color{red}{\textbf{RED}}$ commit with failing Go test $\rightarrow$ $\color{green}{\textbf{GREEN}}$ commit with minimal Go implementation + `bin/agent` delegation $\rightarrow$ **REFACTOR** commit).
3. **Keeping every round within ~1,500 counted lines** (`added + deleted` in `agentbus round-size`, including `*_test.go`, excluding generated `*_gen.go`).

Because `agentbus round-size` counts **both added and deleted lines** (`Lines = Added + Deleted`), migrating 8,583 lines of Bash into ~7,200 lines of Go implementation and ~7,200 lines of Go unit tests (~23,000 total counted line changes) requires **16 rounds**, each budgeted at **1,250–1,490 counted lines**.

---

## 2. Target Go Architecture (`agent/*.go`)

The Go implementation lives in a new Go module `agent/` (`agent/go.mod`, `module agent`, Go 1.22+, standard library only — zero third-party dependencies per `AGENTS.md` Rule 8).

### 2.1 File Layout and Domain Mapping

| Go File | Go Test File | Domain & Responsibility | `bin/agent` Lines Replaced |
|---|---|---|---|
| `agent/main.go` | `agent/main_test.go` | CLI entry point, subcommand dispatcher, `usage()`, `_fn` test-shim bridge | `8475–8583` |
| `agent/errors.go` | `agent/errors_test.go` | `ErrorClass`, `OpError[C]`, `*StorageError`, `*ValueError`, `*ProcessError`, `asError()` | `284` (`die`) |
| `agent/errors.json` | *(generated `errors_gen.go`)* | Per-operation error cause specifications for `tools/errgen` (`make generate`) | — |
| `agent/logging.go` | `agent/logging_test.go` | `LogEvent` constants (240+ events), `logs/agent.jsonl`, journald socket (`SYSLOG_IDENTIFIER=agent`), runner fd 3, `try`, `jq_soft` | `209–283` |
| `agent/types.go` | `agent/types_test.go` | Typed enums (`TaskStatus`, `TaskKind`, `AgentType`, `AgentTier`, `Verdict`, `LimitKind`, `NoticeKind`), process boundary vars | `294–308`, `392–393` |
| `agent/config.go` | `agent/config_test.go` | `agents.conf` & `secrets.env` loader, defaults, provider/tier resolution, `.stop-all` check (`stopping`) | `202–208`, `286–461` |
| `agent/meta.go` | `agent/meta_test.go` | `meta.env` reader/writer (`meta_set`, `meta_load`, `task_field`), `flock` (`meta.env.lock`, `task-ids.lock`), `resolve_id`, `unit_active`, `agent_running_task`, `job_transitions.jsonl` | `463–565`, `4946–4962` |
| `agent/gh.go` | `agent/gh_test.go` | `agent_gh` (`argv[0] == "gh"` filter), trusted GitHub accounts, review byline injection, GraphQL blocking, `github_repo_url`, `github_owner`, `trusted_prs` | `1–200`, `1159–1180`, `1236–1285` |
| `agent/questions.go` | `agent/questions_test.go` | Question counting, answer validation, recommendation checks, answer expansion, systemd `.path` watches, VS Code opener, remote question staging/ready | `566–853`, `1828–1844`, `4398–4435`, `5740–5847` |
| `agent/notify.go` | `agent/notify_test.go` | `notify`, `notify_send`, `notify_failure_keep` (`notify-failures.jsonl`), `notice` (typed bus notices), `push_text` | `898–948`, `3068–3167` |
| `agent/prompts.go` | `agent/prompts_test.go` | Template `render` (`{{PLACEHOLDERS}}`), `build_prompt`, `git_identity`, `commit_trailers`, `agents_md_text`, `privileges_text`, `peers_list`, `checkpoints_latest`, `round_text_strip` | `854–872`, `2498–2792`, `6387–6406` |
| `agent/limits.go` | `agent/limits_test.go` | Quota/rate-limit/network/capacity classifier (`limit_kind`, `quota_reset_seconds`), wake-loop guard, `pause_task`, `resume_paused`, `fresh_session_due`, `long_run_end`, `cmd_tokens`, `cmd_wake` | `1798–1827`, `2295–2497`, `6339–6370`, `6407–6514` |
| `agent/rounds.go` | `agent/rounds_test.go` | Round size checks (`round_check`, `cmd_round_size`), skip scanner (`skips_in_repo`, `skips_added`, `pr_skipped_section`), `findings_count`, `review_verdict`, `round_test_changes`, `pr_format_issues`, `merge_blockers`, `round_open_prs`, `round_pr_supersede` | `982–1158`, `1181–1233`, `1286–1334`, `1483–1540` |
| `agent/cli_exec.go` | `agent/cli_exec_test.go` | Running agent CLIs (`run_claude`, `run_codex`, `run_gemini`, `run_opencode`), MCP config generation (`bus_args_json`, `codex_mcp_args`, `opencode_config`), hang/step watchdog (`watch_run`, `killtree`), `act` | `2793–3067` |
| `agent/runner.go` | `agent/runner_test.go` | Background `_runner` (`cmd__runner`, `runner_exit`, `start_runner`), `final_status`, status/question bounces, post-run dispatch | `4393–4942` |
| `agent/helpers.go` | `agent/helpers_test.go` | Worker delegation (`free_worker`, `dispatch_helpers`, `start_helper`, `helper_finish`, `cmd_helpers`) | `4247–4357`, `8255` |
| `agent/team.go` | `agent/team_test.go` | Team helpers (`team_json`, `team_summary`, `finish_team`, `team_after_run`, `member_out`, `team_close`, `cmd_team`, `cmd__wake`, `cmd_kick`, `supervise_teams`, `remote_member_task`, `remote_member_start`, `cmd__remote_member`) | `873–897`, `949–981`, `2101–2294`, `6047–6066`, `6131–6198`, `6239–6338`, `6515–6573`, `8213–8241` |
| `agent/review.go` | `agent/review_test.go` | `--verify` & `--autopilot` lifecycle (`verify_gaps`, `start_precheck`, `cmd__precheck`, `review_precheck`, `request_review`, `review_followup`, `review_fix_pass`, `cmd__fixpass`, `do_fix_pass`, `start_review_pairs`, `start_review_pair`, `review_pair_done`, `retry_review_slot`, `cmd__merge`, `cmd_autopilot`, `auto_stage`, `human_on_pr`) | `1335–1482`, `1541–1797`, `1862–2100`, `4358–4392` |
| `agent/investigation.go` | `agent/investigation_test.go` | Investigator queue & rotation (`pick_investigator`, `free_investigator`, `free_investigator_list`, `dispatch`, `start_investigation`, `inv_finish`, `cmd_inv`, `cmd_dispatch`) + Investigation Teams Phases 1–5 (`cmd__investigation_team`, `investigation_team_admit`, `investigation_team_tick`, `investigation_pipeline_tick`, `investigation_phase_pick`, `investigation_handoff`, `investigation_resume*`, `investigation_team_stop`) | `3168–4246`, `5497–5531`, `6015–6029` |
| `agent/run.go` | `agent/run_test.go` | Task allocation & creation (`allocate_run_ids`, `create_task`, `prepare_job_task`, `worktree_stash`), `resume_run`, `cmd_run`, `cmd_resume`, `cmd_stop`, `stop_all`, `cmd_clean`, `cmd__supervise`, `restart_interrupted`, `boot_id` | `4963–5496`, `5532–5739`, `5816–5847`, `5921–6001`, `6067–6130`, `6205–6238`, `6371–6386` |
| `agent/node.go` | `agent/node_test.go` | Node daemon & remote jobs (`cmd_node`, `node_poll`, `node_heartbeat_json`, `node_process_pending_launches`, `node_recover_tasks`, `node_scan_tasks`, `node_flush_reports`, `node_handle_controls`, `node_apply_answer`, `node_apply_kick`, `node_handle_stop`, `node_poll_remote_helpers`, `submit_remote_job`, `cmd_jobs`, `cmd_nodes`, `cmd_lead`, `cmd_web`) | `6574–7421` |
| `agent/node_sync.go` | `agent/node_sync_test.go` | Node artifact & log mirror sync (`node_sync_tasks`, `node_sync_task`, `node_send_sync`, `node_read_meta`, `node_sync_has_new_logs`, `node_mark_job_obsolete`) | `7422–7983` |
| `agent/authenticated.go` | `agent/authenticated_test.go` | Account & model authentication probes (`auth_classify`, `api_error_text`, `agent_auth_target`, `cmd__authcheck`, `auth_probe`, `cmd_authenticated`, `auth_providers`) | `8018–8212` |
| `agent/archive.go` | `agent/archive_test.go` | Task archiving, discard, transcript, status & inspection commands (`archive_index`, `archive_group`, `archive_one`, `cmd_archive`, `cmd_discard`, `cmd_transcript`, `cmd_status`, `cmd_show`, `cmd_log`, `render_log`, `cmd_agents`, `cmd__agents_json`, `cmd_ack`, `cmd_notes`, `cmd_bus`, `cmd_say`, `cmd_live`) | `5848–5920`, `6002–6014`, `6030–6046`, `7984–8017`, `8242–8474` |

---

## 3. Critical Mechanics: How Bats Tests Stay Untouched and Green in Every Round

Before writing code, every implementing agent must understand the four repository constraints that govern how `bin/agent`, `Makefile`, and `tests/unit/*.bats` interact:

### 3.1 Constraint A: `round_check` Counts `Added + Deleted` Lines
`agentbus round-size` computes `Lines = Added + Deleted` (excluding `*_gen.go`, `go.sum`, `package-lock.json`, `vendor/`, `.agents/reports/`).
- Therefore, `agent/errors_gen.go` is **free** (excluded by `*_gen.go`), so `agent/errors.json` can define as many typed operation causes as needed.
- Conversely, deleting lines from `bin/agent` **counts toward the round limit**. You must delete each migrated Bash section in the same round (or paired round) where its Go replacement is wired up, keeping `Added (Go + Go tests) + Deleted (Bash) <= 1,500` lines in every round.

### 3.2 Constraint B: `merge_blockers` Blocks Deleting Lines in Existing Test Files
In `bin/agent`, `round_test_changes()` checks:
```bash
changed=$(git -C "$repo" diff --numstat "$base" "$head" -- "${TEST_PATHSPECS[@]}" | awk '$2 > 0 { n++ } END { print n + 0 }')
```
If **any** existing `*.bats` or `*_test.go` file has even 1 deleted line (`$2 > 0`) against the round's base branch, `merge_blockers()` reports `it changes existing tests` and holds the PR for human approval.
- **Rule for every round:** Never modify or delete any `tests/unit/*.bats` file.
- **Rule for Go test files across rounds:** Each round creates its **own** new `agent/<module>_test.go` file (or purely appends new `func Test...` blocks at the end of an existing `*_test.go` file without editing or deleting any line committed in an earlier round).
- **Rule for `AUTO_MERGE_HUMAN_PATHS`:** Do not touch `install.sh`, `.github/`, `systemd/`, or `AGENTS.md` in intermediate rounds (Rounds 1–15); update `install.sh` only in the final round (Round 16).

### 3.3 Constraint C: How `bin/agent` Finds `build/agent` in All Test Environments
`tests/unit/helpers.bash` (which cannot be edited) copies `$REPO_ROOT/bin/agent` to `$HOME/.local/bin/agent` (and two-home tests copy it to `$NODE/.local/bin/agent`), while only compiling `agentbus` in `setup_file()`.
However, `helpers.bash` always prepends `$BATS_TEST_DIRNAME/stubs` (`<REPO_ROOT>/tests/unit/stubs`) to both `PATH` and `RUN_PATH`.
In Round 1:
1. Update `Makefile` so `unit-bats: build` compiles `agent/` to `build/agent` before running `tools/unit-bats.sh`, and exports `AGENT_GO_BIN="$(CURDIR)/build/agent"`.
2. Add a small helper `resolve_agent_go()` at the top of `bin/agent` that locates the compiled Go binary in all contexts:
   - `$AGENT_GO_BIN` if set and executable;
   - `$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/../build/agent` (when run or symlinked from `$REPO_ROOT/bin/agent`, including the `gh` symlink in `agent_gh.bats`);
   - `${stub_dir%/tests/unit/stubs}/build/agent` for any `*/tests/unit/stubs` entry in `$PATH` or `$RUN_PATH` (when copied to `$HOME/.local/bin/agent` or `$NODE/.local/bin/agent` in Bats);
   - If `build/agent` does not exist yet (e.g. a developer ran `bats tests/unit/foo.bats` directly without `make build`), build it once on the fly under `flock "$repo_root/build/agent.lock"`.

### 3.4 Constraint D: Supporting the 47 Bats Files That `source` or `sed` `bin/agent`
Out of 120 `.bats` files:
- **73 `.bats` files** invoke `agent <subcommand>` (or `gh`) purely as an external process. Once `main()` in `bin/agent` delegates `<subcommand>` to `exec "$AGENT_GO_BIN" "$cmd" "$@"`, those 73 Bats files immediately test the Go binary end-to-end.
- **43 `.bats` files** run `source "$REPO_ROOT/bin/agent"` (or `source "$HOME/.local/bin/agent"`) and invoke internal functions directly in Bash (sometimes after overriding shell variables like `INVESTIGATORS_TYPE` or mock shell functions like `bus()`, `systemctl()`, `start_runner()`, `unit_active()`, `node_request()`, `notify()`).
- **4 `.bats` files** (`node_agents.bats`, `node_report_agent.bats`, `job_attribution.bats`, `job_attribution_paths.bats`) extract a function body using `sed -n '/^<func>() {/,/^}/p' .../bin/agent` and source that snippet in a subshell with mocked helper functions.

To keep all 47 `source`/`sed` Bats files passing **while deleting the Bash business logic from `bin/agent`**:
1. Implement an internal subcommand `agent _fn <func_name> [args...]` in `agent/main.go`.
2. In `bin/agent`, define a 10-line bridge helper `_go_fn()`:
   - Exports any shell functions overridden by the calling Bats test (`export -f bus systemctl start_runner unit_active node_request notify notify_send act auto_stage stop_answers_watch dispatch dispatch_helpers team_after_run team_lead node_start_ack node_apply_answer node_handle_stop node_heartbeat_json free_investigator_list start_review_slot is_investigator agent_tier agent_cli_present agent_running_task node_activity_json log_error log_warn die lead_url node_time node_save grep 2>/dev/null || true`) and records which functions are shell overrides in `AGENT_SHELL_MOCKS`.
   - Passes serialized shell variables/associative arrays (`declare -p AGENT_TYPE AGENT_MODEL AGENT_EXTRA_ARGS NORMAL_AGENTS WORKER_AGENTS INVESTIGATORS_TYPE INVESTIGATORS_MODEL INVESTIGATORS_EXTRA_ARGS ROUND_MAX_LINES_REPO NODE_REPOS INSTANCE_CAPABILITIES 2>/dev/null || true`) in `AGENT_SHELL_DECLS`.
   - Invokes `"$AGENT_GO_BIN" _fn "$func_name" "$@"`.
   - For functions that mutate a caller shell variable in Bats (`create_task` / `prepare_job_task` setting `NEW_ID`, `fresh_session_due` setting `FRESH_WHY`, `watch_run` setting `HUNG`, `node_transition` / `node_reconcile` / `node_clear_control_acks` / `node_apply_answer` / `node_apply_kick` / `node_handle_stop` / `node_handle_reply` mutating `NODE_STATE`), `_fn` writes the updated variable to a small sidecar file (`$ AGENT_FN_OUT`) which the 1-line Bash wrapper evaluates.
3. For the 5 functions extracted by `sed -n '/^<func>() {/,/^}/p'` (`node_heartbeat_json`, `node_transition`, `submit_remote_job`, `github_repo_url`, `node_poll_remote_helpers`), keep each wrapper self-contained between `^<func>() {` and `^}` so `sed -n` extracts a complete, working wrapper that calls `build/agent _fn <func>`.

---

## 4. TDD & Verification Protocol for Every Round

Every round **must** follow this exact sequence (`docs/tdd.md` and `AGENTS.md`):

1. **Step 1 — $\color{red}{\textbf{RED}}$ Commit (Tests First):**
   - Add the round's operation error types and causes to `agent/errors.json` and run `make generate` (`agent/errors_gen.go`).
   - Add the new Go file `agent/<module>.go` containing exported/package types and function signatures with **zero-value bodies** (so it compiles cleanly and passes `doccheck`, without `panic("TODO")`).
   - Add `agent/<module>_test.go` with unit tests translated 1-to-1 from the round's target `tests/unit/*.bats` files (testing pure logic directly and process boundaries via package-level fake function variables).
   - Run the new Go tests and verify they fail on assertions:
     ```bash
     cd agent && go test -v -run 'Test<Module>' ./...
     ```
   - Commit **only** the $\color{red}{\textbf{RED}}$ state (`agent/errors.json`, `agent/errors_gen.go`, `agent/<module>.go` stubs, `agent/<module>_test.go`) with the failing test output at the end of the commit message:
     ```text
     RED: cd agent && go test -run 'Test<Module>' ./...
     --- FAIL: Test<Module>...
     ```
2. **Step 2 — $\color{green}{\textbf{GREEN}}$ Commit (Implementation + `bin/agent` Wire-up):**
   - Implement the functions in `agent/<module>.go` and wire the subcommands and `_fn` handlers into `agent/main.go`.
   - Replace the migrated Bash functions/subcommands in `bin/agent` with `_go_fn` wrappers and `exec "$AGENT_GO_BIN"` dispatch.
   - Run the Go unit tests with `-race`:
     ```bash
     cd agent && go test -race -v -run 'Test<Module>' ./...
     ```
   - Run the corresponding Bats baseline files to verify 100% compatibility:
     ```bash
     make unit-bats BATS_FILES="tests/unit/<file1>.bats tests/unit/<file2>.bats"
     ```
   - Run the full repository gate:
     ```bash
     make gate
     ```
   - Commit with `GREEN: cd agent && go test -race -run 'Test<Module>' ./...` and the passing output in the commit message.
3. **Step 3 — REFACTOR Commit (Optional, Tests Untouched):**
   - Tighten types, comments, and helpers in `agent/<module>.go` while keeping `git diff --stat -- '*_test.go'` empty.
   - Verify `make gate` passes with 0 findings.
4. **Step 4 — Pre-PR Round Budget & Hygiene Check:**
   - Check round size is $\le 1,500$ lines:
     ```bash
     build/agentbus round-size --repo . --base origin/main --exclude "*_gen.go go.sum package-lock.json vendor/ .agents/reports/"
     ```
   - Check no existing test files had lines deleted (`changed=0`):
     ```bash
     git diff --numstat origin/main HEAD -- ':(glob)**/*.bats' ':(glob)**/*_test.go' | awk '$2 > 0'
     ```

---

## 5. Step-by-Step Round Plan (16 Rounds, $\le 1,500$ Lines Each)

### Round 1: Go Module Scaffold, Typed Errors, Structured Logging & Config Loader
- **Goal:** Create the `agent/` Go module, integrate it into `Makefile`, implement the error model (`errors.json` $\rightarrow$ `errors_gen.go`), structured logging (`agent.jsonl` + journald + runner fd 3), and `agents.conf` / `secrets.env` loading.
- **Files Created / Modified:**
  - `Makefile` (+12 / -4 lines): Add `agent` to `GO_MODULES`, `unit-go`, `lint-docs`, `build`, `generate`, `check-generated`; make `unit-bats` depend on `build` and export `AGENT_GO_BIN`.
  - `agent/go.mod`: `module agent`, `go 1.22`.
  - `agent/errors.json` & `agent/errors_gen.go` (generated, 0 counted lines).
  - `agent/errors.go` (~160 lines) & `agent/errors_test.go` (~120 lines): `ErrorClass`, `OpError[C]`, `StorageError`, `ValueError`, `ProcessError`, `asError()`.
  - `agent/logging.go` (~240 lines) & `agent/logging_test.go` (~160 lines): `LogEvent` constants, `setupLogging`, `logInfo`/`logWarn`/`logError`, `tryCmd`, `jqSoft`, `die`.
  - `agent/types.go` (~140 lines) & `agent/types_test.go` (~90 lines): `AgentType`, `AgentTier`, `TaskStatus`, `TaskKind`, external command hook variables (`callBus`, `callGit`, `callGH`, `callSystemctl`, `callSystemdRun`, etc.) and `TestMain` guard.
  - `agent/config.go` (~260 lines) & `agent/config_test.go` (~180 lines): `Config` struct, `loadConfig` (evaluates `agents.conf` + env defaults + `_fn` `AGENT_SHELL_DECLS`), `agentProvider`, `agentTier`, `investigatorNames`, `isInvestigator`, `stopping`.
  - `agent/main.go` (~80 lines): `main()`, `_fn` dispatcher, `resolve_agent_go()` & `_go_fn()` in `bin/agent` (+45 / -40 lines).
- **Counted Line Budget:** ~1,450 lines (+1,410 added, -44 deleted).
- **Bats Tests Ported to Go (`agent/logging_test.go`, `agent/config_test.go`):**
  - `tests/unit/agent.bats` (logging & config cases: missing `agents.conf`, structured JSON log records in `logs/agent.jsonl`, `command_failed` on `die`, `try` logging failed side-effect commands with exit code and stderr, `jq_soft` logging malformed JSON).
  - `tests/unit/lint_shell_memory.bats` & `tests/unit/gate.bats` (verified via `make gate`).
- **What to Check:**
  - $\color{red}{\textbf{RED}}$: `cd agent && go test -run 'TestError|TestLog|TestConfig' ./...` fails on assertions.
  - $\color{green}{\textbf{GREEN}}$: `cd agent && go test -race ./...` passes; `build/doccheck agent/*.go` reports 0 undocumented declarations; `make gate` passes all 6 steps.

---

### Round 2: Task Metadata (`meta.env`), Locking, ID Resolution & Simple Bus/Status Commands
- **Goal:** Port `meta.env` locking/reading/writing (`meta_set`, `meta_load`, `task_field`), `job_transitions.jsonl` journaling on `STATUS` updates, task ID resolution (`resolve_id`), systemd unit status (`unit_name`, `unit_active`, `agent_running_task`), and simple CLI subcommands (`bus`, `say`, `ack`, `clean`, `notes`, `helpers`, `live`, `_agents-json`).
- **Files Created / Modified:**
  - `agent/meta.go` (~310 lines) & `agent/meta_test.go` (~290 lines).
  - `agent/Status/Simple CLI in agent/archive.go` (initial slice: `cmdBus`, `cmdSay`, `cmdAck`, `cmdClean`, `cmdNotes`, `cmdHelpers`, `cmdLive`, `cmdAgentsJSON`, `usage` — ~260 lines) & `agent/main_test.go` (~220 lines).
  - `bin/agent` (+25 / -240 lines): Replace `meta_file`, `meta_set`, `meta_load`, `task_field`, `resolve_id`, `unit_name`, `unit_active`, `agent_running_task`, `cmd_clean`, `cmd__agents_json`, `cmd_ack`, `cmd_notes`, `cmd_helpers`, `cmd_bus`, `cmd_say`, `cmd_live`, `usage` with `_go_fn` / `exec "$AGENT_GO_BIN"`.
- **Counted Line Budget:** ~1,345 lines (+1,105 added, -240 deleted).
- **Bats Tests Ported to Go (`agent/meta_test.go`, `agent/main_test.go`):**
  - `tests/unit/meta.bats` (20 concurrent `meta_set` writers on different keys all survive under `meta.env.lock`; stale optional `[run]` parameter is a no-op; `STATUS` update appends `{state, at}` to `job_transitions.jsonl` when `JOB_ID` is present; read fault on `meta.env` returns error instead of silently skipping `JOB_ID`).
  - `tests/unit/web.bats` (`_agents-json` output for configured agents and investigators).
  - `tests/unit/agent.bats` (`resolve_id` unambiguous fragment vs ambiguous error, `agent clean`, `agent ack`, `agent say` clearing `.team-*.stuck`, `agent notes`, `agent helpers`).
- **What to Check:**
  - Verify `formatBashQuoted` in `agent/meta.go` produces output compatible with both `bash -c 'source meta.env'` and `live/data.go`'s `unquote()`.
  - Run `cd agent && go test -race -run 'TestMeta|TestResolveID|TestSimpleCmds' ./...` and `make unit-bats BATS_FILES="tests/unit/meta.bats tests/unit/web.bats tests/unit/agent.bats"`.

---

### Round 3: `agent_gh` GitHub Trust Filter, Review Bylines & Repository URL Helpers
- **Goal:** Port `agent_gh` (invoked when `filepath.Base(os.Args[0]) == "gh"` or `agent _gh`) and the GitHub origin/trust helpers (`github_repo_url`, `github_owner`, `github_trusted`, `trusted_prs`, `review_byline`, `agents_gh_env`) into `agent/gh.go`.
- **Files Created / Modified:**
  - `agent/gh.go` (~390 lines) & `agent/gh_test.go` (~360 lines).
  - `bin/agent` (+25 / -265 lines): Replace `agent_gh` (lines 16–200) and `github_repo_url`, `github_owner`, `agents_gh_env`, `review_byline`, `github_trusted`, `trusted_prs` with Go calls (preserving the self-contained `github_repo_url() { ... }` block for `sed -n` in `job_attribution.bats` and `job_attribution_paths.bats`).
- **Counted Line Budget:** ~1,040 lines (+775 added, -265 deleted).
- **Bats Tests Ported to Go (`agent/gh_test.go`):**
  - `tests/unit/agent_gh.bats` (7 tests: filtering unknown accounts' comments/reviews, filtering `"ignore this mate"` comments, blocking GraphQL queries for `comments|reviews|reviewThreads|timelineItems|body`, filtering `gh pr list` / `gh issue list` and `gh pr view|diff|checkout` from forks or untrusted authors, rejecting `--template` and `--web`, passing `--jq` expressions).
  - `tests/unit/github_trust.bats` (4 tests: fork PRs with same branch name ignored and logged as `pr_untrusted_ignored`).
  - `tests/unit/review_byline.bats` (3 tests: prepending `AGENT_REVIEW_BYLINE` on `gh api .../pulls/N/reviews --input`, idempotence when byline is already present, `review_byline` formatting).
  - `tests/unit/job_attribution.bats` & `tests/unit/job_attribution_paths.bats` (`github_repo_url` canonicalization of `git@github.com:...`, `ssh://git@github.com/...`, `https://github.com/...`, ignoring non-GitHub origins, using first origin URL).
- **What to Check:**
  - Symlink test in Go (`os.Args[0] = ".../gh"`) and Bats (`make unit-bats BATS_FILES="tests/unit/agent_gh.bats tests/unit/github_trust.bats tests/unit/review_byline.bats tests/unit/job_attribution.bats tests/unit/job_attribution_paths.bats"`).

---

### Round 4: Questions, Answers, Notifications & Typed Bus Notices
- **Goal:** Port question parsing, recommendation checking, answer validation/expansion, systemd `.path` watches, VS Code remote opener, remote question staging (`_remote_questions`, `_remote_questions_ready`, `supervise_remote_questions`), and notification delivery (`notify`, `notify_send`, `notify_failure_keep`, `notice`, `push_text`).
- **Files Created / Modified:**
  - `agent/questions.go` (~380 lines) & `agent/questions_test.go` (~340 lines).
  - `agent/notify.go` (~210 lines) & `agent/notify_test.go` (~170 lines).
  - `bin/agent` (+35 / -385 lines): Replace lines 566–853 and 898–948 and 3068–3167 with `_go_fn` / `exec "$AGENT_GO_BIN"`.
- **Counted Line Budget:** ~1,485 lines (+1,100 added, -385 deleted).
- **Bats Tests Ported to Go (`agent/questions_test.go`, `agent/notify_test.go`):**
  - `tests/unit/agent.bats` (question counting, `validate_answers`, `questions_missing_recommendations`, `expand_answers` with `ok` verdict, `**ANSWERED**` marker, `questions_from_final`, `questions_from_nostatus`, `notify-failures.jsonl` tracking).
  - `tests/unit/remote_questions_atomic.bats` (1 test: staging remote questions atomically with permissions `0600`/`0700`).
  - `tests/unit/remote_questions_preparation_retry.bats` (3 tests: cleanup on staging failure, retry recovery).
  - `tests/unit/remote_control_notifications.bats` (1 test: remote question notification links and `<task>@<node>` answer command).
  - `tests/unit/remote_answer_round.bats` & `tests/unit/remote_control_proxy.bats` (`cmd_answer` on `<task>@<node>` and local remote-member proxy).
- **What to Check:**
  - Run `cd agent && go test -race -run 'TestQuestions|TestNotify|TestRemoteQuestions' ./...` and `make unit-bats BATS_FILES="tests/unit/remote_questions_atomic.bats tests/unit/remote_questions_preparation_retry.bats tests/unit/remote_control_notifications.bats tests/unit/remote_answer_round.bats tests/unit/remote_control_proxy.bats"`.

---

### Round 5: Limits, Quota/Network Classification, Wake-Loop Guard, Tokens & `agent wake`
- **Goal:** Port stderr limit classification (`limit_kind`, `limit_line`, `last_error_line`, `quota_reset_seconds`, `duration_seconds`), wake-loop detection (`wake_loop_applies`, `wake_loop_check`), task pausing (`pause_task`, `schedule_wake`, `resume_paused`), context/session hygiene (`fresh_session_due`, `long_run_end`, `runs_usage`), `cmd_tokens`, and `cmd_wake`.
- **Files Created / Modified:**
  - `agent/limits.go` (~410 lines) & `agent/limits_test.go` (~390 lines).
  - `bin/agent` (+30 / -340 lines): Replace lines 1798–1827, 2295–2497, 6339–6370, 6407–6514.
- **Counted Line Budget:** ~1,170 lines (+830 added, -340 deleted).
- **Bats Tests Ported to Go (`agent/limits_test.go`):**
  - `tests/unit/limits.bats` (3 tests: HTTP 429 rate limit vs quota, tail line limit `AGENT_LIMIT_TAIL_LINES`).
  - `tests/unit/network.bats` (3 tests: DNS/network timeout pauses for `AGENT_NETWORK_PAUSE_MIN`, fails after `AGENT_NETWORK_MAX_PAUSES`).
  - `tests/unit/resilience.bats` (3 tests: model at capacity short pause, local fork/socket errors are not provider limits).
  - `tests/unit/run_notes.bats` (3 tests: workspace monthly budget ISO reset timestamp parsing, `node_job_result`).
  - `tests/unit/remote_fixes.bats` (3 tests: `wake_loop_applies` watches local members only, not remote ones).
  - `tests/unit/fresh_session.bats` (3 tests) & `tests/unit/fresh_relative.bats` (1 test): `fresh_session_due` and `long_run_end`.
  - `tests/unit/tokens.bats` (5 tests: `agent tokens` and `agent tokens --runs` aggregation across tasks, helpers, and investigations).
  - `tests/unit/wake_cmd.bats` (4 tests: `agent wake` on local and remote tasks).
- **What to Check:**
  - Run `cd agent && go test -race -run 'TestLimit|TestQuota|TestWakeLoop|TestFresh|TestTokens|TestWake' ./...` and `make unit-bats BATS_FILES="tests/unit/limits.bats tests/unit/network.bats tests/unit/resilience.bats tests/unit/run_notes.bats tests/unit/remote_fixes.bats tests/unit/fresh_session.bats tests/unit/fresh_relative.bats tests/unit/tokens.bats tests/unit/wake_cmd.bats"`.

---

### Round 6: Rounds, Skipped Tests, PR Format Checks & Merge Blockers
- **Goal:** Port round size limits (`round_limit`, `round_soft`, `round_base_ref`, `round_check`, `cmd_round_size`), skipped test detection (`skips_in_repo`, `skips_added`, `pr_skipped_section`, `skip_note`), review text parsers (`count_of`, `findings_count`, `review_verdict`, `review_task_verdict`), PR formatting and merge blockers (`round_test_changes`, `pr_format_issues`, `merge_blockers`, `round_open_prs`, `round_pr_note`, `round_pr_of_team`, `round_pr_supersede`, `attribution_block`).
- **Files Created / Modified:**
  - `agent/rounds.go` (~430 lines) & `agent/rounds_test.go` (~410 lines).
  - `bin/agent` (+30 / -330 lines): Replace lines 982–1158, 1181–1233, 1286–1334, 1483–1540.
- **Counted Line Budget:** ~1,200 lines (+870 added, -330 deleted).
- **Bats Tests Ported to Go (`agent/rounds_test.go`):**
  - `tests/unit/findings.bats` (1 test: `findings_count` across all critical/blocker/major formats).
  - `tests/unit/autopilot_review.bats` (`review_verdict` and `round_test_changes` cases).
  - `tests/unit/github_format.bats` (2 tests: `pr_format_issues` on unlinked commits/PRs, untagged code blocks, plain RED/GREEN).
  - `tests/unit/autopilot_merge.bats` (`merge_blockers` on `AUTO_MERGE_HUMAN_PATHS` and changed existing tests).
  - `tests/unit/skips.bats` (7 tests: `skips_in_repo`, `skips_added`, `pr_skipped_section` across Go, Bats, Python, JS/TS, Rust).
  - `tests/unit/rounds.bats` (8 tests) & `tests/unit/round_reports.bats` (1 test: `.agents/reports/` always excluded).
  - `tests/unit/one_pr_per_round.bats` (3 tests: `round_open_prs`) & `tests/unit/round_handover.bats` (4 tests: `round_pr_supersede`).
- **What to Check:**
  - Run `cd agent && go test -race -run 'TestRound|TestSkip|TestFinding|TestVerdict|TestPRFormat|TestMergeBlocker' ./...` and corresponding Bats files.

---

### Round 7: Prompt Rendering, Git Identity, CLI Execution & Hang Watchdog
- **Goal:** Port prompt rendering (`render`, `build_prompt`, `git_identity`, `commit_trailers`, `agents_md_text`, `privileges_text`, `peers_list`, `checkpoints_latest`, `round_text_strip`) into `agent/prompts.go` and CLI execution + watchdog (`act`, `killtree`, `watch_run`, `activity_progress`, `cli_prompt`, `is_independent_investigation`, `bus_args_json`, `codex_mcp_args`, `opencode_config`, `run_claude`, `run_codex`, `run_gemini`, `run_opencode`) into `agent/cli_exec.go`.
- **Files Created / Modified:**
  - `agent/prompts.go` (~310 lines) & `agent/prompts_test.go` (~220 lines).
  - `agent/cli_exec.go` (~340 lines) & `agent/cli_exec_test.go` (~260 lines).
  - `bin/agent` (+30 / -340 lines): Replace lines 854–872, 2498–3067, 6387–6406.
- **Counted Line Budget:** ~1,500 lines (+1,160 added, -340 deleted).
- **Bats Tests Ported to Go (`agent/prompts_test.go`, `agent/cli_exec_test.go`):**
  - `tests/unit/identity.bats` (3 tests: `git_identity` default `.invalid` email, SSH signing key configuration, `commit_trailers`).
  - `tests/unit/prompt_checkpoints.bats` (1 test: `checkpoints_latest`), `tests/unit/round_only.bats` (2 tests: `round_text_strip`), `tests/unit/context_hygiene.bats` (2 tests), `tests/unit/coordination.bats` (1 test).
  - `tests/unit/prompt_file.bats` (2 tests: prompts exceeding `AGENT_PROMPT_ARG_MAX` passed via `.agents/prompt-N.md` and excluded from git).
  - `tests/unit/long_run_cut.bats` (2 tests: `watch_run` step limit `AGENT_LONG_RUN_HARD` and hang timeout `AGENT_HANG_MIN`).
  - `tests/unit/investigation_mcp_off.bats` (2 tests), `tests/unit/investigation_team_isolation.bats` (4 tests), `tests/unit/investigation_team_opencode_schema.bats` (1 test): MCP config isolation for independent investigators across Claude, Codex, Gemini, and OpenCode.
- **What to Check:**
  - Ensure rendered prompts contain zero unresolved `{{` placeholders (verified by `agent.bats` and `TestBuildPromptNoPlaceholdersLeft`).

---

### Round 8: Worker Requests (`delegate`), Background Runner (`_runner`), Status & Archive/Transcript
- **Goal:** Port worker delegation (`free_worker`, `dispatch_helpers`, `start_helper`, `helper_finish`) into `agent/helpers.go`, the background task runner (`cmd__runner`, `runner_exit`, `start_runner`, `final_status`) into `agent/runner.go`, and task archiving/transcript/status (`archive_index`, `archive_group`, `archive_one`, `cmd_archive`, `cmd_discard`, `cmd_transcript`, `cmd_status`, `cmd_show`, `cmd_log`, `cmd_agents`) into `agent/archive.go`.
- **Files Created / Modified:**
  - `agent/helpers.go` (~140 lines) & `agent/helpers_test.go` (~110 lines).
  - `agent/runner.go` (~360 lines) & `agent/runner_test.go` (~260 lines).
  - `agent/archive.go` (~240 lines) & `agent/archive_test.go` (~160 lines).
  - `bin/agent` (+20 / -210 lines): Replace lines 4247–4942 (runner & helpers) and 5848–5920, 6002–6014, 8257–8465 (status, show, log, agents, transcript, archive, discard).
- **Counted Line Budget:** ~1,480 lines (+1,270 added, -210 deleted; remaining Bash lines of `cmd__runner` deleted in Round 9).
- **Bats Tests Ported to Go (`agent/helpers_test.go`, `agent/runner_test.go`, `agent/archive_test.go`):**
  - `tests/unit/agent.bats` (solo task lifecycle `DONE`/`FAILED`/`QUESTIONS`, hang watchdog strike progression, worker delegation provider load balancing, `agent archive` with 7z resume chains, `agent discard`, `agent transcript`, `agent status`, `agent show`, `agent log` structured rendering).
  - `tests/unit/member_out.bats` (unknown `STATUS:` bounce once via `prompts/status-bounce.md`).
  - `tests/unit/autopilot_questions.bats` (2 tests: `--autopilot` bounces questions once via `prompts/autopilot-questions.md`).
- **What to Check:**
  - Verify `runner_exit` trap semantics on SIGTERM (`rc=143`) when a solo or team task is kicked vs unexpectedly killed.

---

### Round 9: Team Orchestration, Post-Run Team Logic, Wake, Kick & Close
- **Goal:** Port team management (`team_json`, `team_lead`, `team_done_flag`, `team_tasks`, `team_summary`, `finish_team`, `team_waits_on_others`, `team_after_run`, `member_out`, `cmd__wake`, `cmd_kick`, `cmd_team`, `team_close`, `supervise_teams`, `supervise_member_jobs`, `remote_member_task`, `remote_member_start`, `remote_start_check`, `handover_failed`, `cmd__remote_member`) into `agent/team.go`.
- **Files Created / Modified:**
  - `agent/team.go` (~460 lines) & `agent/team_test.go` (~420 lines).
  - `bin/agent` (+30 / -580 lines): Replace team functions in lines 873–897, 949–990, 1845–1874, 2101–2294, 4507–4924 (remaining `_runner` Bash body), 6047–6066, 6111–6198, 6239–6338, 6515–6573, 8213–8241.
- **Counted Line Budget:** ~1,490 lines (+910 added, -580 deleted).
- **Bats Tests Ported to Go (`agent/team_test.go`):**
  - `tests/unit/team_close.bats` (3 tests: `agent team <id> --close` stops local members and cancels/revokes remote member jobs; abandoned team notification after `AGENT_TEAM_ABANDON_MIN`).
  - `tests/unit/kick.bats` (3 tests: `agent kick` on solo task stops it, on team member puts it to sleep and releases nothing).
  - `tests/unit/remote_control_kick_exit.bats` (3 tests: kicking a solo or team job records `stopped`/`sleeping` without false failed `runner_exit`).
  - `tests/unit/member_out.bats` (6 tests: failed/excluded member releases board items with progress notes and notifies team).
  - `tests/unit/pause_notify.bats` (3 tests: remote member pause notifies coordinator and releases board items on long pause).
  - `tests/unit/handover_failed.bats` (3 tests: `remote_start_check` and `handover_failed` when lead is unreachable or base unpushed).
  - `tests/unit/remote_proxy.bats` (2 tests: remote member proxy task in `agent status`).
- **What to Check:**
  - Run `cd agent && go test -race -run 'TestTeam|TestKick|TestMemberOut|TestRemoteMember' ./...` and all 7 Bats files above.

---

### Round 10: `--verify` and `--autopilot` Prechecks, Review Pairs, Fix Passes & Auto-Merge
- **Goal:** Port the `--verify` and `--autopilot` state machine (`verify_gaps`, `gate_at`, `start_task_unit`, `cmd__merge`, `cmd_autopilot`, `supervise_review_waits`, `review_gate_cmd`, `auto_stage`, `human_on_pr`, `gate_progress`, `pair_stage`, `start_precheck`, `team_wait_settled`, `cmd__precheck`, `review_precheck`, `review_followup`, `review_fix_pass`, `cmd__fixpass`, `do_fix_pass`, `review_without_verdict`, `retry_review_slot`, `review_pair_done`, `start_review_pairs`, `start_review_pair`, `start_review_slot`, `request_review`) into `agent/review.go`.
- **Files Created / Modified:**
  - `agent/review.go` (~480 lines) & `agent/review_test.go` (~420 lines).
  - `bin/agent` (+25 / -565 lines): Replace lines 1335–1482, 1541–1797, 1875–2100, 4358–4392.
- **Counted Line Budget:** ~1,490 lines (+925 added, -565 deleted).
- **Bats Tests Ported to Go (`agent/review_test.go`):**
  - `tests/unit/verify_loop.bats` (3 tests: `verify_gaps`, review findings waking team for fix pass, `--last-team`).
  - `tests/unit/auto_stage.bats` (2 tests: `AUTO_STAGE` gate progress and review pair stage updates).
  - `tests/unit/autopilot_review.bats` (8 tests: precheck gate failure bounce, two reviewers in parallel, fix pass on `needs-fixes`).
  - `tests/unit/autopilot_merge.bats` (5 tests: `--match-head-commit` merge, post-merge gate check, starting next round of plan).
  - `tests/unit/review_pair_random.bats` (4 tests: random draw without replacement across free investigators).
  - `tests/unit/review_resilience.bats` (7 tests: replacement reviewer when one reviewer finishes without a verdict or hits quota; closed team with all members finished still goes to review).
- **What to Check:**
  - Run `cd agent && go test -race -run 'TestReview|TestPrecheck|TestAutopilot|TestVerify' ./...` and the 6 Bats files above.

---

### Round 11: Investigator Queue, Rotation & Investigation Teams (Phase 1, Admission & Supervision)
- **Goal:** Port investigator rotation and queue dispatch (`pick_investigator`, `remember_investigator`, `free_investigator`, `free_investigator_list`, `_investigator_is_busy`, `dispatch`, `start_investigation`, `inv_finish`, `maybe_resume_worker`, `cmd_inv`, `cmd_dispatch`) and Investigation Team validation + Phase 1 (`validate_investigation_team`, `validate_investigation_topic`, `investigation_phase1_write`, `investigation_published`, `investigation_phase1_recover`, `investigation_team_tick`, `investigation_team_admit`, `investigation_teams_admit`, `investigation_team_stop`, `investigation_teams_cleanup`, `cmd__investigation_team`) into `agent/investigation.go`.
- **Files Created / Modified:**
  - `agent/investigation.go` (Part 1: ~470 lines) & `agent/investigation_test.go` (Part 1: ~430 lines).
  - `bin/agent` (+25 / -560 lines): Replace lines 3168–3522, 3779–3798, 3912–4246, 5497–5531, 6015–6029.
- **Counted Line Budget:** ~1,485 lines (+925 added, -560 deleted).
- **Bats Tests Ported to Go (`agent/investigation_test.go`):**
  - `tests/unit/rotation.bats` (2 tests: provider-aware investigator rotation and `last-investigator` persistence).
  - `tests/unit/investigation_team_cli.bats` (12 tests) & `tests/unit/investigation_team_validation.bats` (13 tests): CLI flag validation and fail-fast refusals.
  - `tests/unit/investigation_team_stdin.bats` (2 tests: multiline topic from stdin and `-f`).
  - `tests/unit/investigation_single_investigator.bats` (2 tests).
  - `tests/unit/investigation_team_runtime.bats` (8 tests: concurrent Phase 1 launch from frozen commit SHA).
  - `tests/unit/investigation_team_admission.bats` (8 tests: all-or-nothing cohort admission under `dispatch.lock`, `phase1.json` skeleton before `INVESTIGATION_LEASE=1`).
  - `tests/unit/investigation_team_publication_retry.bats` (2 tests: transient `git ls-remote` retry vs invalid commit failure).
  - `tests/unit/investigation_team_supervision.bats` (4 tests), `tests/unit/investigation_team_regressions.bats` (2 tests), `tests/unit/investigation_team_reboot_regression.bats` (2 tests).
- **What to Check:**
  - Run `cd agent && go test -race -run 'TestInvestigat|TestRotation' ./...` and all 11 Bats files above.

---

### Round 12: Investigation Teams Phases 2–5 (Merge, Review, Handoff, Advisory Resume & Recovery)
- **Goal:** Complete `agent/investigation.go` with Phases 2–4 pipeline (`investigation_field`, `investigation_completed_wait`, `investigation_handoff`, `investigation_phase_start`, `investigation_phase_choose`, `investigation_phase_failed`, `investigation_pipeline_tick`, `investigation_phase_pick`, `investigation_teams_tick`) and Phase 5 advisory resume (`investigation_resume_write`, `investigation_resume_tick`, `investigation_resume_admit`, `investigation_resume`).
- **Files Created / Modified:**
  - `agent/investigation.go` (Part 2: +390 lines) & `agent/investigation_pipeline_test.go` (~460 lines).
  - `bin/agent` (+20 / -495 lines): Replace lines 3215–3371 and 3523–3911.
- **Counted Line Budget:** ~1,365 lines (+870 added, -495 deleted).
- **Bats Tests Ported to Go (`agent/investigation_pipeline_test.go`):**
  - `tests/unit/investigation_team_phase_selection.bats` (11 tests: deterministic `AGENT_INVESTIGATION_SEED` and `/dev/urandom` uniform draws, merger/reviewer eligibility and replacement).
  - `tests/unit/investigation_team_phase_queue.bats` (4 tests: waiting in capacity queue when candidates are in `QUOTA_UNTIL`).
  - `tests/unit/investigation_team_handoff.bats` (8 tests: PR creation, precheck bounce, review verdict, `--investigation` PR comment, notification).
  - `tests/unit/investigation_single_pipeline.bats` (2 tests), `tests/unit/investigation_independent_labels.bats` (1 test), `tests/unit/investigation_terminal_completion.bats` (2 tests), `tests/unit/investigation_completed_legacy.bats` (4 tests).
  - `tests/unit/investigation_clarification.bats` (1 test), `tests/unit/investigation_pipeline_clarification.bats` (2 tests), `tests/unit/investigation_prompt_clarification_resume.bats` (2 tests), `tests/unit/investigation_independent_resume_prompt.bats` (1 test), `tests/unit/investigation_resume_independence.bats` (1 test).
  - `tests/unit/investigation_team_advisory_resume.bats` (4 tests), `tests/unit/investigation_team_resume_boundaries.bats` (3 tests), `tests/unit/investigation_team_resume_recovery.bats` (4 tests), `tests/unit/investigation_team_recovery_regressions.bats` (8 tests).
- **What to Check:**
  - Run `cd agent && go test -race -run 'TestInvestigation' ./...` and all `tests/unit/investigation_*.bats` files.

---

### Round 13: Task Creation, Worktree Checkout, Resume, `cmd_run`, `cmd_stop` & `_supervise`
- **Goal:** Port task ID allocation and worktree creation (`allocate_run_ids`, `create_task`, `drop_unstarted_task`, `job_refusal`, `prepare_job_task`, `discard_task`, `abandon_run`, `worktree_stash`), session/team resume (`resume_history`, `resume_run`, `resume_abandon`), `cmd_run`, `cmd_answer`, `cmd__answers_saved`, `cmd_resume`, `stop_all`, `cmd_stop`, and `cmd__supervise` (`boot_id`, `restart_interrupted`) into `agent/run.go`.
- **Files Created / Modified:**
  - `agent/run.go` (~520 lines) & `agent/run_test.go` (~440 lines).
  - `bin/agent` (+25 / -510 lines): Replace lines 4963–5496, 5532–5847, 5921–6001, 6067–6110, 6205–6238, 6288–6317, 6371–6386.
- **Counted Line Budget:** ~1,495 lines (+985 added, -510 deleted; remaining Bash lines of `resume_run` deleted in Round 14).
- **Bats Tests Ported to Go (`agent/run_test.go`):**
  - `tests/unit/node_checkout.bats` (18 tests: `create_task` at specific commit SHA, `prepare_job_task` repository allow-list `NODE_REPOS`, unknown/investigator/busy agent refusal, rollback on worktree failure).
  - `tests/unit/resume_alloc.bats` (5 tests: `allocate_run_ids` monotonic timestamp collision avoidance under `task-ids.lock`, predecessor metadata preservation).
  - `tests/unit/resume_members.bats` (2 tests: `--resume --team` member subset and new member branching from coordinator).
  - `tests/unit/resume_stash.bats` (1 test: `worktree_stash` carrying uncommitted and `--intent-to-add` changes into resumed worktree).
  - `tests/unit/restart.bats` (4 tests: `boot_id` and `restart_interrupted` resuming up to `AGENT_RESTART_MAX` times).
- **What to Check:**
  - Run `cd agent && go test -race -run 'TestRun|TestCreateTask|TestPrepareJob|TestResume|TestRestart' ./...` and the 5 Bats files above.

---

### Round 14: Node Daemon, Heartbeats, Job Launches, Recovery Fence & Remote Controls
- **Goal:** Port the node daemon and remote job CLI (`node_jobs_load`, `node_jobs_save`, `node_lock`, `lead_url`, `node_request`, `submit_remote_job`, `cmd_jobs`, `cmd_nodes`, `cmd_lead`, `cmd_web`, `agent_cli_present`, `node_activity_json`, `node_job_result`, `node_heartbeat_json`, `node_select_agent`, `node_time`, `node_map_state`, `node_transition`, `node_reconcile`, `node_start_ack`, `node_process_pending_launches`, `node_recover_tasks`, `node_bind_task`, `node_scan_tasks`, `node_flush_reports`, `node_clear_control_acks`, `node_apply_answer`, `node_apply_kick`, `node_handle_controls`, `node_handle_stop`, `node_handle_reply`, `node_poll_remote_helpers`, `node_poll`, `cmd_node`) into `agent/node.go`.
- **Files Created / Modified:**
  - `agent/node.go` (~540 lines) & `agent/node_test.go` (~430 lines).
  - `bin/agent` (+30 / -495 lines): Replace lines 6574–7421 (and remaining lines of `resume_run`). Keep the 5 self-contained 5-line `_go_fn` blocks for `node_heartbeat_json`, `node_transition`, `submit_remote_job`, `github_repo_url`, and `node_poll_remote_helpers` so the 4 `sed -n` Bats tests continue to extract and execute them via `build/agent _fn`.
- **Counted Line Budget:** ~1,495 lines (+1,000 added, -495 deleted).
- **Bats Tests Ported to Go (`agent/node_test.go`):**
  - `tests/unit/lead.bats` (3 tests), `tests/unit/lead_cli.bats` (4 tests), `tests/unit/lead_cli_context.bats` (5 tests), `tests/unit/web_jobs.bats` (1 test).
  - `tests/unit/node_agents.bats` (1 test), `tests/unit/node_activity.bats` (3 tests), `tests/unit/node_report_agent.bats` (1 test), `tests/unit/node_safety.bats` (4 tests).
  - `tests/unit/node_prelaunch_cancel.bats` (2 tests), `tests/unit/node_cancel_exit.bats` (4 tests), `tests/unit/node_recovery_fence.bats` (6 tests), `tests/unit/node_review_boundaries.bats` (7 tests), `tests/unit/node_large_assignment.bats` (1 test).
  - `tests/unit/node_helper_hooks.bats` (12 tests), `tests/unit/delegate_jobs.bats` (7 tests).
  - `tests/unit/remote_control_cancel_order.bats` (1 test), `tests/unit/remote_control_heartbeat_limits.bats` (3 tests), `tests/unit/remote_control_review.bats` (7 tests), `tests/unit/remote_control_validation.bats` (7 tests), `tests/unit/remote_controls.bats` (3 tests), `tests/unit/remote_control_web.bats` (1 test), `tests/unit/node_daemon.bats` (15 tests), `tests/unit/node_two_homes.bats` (11 tests).
- **What to Check:**
  - Run `cd agent && go test -race -run 'TestNode|TestRemoteJob|TestRemoteControl' ./...` and all node/remote Bats files.

---

### Round 15: Node Task/Log Mirror Sync (`node_sync.go`) & Account Authentication (`authenticated.go`)
- **Goal:** Port node artifact and log mirror sync (`node_sync_save_json`, `node_read_meta`, `node_mark_job_obsolete`, `node_sync_terminal_unchanged`, `node_sync_has_new_logs`, `node_send_sync`, `node_sync_task`, `node_sync_tasks`) into `agent/node_sync.go`, and account authentication probes (`auth_classify`, `api_error_text`, `agent_auth_target`, `cmd__authcheck`, `auth_probe`, `opencode_model_for`, `cmd_authenticated`, `auth_providers`) into `agent/authenticated.go`.
- **Files Created / Modified:**
  - `agent/node_sync.go` (~340 lines) & `agent/node_sync_test.go` (~310 lines).
  - `agent/authenticated.go` (~190 lines) & `agent/authenticated_test.go` (~150 lines).
  - `bin/agent` (+15 / -480 lines): Replace lines 7422–7930 and 8018–8212 (and remaining retired Bash helpers from Round 14).
- **Counted Line Budget:** ~1,485 lines (+1,005 added, -480 deleted).
- **Bats Tests Ported to Go (`agent/node_sync_test.go`, `agent/authenticated_test.go`):**
  - `tests/unit/node_sync.bats` (6 tests), `tests/unit/node_sync_freshness.bats` (3 tests), `tests/unit/node_sync_limits.bats` (4 tests), `tests/unit/node_sync_review.bats` (8 tests), `tests/unit/node_sync_state.bats` (1 test), `tests/unit/node_sync_tails.bats` (2 tests), `tests/unit/node_sync_two_homes.bats` (1 test).
  - `tests/unit/authenticated.bats` (3 tests: per-agent model probe, HTTP 401/402/429 classification with provider message, `--providers` mode).
- **What to Check:**
  - Run `cd agent && go test -race -run 'TestNodeSync|TestAuth' ./...` and `make unit-bats BATS_FILES="tests/unit/node_sync*.bats tests/unit/authenticated.bats"`.

---

### Round 16: Final `bin/agent` Cutover, `install.sh` Build Integration & Documentation
- **Goal:** Retire the remaining legacy Bash function bodies from `bin/agent` so `bin/agent` contains only the minimal `exec` launcher + `_go_fn` Bats `source`/`sed` shim (~110 lines total), update `install.sh` to build `agent/` with `go build`, add Go unit tests for `install.sh` unit generation (`lead_install.bats` / `node_install.bats` equivalents in `agent/install_test.go`), and update `README.md`.
- **Files Created / Modified:**
  - `agent/install_test.go` (~120 lines): Go unit tests mirroring `lead_install.bats` and `node_install.bats`.
  - `bin/agent` (+10 / -1,150 lines): Remove all remaining retired Bash code, leaving only the `exec "$AGENT_GO_BIN" "$@"` entry point and the `_go_fn` wrappers for the 47 Bats files that `source` or `sed` `bin/agent`.
  - `install.sh` (+6 / -2 lines): Build `agent` from `$SRC/agent` alongside `agentbus` and `agentlive`. *(Note: touching `install.sh` is intentionally placed in the final round because `install.sh` is in `AUTO_MERGE_HUMAN_PATHS`.)*
  - `README.md` (~30 lines): Update architecture table and build instructions to describe `agent/` as a Go module.
- **Counted Line Budget:** ~1,318 lines (+166 added, -1,152 deleted).
- **Bats Tests Ported to Go (`agent/install_test.go`):**
  - `tests/unit/lead_install.bats` (2 tests) & `tests/unit/node_install.bats` (4 tests).
- **What to Check:**
  - All 120 Bats files (483 `@test` cases) have 1-to-1 Go unit test equivalents in `agent/*_test.go`.
  - `cd agent && go test -race ./...` passes 100% of Go unit tests in seconds.
  - `make unit-bats` passes all 483 `@test` cases with 0 modifications to `tests/unit/**`.
  - `make gate` passes all 6 steps (`check-generated`, `lint-shell`, `lint-go`, `lint-docs`, `unit-go`, `unit-bats`) with zero warnings or suppressions.

---

## 6. Complete Round-by-Round Summary Matrix

| Round | Target `agent/` Files | Target `agent/*_test.go` Files | Bats Files Ported to Go (`tests/unit/*.bats`) | Est. Counted Lines (`+Add / -Del`) |
|---|---|---|---|---|
| **1** | `go.mod`, `errors.json`, `errors.go`, `logging.go`, `types.go`, `config.go`, `main.go` | `errors_test.go`, `logging_test.go`, `types_test.go`, `config_test.go` | `agent.bats` (config/log), `gate.bats`, `lint_shell_memory.bats` | ~1,450 (`+1410 / -40`) |
| **2** | `meta.go`, `archive.go` (simple cmds) | `meta_test.go`, `main_test.go` | `meta.bats`, `web.bats`, `agent.bats` (meta/simple cmds) | ~1,345 (`+1105 / -240`) |
| **3** | `gh.go` | `gh_test.go` | `agent_gh.bats`, `github_trust.bats`, `review_byline.bats`, `job_attribution.bats`, `job_attribution_paths.bats` | ~1,040 (`+775 / -265`) |
| **4** | `questions.go`, `notify.go` | `questions_test.go`, `notify_test.go` | `remote_questions_atomic.bats`, `remote_questions_preparation_retry.bats`, `remote_control_notifications.bats`, `remote_answer_round.bats`, `remote_control_proxy.bats` | ~1,485 (`+1100 / -385`) |
| **5** | `limits.go` | `limits_test.go` | `limits.bats`, `network.bats`, `resilience.bats`, `run_notes.bats`, `remote_fixes.bats`, `fresh_session.bats`, `fresh_relative.bats`, `tokens.bats`, `wake_cmd.bats` | ~1,170 (`+830 / -340`) |
| **6** | `rounds.go` | `rounds_test.go` | `findings.bats`, `github_format.bats`, `skips.bats`, `rounds.bats`, `round_reports.bats`, `one_pr_per_round.bats`, `round_handover.bats` | ~1,200 (`+870 / -330`) |
| **7** | `prompts.go`, `cli_exec.go` | `prompts_test.go`, `cli_exec_test.go` | `identity.bats`, `prompt_checkpoints.bats`, `round_only.bats`, `context_hygiene.bats`, `coordination.bats`, `prompt_file.bats`, `long_run_cut.bats`, `investigation_mcp_off.bats`, `investigation_team_isolation.bats`, `investigation_team_opencode_schema.bats` | ~1,500 (`+1160 / -340`) |
| **8** | `helpers.go`, `runner.go`, `archive.go` | `helpers_test.go`, `runner_test.go`, `archive_test.go` | `agent.bats` (runner/helpers/archive/transcript/discard), `autopilot_questions.bats` | ~1,480 (`+1270 / -210`) |
| **9** | `team.go` | `team_test.go` | `team_close.bats`, `kick.bats`, `remote_control_kick_exit.bats`, `member_out.bats`, `pause_notify.bats`, `handover_failed.bats`, `remote_proxy.bats` | ~1,490 (`+910 / -580`) |
| **10** | `review.go` | `review_test.go` | `verify_loop.bats`, `auto_stage.bats`, `autopilot_review.bats`, `autopilot_merge.bats`, `review_pair_random.bats`, `review_resilience.bats` | ~1,490 (`+925 / -565`) |
| **11** | `investigation.go` (Part 1) | `investigation_test.go` | `rotation.bats`, `investigation_team_cli.bats`, `investigation_team_validation.bats`, `investigation_team_stdin.bats`, `investigation_single_investigator.bats`, `investigation_team_runtime.bats`, `investigation_team_admission.bats`, `investigation_team_publication_retry.bats`, `investigation_team_supervision.bats`, `investigation_team_regressions.bats`, `investigation_team_reboot_regression.bats` | ~1,485 (`+925 / -560`) |
| **12** | `investigation.go` (Part 2) | `investigation_pipeline_test.go` | `investigation_team_phase_selection.bats`, `investigation_team_phase_queue.bats`, `investigation_team_handoff.bats`, `investigation_single_pipeline.bats`, `investigation_independent_labels.bats`, `investigation_terminal_completion.bats`, `investigation_completed_legacy.bats`, `investigation_clarification.bats`, `investigation_pipeline_clarification.bats`, `investigation_prompt_clarification_resume.bats`, `investigation_independent_resume_prompt.bats`, `investigation_resume_independence.bats`, `investigation_team_advisory_resume.bats`, `investigation_team_resume_boundaries.bats`, `investigation_team_resume_recovery.bats`, `investigation_team_recovery_regressions.bats` | ~1,365 (`+870 / -495`) |
| **13** | `run.go` | `run_test.go` | `node_checkout.bats`, `resume_alloc.bats`, `resume_members.bats`, `resume_stash.bats`, `restart.bats` | ~1,495 (`+985 / -510`) |
| **14** | `node.go` | `node_test.go` | `lead.bats`, `lead_cli.bats`, `lead_cli_context.bats`, `web_jobs.bats`, `node_agents.bats`, `node_activity.bats`, `node_report_agent.bats`, `node_safety.bats`, `node_prelaunch_cancel.bats`, `node_cancel_exit.bats`, `node_recovery_fence.bats`, `node_review_boundaries.bats`, `node_large_assignment.bats`, `node_helper_hooks.bats`, `delegate_jobs.bats`, `remote_control_cancel_order.bats`, `remote_control_heartbeat_limits.bats`, `remote_control_review.bats`, `remote_control_validation.bats`, `remote_controls.bats`, `remote_control_web.bats`, `node_daemon.bats`, `node_two_homes.bats` | ~1,495 (`+1000 / -495`) |
| **15** | `node_sync.go`, `authenticated.go` | `node_sync_test.go`, `authenticated_test.go` | `node_sync.bats`, `node_sync_freshness.bats`, `node_sync_limits.bats`, `node_sync_review.bats`, `node_sync_state.bats`, `node_sync_tails.bats`, `node_sync_two_homes.bats`, `authenticated.bats` | ~1,485 (`+1005 / -480`) |
| **16** | `bin/agent` final trim, `install.sh`, `README.md` | `install_test.go` | `lead_install.bats`, `node_install.bats` + full 120-file Bats & Go verification | ~1,318 (`+166 / -1152`) |
