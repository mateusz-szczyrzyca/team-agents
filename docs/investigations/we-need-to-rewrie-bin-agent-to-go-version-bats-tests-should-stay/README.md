# Unified Plan: Rewriting `bin/agent` in Go While Preserving the Bats Baseline

This document merges and reconciles the three Phase 1 investigation reports ([`inv-kimi.md`](inv-kimi.md), [`inv-astra.md`](inv-astra.md), and [`inv-gemini-argon.md`](inv-gemini-argon.md)) into a single, step-by-step engineering plan for rewriting `bin/agent` from Bash into a multi-file Go executable under `agent/` while keeping the existing Bats test suite intact as the baseline verification oracle.

---

## 1. Executive Summary & Core Architecture Decisions

1. **Multi-File, Standard-Library-Only Go Module (`agent/`):**
   Build the replacement in `agent/` (`package main`, same Go version as `agentbus`, zero third-party dependencies per `AGENTS.md` Rule 8), split by domain responsibility (`agent/run.go`, `agent/team.go`, `agent/archive.go`, `agent/authenticated.go`, `agent/gh.go`, `agent/node.go`, etc.).
2. **No Runtime Bash Fallback or Production Test Hooks:**
   During incremental rounds, build the candidate binary at `build/agent-go` while keeping `bin/agent` and `tests/unit/**` byte-for-byte untouched. The Go binary must never fall back to executing `bin/agent` for unimplemented subcommands (which hides unported behavior), nor expose production `_test-*` / `_fn` flags solely to satisfy shell tests. Unimplemented commands in early rounds return a typed `OpError` cause.
3. **Three Verification Lanes (Preserving `AGENTS.md` Rule 1):**
   The baseline suite has **`110` `.bats` files (`12,372` lines) containing `462` `@test` cases**:
   - **`369` process-level cases (`66` files are pure CLI):** execute `agent` (or the `gh` symlink) as an external process and run unchanged against a compiled Go binary.
   - **`93` white-box cases across `44` files (`40` using `source` and `4` using `sed` function extraction):** source `bin/agent` into a Bash subshell, extract Bash functions with `sed`, or override internal shell functions/variables. An ELF binary cannot be sourced by Bash or parsed by `sed`.

   To satisfy both *"produce `bin/agent` as a Go program"* and *"Bats tests stay unmodified as baseline (`AGENTS.md` Rule 1)"*, every round uses three explicit verification lanes:
   - **Lane 1 — Unchanged Historical Bats Baseline:** runs all `462` original `@test` cases against the pinned Bash baseline (`bats tests/unit` during development; a disposable export of the pinned baseline commit after cutover).
   - **Lane 2 — Candidate Bats Contracts:** stages a disposable working-tree export with `build/agent-go` placed at that export's `bin/agent`, and runs the ledger-selected process-level Bats cases (`369` cases by Round 15) without modifying any file in `tests/unit/`.
   - **Lane 3 — Native Go Parity (`agent/*_test.go`):** ports every one of the `462` Bats cases (including all `93` `source`/`sed`-coupled cases and their fault injections) to 1-to-1 native Go unit and subprocess tests in `agent/*_test.go`, written RED → GREEN → REFACTOR (`docs/tdd.md`).

   *(Optional Post-Parity Conversion Gate:* If the human additionally wants the `93` white-box `.bats` cases in the live tree converted to process-level CLI or `go test` invocations after parity is proven in Round 15, that requires explicit human approval under `AGENTS.md` Rule 1 in a dedicated test-only round. The three-lane design requires zero edits to `tests/unit/**`.)*

---

## 2. Baseline Inventory & Pre-Existing Test Conflict on `main`

### 2.1 Frozen Baseline Metrics (`f6a602465a7d30e4b6a6737515e67e056f757a05`)

| Metric | Value |
|---|---|
| `bin/agent` size | `8,523` lines, `312`–`316` shell functions |
| `bin/agent` SHA-256 | `461b7f65ecacd9496302fd351f8acc9d0aafdc8fde06b6ff2312f570c8ca7413` |
| Bats unit suite (`tests/unit/*.bats`) | `110` files, `462` `@test` cases, `12,372` lines (plus `helpers.bash` and `remote_control_homes.bash`) |
| Process-level `@test` cases | `369` cases (`66` files use only process execution) |
| White-box `@test` cases (`source` / `sed`) | `93` cases across `44` files (`40` files use `source`, `4` files use `sed -n`) |
| Ordered test-content SHA-256 | `fca84f91a741e8ae428814aefab4824af711ae70b18101776996ad47aca5550e` |

### 2.2 Pre-Existing Baseline Conflict Requiring Human Resolution Before Round 0 Freeze

Running `make gate` on unmodified base commit `f6a602465a7d30e4b6a6737515e67e056f757a05` passes `check-generated`, `lint-shell`, `lint-go`, `lint-docs`, `unit-go`, and **`460` of `462`** Bats tests, but fails on **`2` Bats tests** due to a contradiction introduced when `213db63` / `f6a6024` added `tests/unit/investigation_mcp_off.bats`:

| Failing Test Case | Assertion in Older Test | Conflicting Assertion in `investigation_mcp_off.bats` (`f6a6024`) |
|---|---|---|
| `tests/unit/investigation_team_isolation.bats:91` (`codex invocation excludes agentbus args for investigation team task...`) | Requires `run_codex` for an independent investigator to omit `mcp_servers.agentbus.command` (`grep -q "mcp_servers\.agentbus\.command" "$STUB_CLI_LOG"` must exit non-zero). | `investigation_mcp_off.bats:8` requires `codex_mcp_args` (called by `run_codex`) to include `mcp_servers.agentbus.command=...` alongside `mcp_servers.agentbus.enabled=false`. |
| `tests/unit/investigation_team_opencode_schema.bats:45` (`opencode config for independent investigation emits strictly valid disabled mcp object...`) | Requires `opencode_config` for an independent investigator to emit `.mcp.agentbus | keys == ["enabled"] and .enabled == false` (no `type` or `command`). | `investigation_mcp_off.bats:22` requires `opencode_config` to emit `.mcp.agentbus.type == "local"` and `.mcp.agentbus.command | length >= 1` alongside `.mcp.agentbus.enabled == false`. |

Reproduce directly with:
```sh
bats tests/unit/investigation_mcp_off.bats tests/unit/investigation_team_isolation.bats tests/unit/investigation_team_opencode_schema.bats
```

**Action before implementation rounds:** Under `AGENTS.md` Rule 1, agents must not edit existing tests or add task-name hacks in `bin/agent`. Before starting Round 0, the human should confirm whether `investigation_team_isolation.bats:91` and `investigation_team_opencode_schema.bats:45` should be updated in a baseline-repair commit to match `investigation_mcp_off.bats`, then pin that green commit as the migration base.

---

## 3. Target Go Package Design (`agent/`)

### 3.1 Domain File Layout

Create `agent/go.mod` (`module agent`, `go 1.22`, standard library only). Organize `agent/` into focused domain files:

| File(s) in `agent/` | Responsibility & Key Symbols | `bin/agent` Lines |
|---|---|---:|
| `main.go`, `cli.go` | Thin entry point (`main` calls `run(ctx, args, env) int`), `filepath.Base(os.Args[0]) == "gh"` dispatch, exact CLI flag/positional parsing, exit-code mapping, `usage` | 8460–8523 |
| `errors.json`, `errors.go`, `errors_gen.go` | Per-operation error causes (`OpError[C]`, `ErrorClass`, `*StorageError`, `*ValueError`, `*ProcessError`) generated via `tools/errgen` (`make generate`); never edit `errors_gen.go` by hand | — |
| `types.go` | Typed enums (`TaskStatus`, `TaskKind`, `RunMode`, `AutoStage`, `PairStage`, `LimitKind`, `Provider`, `Tier`, `LogEvent`, `Timestamp`) with `Valid()`, `parseEnum`, and `UnmarshalJSON` | — |
| `logging.go` | Structured JSONL logging (`logInfo`, `logWarn`, `logError`, `warnSideEffect`, `cliWriter`) writing `{ts, level, component: "agent", event, ...}` to `logs/system.jsonl` and journal/runner logs | 202–285 |
| `config.go`, `config_shell.go` | `Env` dependency struct, `Config` defaults, agent/tier/provider lookup, and bounded `bash --noprofile --norc` configuration evaluator for `agents.conf` and `secrets.env` | 286–462 |
| `process.go`, `systemd.go`, `bus.go` | `Runner` interface (`exec.Cmd` with explicit argv slices, env allowlists, exit status preservation, process-group `killTree`), `systemd-run`/`systemctl --user` wrappers, and typed `agentbus` CLI client (`BusInvalidArg=2` .. `BusProcess=10`) | 436–453, 886–896 |
| `metadata.go`, `lock_linux.go` | `syscall.Flock` process locks (`meta.env.lock`, `task-ids.lock`, `dispatch.lock`, `wake.lock`, `helpers.lock`, `node-daemon.lock`), atomic sibling-temp-file writes, lossless `printf '%q'` `meta.env` codec preserving unknown fields, and `job_transitions.jsonl` emission | 463–565 |
| `gh.go` | Trusted `gh` filter (`argv[0] == "gh"`), `gh api` endpoint/method allowlist, GraphQL comment/review refusal, trusted author/fork filtering (`github_trusted`, `trusted_prs`), and `review_byline` injection | 9–200, 1189–1285 |
| `authenticated.go` | Provider and SSH/GitHub authentication probes (`authenticated`, `_authcheck`), `authClassify`, `apiErrorText`, `agentAuthTarget`, `opencodeModelFor` | 7958–8151 |
| `archive.go`, `tokens.go`, `transcript.go`, `status.go` | `archive` (`7z`/`7za`/`7zr` packing, verification before deletion, `archive.jsonl`), `discard`, `tokens`, `transcript`, `status`, `show`, `log`, `clean`, `agents`, `notes`, `helpers`, `bus`, `say`, `live`, `web` | 5816–6045, 6277–6339, 6411–6459, 8266–8505 |
| `questions.go`, `notify.go`, `prompt.go` | Question counting/validation, recommendations check, `**ANSWERED**` path unit watching, VS Code editor integration, remote question staging (`_remote_questions`, `_remote_questions_ready`), `notify`/`notice` (`notify-failures.jsonl`), `buildPrompt` (`{{PLACEHOLDERS}}`), `gitIdentity`, `commitTrailers`, `checkpointsLatest`, `roundTextStrip` | 566–981, 2494–2779, 3055–3154, 5687–5739, 6340–6410 |
| `limits.go`, `provider.go` | `limitKind`, `quotaResetSeconds`, `wakeLoopApplies`/`wakeLoopCheck`, `pauseTask`/`scheduleWake`, and per-provider CLI adapters (`runClaude`, `codexMCPArgs`, `runCodex`, `runGemini`, `opencodeConfig`, `runOpencode`, `isIndependentInvestigation`, `cliPrompt`) | 2273–2493, 2850–3054 |
| `runner.go` | Background task runner (`_runner`), `watchRun` hang watchdog, `activityProgress`, `verifyGaps`, `finalStatus`, missing-status question fallback, `freshSessionDue`, `longRunEnd`, `runnerExit` | 2780–2849, 4299–4889 |
| `run.go`, `resume.go` | Collision-safe ID allocation under `task-ids.lock` (`allocateRunIDs`), `createTask`, worktree setup/rollback, `prepareJobTask`, `resumeRun`, stash handling (`--intent-to-add`), `run`, `resume`, `stop`, `wake`, `kick` | 4890–5685, 5740–5815, 5915–5969, 6460–6515 |
| `rounds.go`, `autopilot.go` | `roundLimit`, `skipsInRepo`, `skipsAdded`, `prSkippedSection`, `roundCheck`, `cmdRoundSize`, `prFormatIssues`, `roundOpenPRs`, `roundPRSupersede`, `mergeBlockers`, `findingsCount`, `reviewVerdict`, `cmdPrecheck`, `cmdMerge`, `cmdFixpass`, `cmdAutopilot`, `startReviewPair` | 982–2099 |
| `team.go`, `helpers.go`, `investigate.go`, `supervise.go` | `teamAfterRun`, `finishTeam`, `memberOut`, `cmdWakeInternal`, `cmdTeam`, `teamClose`, `cmdRemoteMember`, worker helper queue (`dispatchHelpers`, `startHelper`, `helperFinish`), legacy investigation rotation (`pickInvestigator`, `dispatch`), and `_supervise` / `restartInterrupted` | 897–945, 2100–2271, 3155–3230, 4050–4298, 5970–6276, 8153–8265 |
| `investigation_team.go` | 5-phase investigation team pipeline (`_investigation-team`), fail-fast CLI/topic validation, cohort admission under `dispatch.lock`, `phase1.json` / `resume.json` manifests, publication retry (`git ls-remote`), uniform seeded merger/reviewer selection, `investigationHandoff`, and reboot/stop recovery | 3231–4049, 5480–5545 |
| `node.go`, `sync.go`, `remote_control.go` | `lead`, `jobs`, `nodes`, `node`, `ack`, `_agents-json`, `submitRemoteJob`, `nodePoll`, `nodeHeartbeatJSON`, `nodeTransition`, `nodeProcessPendingLaunches`, `nodeRecoverTasks`, `nodeSyncTasks` (tail clipping, byte caps, mtime freshness), and `nodeHandleReply` / `nodeHandleControls` | 6536–7957 |
| `testhelpers_test.go`, `*_test.go` | Shared Go test harness (`newTestEnv`, `failsWith`, `scenario`, `waitStatus`) and 1-to-1 unit/subprocess tests | — |

### 3.2 Dependency-Injected Application Seam (`Env`)

Avoid package-level mutable state, `os.Chdir`, or process-wide `os.Setenv` in logic. Pass an explicit `*Env` struct into every operation:

```go
// Env holds the resolved paths, configuration, clock, random source, and process runner for one invocation.
type Env struct {
	Home       string            // user home directory ($HOME)
	AgentsHome string            // root state directory ($AGENTS_HOME, default $HOME/agents)
	ConfPath   string            // path to agents.conf ($CONF)
	Secrets    string            // path to secrets.env ($SECRETS)
	RepoRoot   string            // git repository root when invoked inside a checkout
	SelfBin    string            // executable path used for self-invocation ($SELF)
	Argv0      string            // invocation name (os.Args[0], detects "gh" symlink)
	Environ    map[string]string // process environment snapshot
	Cfg        *Config           // decoded configuration and defaults
	Now        func() time.Time  // clock (time.Now in production, deterministic in tests)
	Rand       io.Reader         // entropy source (/dev/urandom in production, deterministic in tests)
	Exec       Runner            // subprocess execution seam
	HTTP       *http.Client      // HTTP client for lead/node and auth probes
	Stdout     io.Writer         // standard output stream
	Stderr     io.Writer         // standard error stream
	Stdin      io.Reader         // standard input stream
}
```

### 3.3 Thirteen Compatibility Contracts (`AGENTS.md` Rule 9)

1. **CLI & Internal Entry Points:** Preserve all public commands, aliases (`ls`, `help`, `-h`, `--help`), option/positional ordering, `--` handling, stdout vs stderr separation, exit codes, and all internal callback subcommands (`_runner`, `_wake`, `_supervise`, `_agents-json`, `_dispatch-helpers`, `_remote_member`, `_precheck`, `_merge`, `_fixpass`, `_answers_saved`, `_remote_questions`, `_remote_questions_ready`, `_authcheck`, `_investigation-team`, and `argv[0] == "gh"`).
2. **Configuration Evaluator (`agent/config_shell.go`):** `agents.conf` and `secrets.env` are executable Bash supporting associative arrays (`declare -A`), indexed arrays, `${VAR:-default}` expansions, and `read -ra` splitting (`AGENT_EXTRA_ARGS`). Do not write a partial regex parser that rejects valid Bash configs. Run a bounded `bash --noprofile --norc` helper with file paths passed as argv (never interpolated shell strings) that sources the configuration and emits an allowlisted schema as NUL-delimited records over a dedicated pipe, preserving unset vs empty semantics and never logging secrets.
3. **Metadata (`agent/metadata.go`):** `tasks/*/meta.env` uses Bash `printf '%s=%q\n'` quoting and is read by `bin/agent`, `agentlive`, and Bats tests. Implement a lossless Go reader/writer verified by differential fuzz/golden tests against Bash `printf %q` (covering empty values, spaces, quotes, backslashes, `$`, multiline strings, and UTF-8 under both default and `C.UTF-8` locales), preserving unknown keys. When `metaSet` changes `STATUS` on a task with `JOB != ""`, atomically append the transition record to `$AGENTS_HOME/bus/job_transitions.jsonl` under `meta.env.lock`.
4. **Process Locks (`agent/lock_linux.go`):** Use `syscall.Flock` on the exact lock files (`meta.env.lock`, `task-ids.lock`, `wake.lock`, `dispatch.lock`, `helpers.lock`, review pair locks, and `node-daemon.lock`), preserving shared/exclusive modes, timeouts (e.g. 60 s on `task-ids.lock`), non-blocking contention no-ops, and closing lock file descriptors (`3`, `7`–`10`) before spawning child processes.
5. **Atomic File Replacement:** Use unique sibling temporary files (e.g. `os.CreateTemp(dir, ".meta.env.tmp.*")`) + `chmod` + `os.Rename` so concurrent goroutines or processes never collide on PID-only temp names.
6. **Subprocess & Process-Group Management:** Build commands with `exec.Cmd` slice arguments (never shell concatenation), preserve stdin streaming for large task/JSON payloads, separate provider CLI exit codes from `tee` / `agentbus normalize` pipeline statuses, and terminate full descendant process groups (`killTree`) on watchdog timeout or `stop`.
7. **Single Finalization Owner:** Never call `os.Exit` inside runner or daemon logic before finalization completes. Guard `runnerExit` with run-generation and stop-request fences so stale callbacks cannot overwrite newer runs.
8. **Git & Worktree Isolation:** Preserve branch naming (`agents/<agent>/<timestamp>`), immutable `INVESTIGATION_BASE_SHA`, `task-ids.lock` reservation across checkout and team registration, dirty-worktree stashing (including `--intent-to-add` retry), and test isolation (`GIT_CONFIG_GLOBAL=/dev/null`, `commit.gpgsign=false`).
9. **Agentbus & Live Boundaries:** Keep `agentbus` and `agentlive` as separate modules invoked via their CLI/process interfaces; do not import their `package main` internals or mutate `bus/state.json` directly from `agent/`.
10. **Seeded Random Selection:** Match the exact lexicographical sorting and `AGENT_INVESTIGATION_SEED` / `INVESTIGATION_SEED` deterministic selection behavior tested by `investigation_team_phase_selection.bats` and `review_pair_random.bats`.
11. **Remote Node & Sync Wire Protocol:** Preserve assignment generations, prepare-before-start-ACK, cancel-before-launch fences, boot recovery, durable control ACKs, tail clipping, byte limits, sha256 hashes, and exact JSON field capitalization.
12. **Archives & Logs:** Keep `7z`/`7za`/`7zr` invocation and delete task directories/worktrees only after archive creation and verification succeed. Preserve all `LogEvent` names and JSONL schemas.
13. **Security & `gh` Filter:** Dispatch `argv[0] == "gh"` before normal config loading, fail closed on untrusted PRs/issues/forks and GraphQL comment/review queries, and never leak tokens in logs or error messages.

---

## 4. Verification Infrastructure & How to Translate Bats Tests to Go

### 4.1 The 462-Case Coverage Ledger

In Round 0, create a machine-readable coverage ledger (`docs/migrations/agent-go-ledger.tsv` or `.json`) with one row per `@test` across all `110` `.bats` files (`462` rows total) and a checker script/test (`make check-agent-parity`):

```text
ID | bats_file | test_name | mode (cli|source|sed) | go_test_name | round | status
```

### 4.2 Complete Mapping of All 110 `.bats` Files (`462` `@test` Cases) to `agent/*_test.go`

| Bats File (`tests/unit/`) | `@test` Cases | Mode (`cli` / `source` / `sed`) | Target Go File(s) | Target Go Test File | Round |
|---|---:|---|---|---|---:|
| `gate.bats` | 4 | `cli` | `tools/gate.sh`, `config.go` | `agent/config_test.go` | 1 |
| `lint_shell_memory.bats` | 1 | `cli` | `Makefile`, `config.go` | `agent/config_test.go` | 1 |
| `meta.bats` | 1 | `source` (`meta_set`) | `agent/metadata.go` | `agent/metadata_test.go` | 1 |
| `agent_gh.bats` | 7 | `cli` (`gh` symlink) | `agent/gh.go` | `agent/gh_test.go` | 2 |
| `github_trust.bats` | 4 | `cli` (`gh` symlink) | `agent/gh.go` | `agent/gh_test.go` | 2 |
| `review_byline.bats` | 3 | `source` (`review_byline`) + `cli` | `agent/gh.go` | `agent/gh_test.go` | 2 |
| `authenticated.bats` | 3 | `cli` | `agent/authenticated.go` | `agent/authenticated_test.go` | 2 |
| `tokens.bats` | 5 | `cli` | `agent/tokens.go` | `agent/tokens_test.go` | 3 |
| `web.bats` | 6 | `cli` | `agent/status.go` | `agent/status_test.go` | 3 |
| `web_jobs.bats` | 1 | `cli` | `agent/status.go` | `agent/status_test.go` | 3 |
| `identity.bats` | 3 | `cli` | `agent/prompt.go` | `agent/prompt_test.go` | 4 |
| `prompt_file.bats` | 2 | `cli` | `agent/prompt.go` | `agent/prompt_test.go` | 4 |
| `prompt_checkpoints.bats` | 1 | `source` (`checkpoints_latest`) | `agent/prompt.go` | `agent/prompt_test.go` | 4 |
| `round_only.bats` | 2 | `source` (`round_text_strip`) | `agent/prompt.go` | `agent/prompt_test.go` | 4 |
| `context_hygiene.bats` | 2 | `cli` | `agent/prompt.go` | `agent/prompt_test.go` | 4 |
| `autopilot_questions.bats` | 2 | `cli` | `agent/questions.go` | `agent/questions_test.go` | 4 |
| `remote_questions_atomic.bats` | 1 | `cli` | `agent/questions.go` | `agent/questions_test.go` | 4 |
| `remote_questions_preparation_retry.bats` | 3 | `cli` | `agent/questions.go` | `agent/questions_test.go` | 4 |
| `remote_answer_round.bats` | 1 | `cli` | `agent/questions.go` | `agent/questions_test.go` | 4 |
| `pause_notify.bats` | 3 | `cli` | `agent/notify.go` | `agent/notify_test.go` | 4 |
| `remote_control_notifications.bats` | 1 | `source` (`notify`, mocks `notify_send`) | `agent/notify.go` | `agent/notify_test.go` | 4 |
| `limits.bats` | 3 | `cli` | `agent/limits.go` | `agent/limits_test.go` | 5 |
| `network.bats` | 3 | `source` (`limit_kind`, `quota_reset_seconds`) | `agent/limits.go` | `agent/limits_test.go` | 5 |
| `auto_stage.bats` | 2 | `source` (`limit_kind`) | `agent/limits.go` | `agent/limits_test.go` | 5 |
| `run_notes.bats` | 3 | `source` (`limit_kind`, `quota_reset_seconds`, `node_job_result`) | `agent/limits.go` | `agent/limits_test.go` | 5 |
| `remote_fixes.bats` | 3 | `source` (`wake_loop_applies`) | `agent/limits.go` | `agent/limits_test.go` | 5 |
| `investigation_mcp_off.bats` | 2 | `source` (`codex_mcp_args`, `opencode_config`) | `agent/provider.go` | `agent/provider_test.go` | 5 |
| `investigation_team_isolation.bats` | 4 | `source` (`run_claude`, `run_codex`, `run_gemini`, `opencode_config`) | `agent/provider.go` | `agent/provider_test.go` | 5 |
| `investigation_team_opencode_schema.bats` | 1 | `source` (`opencode_config`) | `agent/provider.go` | `agent/provider_test.go` | 5 |
| `long_run_cut.bats` | 2 | `source` (`watch_run`) | `agent/runner.go` | `agent/runner_test.go` | 6 |
| `fresh_session.bats` | 3 | `source` (`fresh_session_due`, `long_run_end`) | `agent/runner.go` | `agent/runner_test.go` | 6 |
| `fresh_relative.bats` | 1 | `source` (`fresh_session_due`) | `agent/runner.go` | `agent/runner_test.go` | 6 |
| `verify_loop.bats` | 3 | `cli` | `agent/runner.go` | `agent/runner_test.go` | 6 |
| `resilience.bats` | 3 | `cli` | `agent/runner.go` | `agent/runner_test.go` | 6 |
| `agent.bats` | 30 | `source` (`quota_reset_seconds`, `usage`, mocks `bus`/`notify`) + `cli` | `agent/run.go`, `status.go`, `archive.go` | `agent/run_test.go`, `status_test.go`, `archive_test.go` | 3, 7 |
| `resume_alloc.bats` | 5 | `cli` | `agent/resume.go` | `agent/resume_test.go` | 7 |
| `resume_members.bats` | 2 | `cli` | `agent/resume.go` | `agent/resume_test.go` | 7 |
| `resume_stash.bats` | 1 | `cli` | `agent/resume.go` | `agent/resume_test.go` | 7 |
| `kick.bats` | 3 | `cli` | `agent/resume.go` | `agent/resume_test.go` | 7 |
| `wake_cmd.bats` | 4 | `source` (`node_handle_reply`) + `cli` | `agent/resume.go` | `agent/resume_test.go` | 7 |
| `rounds.bats` | 8 | `cli` | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `round_handover.bats` | 4 | `source` (`merge_blockers`, `round_pr_supersede`, `meta_set`) | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `round_reports.bats` | 1 | `cli` | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `one_pr_per_round.bats` | 3 | `source` (`round_open_prs`) | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `skips.bats` | 6 | `source` (`skips_in_repo`) + `cli` | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `github_format.bats` | 2 | `source` (`pr_format_issues`) | `agent/rounds.go` | `agent/rounds_test.go` | 8 |
| `findings.bats` | 1 | `source` (`findings_count`) | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `autopilot_merge.bats` | 5 | `source` (`merge_blockers`) + `cli` | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `autopilot_review.bats` | 6 | `source` (`review_verdict`, `round_test_changes`) + `cli` | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `review_pair_random.bats` | 4 | `source` (`start_review_pair`, mocks `free_investigator_list`) | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `review_resilience.bats` | 7 | `source` (`free_investigator_list`) + `cli` | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `handover_failed.bats` | 3 | `source` (`handover_failed`, `remote_member_task`, `remote_start_check`) | `agent/autopilot.go` | `agent/autopilot_test.go` | 8 |
| `coordination.bats` | 1 | `cli` | `agent/team.go` | `agent/team_test.go` | 9 |
| `team_close.bats` | 3 | `cli` | `agent/team.go` | `agent/team_test.go` | 9 |
| `member_out.bats` | 6 | `cli` | `agent/team.go` | `agent/team_test.go` | 9 |
| `rotation.bats` | 2 | `source` (`pick_investigator`, `remember_investigator`, `free_investigator`) | `agent/investigate.go` | `agent/investigate_test.go` | 9 |
| `restart.bats` | 4 | `cli` | `agent/supervise.go` | `agent/supervise_test.go` | 9 |
| `investigation_team_validation.bats` | 13 | `source` (`validate_investigation_team`, `validate_investigation_topic`) + `cli` | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_stdin.bats` | 2 | `source` (`validate_investigation_topic`) + `cli` | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_cli.bats` | 12 | `source` (`meta_set`, `task_field`) + `cli` | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_admission.bats` | 8 | `source` (`free_investigator_list`) + `cli` | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_phase_selection.bats` | 11 | `source` (`investigation_phase_choose`, `investigation_phase_pick`) | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_phase_queue.bats` | 4 | `source` (`investigation_pipeline_tick`, `investigation_team_tick`) | `agent/investigation_team.go` | `agent/investigation_team_test.go` | 10 |
| `investigation_team_handoff.bats` | 8 | `source` (`investigation_handoff`, `investigation_team_tick`) | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_publication_retry.bats` | 2 | `source` (`investigation_published`, `investigation_phase1_recover`) | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_advisory_resume.bats` | 4 | `source` (`investigation_resume_tick`, `investigation_team_tick`) + `cli` | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_resume_boundaries.bats` | 3 | `source` (`investigation_resume`, `investigation_resume_admit`) + `cli` | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_resume_recovery.bats` | 4 | `source` (`investigation_resume_tick`) + `cli` | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_runtime.bats` | 8 | `source` (`free_investigator`, `free_investigator_list`, `start_review_pair`) | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_supervision.bats` | 4 | `source` (`investigation_teams_tick`) + `cli` | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_reboot_regression.bats` | 2 | `source` (`restart_interrupted`) + `cli` | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_recovery_regressions.bats` | 8 | `source` (`free_investigator_list`, `investigation_team_tick`) | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `investigation_team_regressions.bats` | 2 | `source` (`investigation_team_tick`) + `cli` | `agent/investigation_team.go` | `agent/investigation_team_recovery_test.go` | 11 |
| `lead.bats` | 3 | `cli` | `agent/node.go` | `agent/node_test.go` | 12 |
| `lead_cli.bats` | 4 | `cli` | `agent/node.go` | `agent/node_test.go` | 12 |
| `lead_cli_context.bats` | 5 | `cli` | `agent/node.go` | `agent/node_test.go` | 12 |
| `lead_install.bats` | 2 | `cli` | `agent/node.go`, `install.sh` | `agent/node_test.go` | 12, 16 |
| `node_install.bats` | 4 | `cli` | `agent/node.go`, `install.sh` | `agent/node_test.go` | 12, 16 |
| `delegate_jobs.bats` | 7 | `cli` | `agent/node.go` | `agent/node_test.go` | 12 |
| `job_attribution.bats` | 2 | **`sed`** (`submit_remote_job`, `github_repo_url`) | `agent/node.go` | `agent/node_test.go` | 12 |
| `job_attribution_paths.bats` | 2 | **`sed`** (`github_repo_url`, `node_poll_remote_helpers`) | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_agents.bats` | 1 | **`sed`** (`node_heartbeat_json`) | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_activity.bats` | 3 | `source` (`node_heartbeat_json`, `node_job_result`) | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_report_agent.bats` | 1 | **`sed`** (`node_transition`) | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_safety.bats` | 4 | `source` (`node_lock`, `prepare_job_task`, `agent_running_task`) | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_large_assignment.bats` | 1 | `cli` | `agent/node.go` | `agent/node_test.go` | 12 |
| `node_checkout.bats` | 18 | `source` (`prepare_job_task`, `create_task`, `meta_set`, mocks `grep`) | `agent/node.go` | `agent/node_test.go` | 13 |
| `node_daemon.bats` | 15 | `source` (`prepare_job_task`, `node_jobs_load`) + `cli` | `agent/node.go` | `agent/node_daemon_test.go` | 13 |
| `node_cancel_exit.bats` | 4 | `source` (`node_handle_stop`, `runner_exit`, `prepare_job_task`) | `agent/node.go` | `agent/node_daemon_test.go` | 13 |
| `node_prelaunch_cancel.bats` | 2 | `source` (`node_process_pending_launches`, mocks `start_runner`) | `agent/node.go` | `agent/node_daemon_test.go` | 13 |
| `node_recovery_fence.bats` | 6 | `source` (`node_recover_tasks`, `node_process_pending_launches`, mocks `start_runner`) | `agent/node.go` | `agent/node_daemon_test.go` | 13 |
| `node_review_boundaries.bats` | 7 | `source` (`node_recover_tasks`, `node_scan_tasks`, `cmd_stop`) | `agent/node.go` | `agent/node_daemon_test.go` | 13 |
| `node_helper_hooks.bats` | 12 | `cli` | `agent/node.go` | `agent/node_daemon_test.go` | 13 |
| `node_sync.bats` | 6 | `cli` | `agent/sync.go` | `agent/sync_test.go` | 14 |
| `node_sync_freshness.bats` | 3 | `cli` | `agent/sync.go` | `agent/sync_test.go` | 14 |
| `node_sync_limits.bats` | 4 | `cli` | `agent/sync.go` | `agent/sync_test.go` | 14 |
| `node_sync_review.bats` | 8 | `cli` | `agent/sync.go` | `agent/sync_test.go` | 14 |
| `node_sync_state.bats` | 1 | `cli` | `agent/sync.go` | `agent/sync_test.go` | 14 |
| `node_sync_tails.bats` | 2 | `cli` | `agent/sync.go` | `agent/sync_test.go` | 14 |
| `node_sync_two_homes.bats` | 1 | `cli` | `agent/sync.go` | `agent/sync_test.go` | 14 |
| `node_two_homes.bats` | 11 | `cli` | `agent/sync.go`, `node.go` | `agent/sync_test.go` | 14 |
| `remote_controls.bats` | 3 | `cli` | `agent/remote_control.go` | `agent/remote_control_test.go` | 14 |
| `remote_control_cancel_order.bats` | 1 | `source` (`node_handle_reply`, mocks `node_handle_stop`, `node_apply_answer`) | `agent/remote_control.go` | `agent/remote_control_test.go` | 14 |
| `remote_control_heartbeat_limits.bats` | 3 | `source` (`node_flush_reports`, `node_clear_control_acks`) | `agent/remote_control.go` | `agent/remote_control_test.go` | 14 |
| `remote_control_kick_exit.bats` | 3 | `source` (`cmd_kick`, `runner_exit`) | `agent/remote_control.go` | `agent/remote_control_test.go` | 14 |
| `remote_control_proxy.bats` | 1 | `cli` | `agent/remote_control.go` | `agent/remote_control_test.go` | 14 |
| `remote_control_review.bats` | 7 | `source` (`node_apply_kick`, `cmd_answer`, `cmd_kick`, `runner_exit`) | `agent/remote_control.go` | `agent/remote_control_test.go` | 14 |
| `remote_control_validation.bats` | 7 | `source` (`node_handle_controls`, `node_apply_answer`, `node_apply_kick`) | `agent/remote_control.go` | `agent/remote_control_test.go` | 14 |
| `remote_control_web.bats` | 1 | `cli` | `agent/remote_control.go` | `agent/remote_control_test.go` | 14 |
| `remote_proxy.bats` | 2 | `cli` | `agent/remote_control.go` | `agent/remote_control_test.go` | 14 |

### 4.3 Shared Go Test Harness (`agent/testhelpers_test.go`) & Lane 2 Candidate Staging

1. **In `agent/testhelpers_test.go`:**
   - `newTestEnv(t *testing.T) *Env`: creates isolated `HOME`, `AGENTS_HOME`, `STUB_HOME`, and `XDG_RUNTIME_DIR` under `t.TempDir()`, copies `prompts/*.md`, writes default `agents.conf` and `0600` `secrets.env`, initializes a local git repository and bare `origin` remote (`GIT_CONFIG_GLOBAL=/dev/null`, `commit.gpgsign=false`), puts `tests/unit/stubs` and freshly built `agentbus` on the subprocess `PATH`, and registers `t.Cleanup` to terminate any child process groups created by the test.
   - `failsWith[C opCause](t *testing.T, want C)`: asserts exact typed `OpError` causes (`err.Cause == want`), never fragile message substrings.
   - `scenario(t, env, jsonSpec)` and `waitStatus` / `waitGrep` / `waitJQ`: drives `tests/unit/stubs/*` with the existing JSON scenario protocol and polls conditions with a deadline instead of bare `sleep`.
2. **Translating `source` and `sed` Tests to Go (`Lane 3`):**
   - Pure functions called via `source bin/agent` (e.g. `round_text_strip`, `quota_reset_seconds`, `limit_kind`, `findings_count`, `pr_format_issues`, `skips_in_repo`) become table-driven Go unit tests calling the internal Go functions directly.
   - Fault-injection tests that override shell functions or builtins (e.g. `node_checkout.bats` overriding `grep`/`printf` to simulate metadata read/write failures; `node_recovery_fence.bats` overriding `start_runner` and `unit_active`; `node_agents.bats` and `job_attribution.bats` extracting functions with `sed`) become Go unit tests that inject controlled fakes via `Env.Exec`, `Env.HTTP`, or storage hooks and assert on the exact persisted state, JSON payloads, and `OpError` causes.
   - Concurrency tests (`meta.bats` concurrent writers, `resume_alloc.bats` timestamp collisions) must run both `go test -race` and real multi-process subprocess tests (including cross-process `Bash` + `Go` lock/metadata interoperability).
3. **Running Unmodified Process-Level Bats Tests Against Go (`Lane 2`):**
   Because `tests/unit/helpers.bash` derives `REPO_ROOT` from the `.bats` file path and copies `$REPO_ROOT/bin/agent` into `$HOME/.local/bin/agent` (including secondary homes in `remote_control_homes.bash`), stage a disposable export under `build/go-bats`:
   ```sh
   rm -rf build/go-bats
   mkdir -p build/go-bats/bin build/go-bats/tests
   cp build/agent-go build/go-bats/bin/agent
   cp -a agentbus prompts tools install.sh Makefile build/go-bats/
   cp -a tests/unit build/go-bats/tests/unit
   bats build/go-bats/tests/unit/<process-level-file>.bats
   ```
   Automate this in a small helper (`make unit-bats-agent-go`) that verifies the copied binary's SHA-256 hash and runs all ledger-marked `cli` test cases without touching `tests/unit/`.

---

## 5. Step-by-Step Migration Rounds

Each round below must stay under the **3,000-line hard limit** (aiming for **≤ 2,000 lines**, checked with `agent round-size <task>`). Split any round that approaches 2,000 lines into lettered sub-rounds (`Round 12a`, `Round 12b`). Every round follows strict TDD (`docs/tdd.md`):
1. Add operation error causes to `agent/errors.json` and run `make generate`.
2. **RED commit:** add compiling zero-value Go signatures and the round's new `agent/*_test.go` tests; run `cd agent && go test ./...` to confirm they fail on the expected assertions; commit with `RED:` output in the commit message.
3. **GREEN commit:** implement the minimal Go code in `agent/*.go` to pass `cd agent && go test -race ./...`, the staged `Lane 2` Bats contracts, and `make gate`; commit with `GREEN:` output.
4. **REFACTOR commit:** clean up code with `git diff --stat -- '*_test.go'` empty and all checks green.

### Round 0 — Baseline Repair, Coverage Ledger, Go Skeleton, and Verification Plumbing
- **Scope:** After human resolution of the 2 conflicting baseline tests (`Section 2.2`), pin the green baseline commit. Add the 462-row coverage ledger, `agent/go.mod`, `agent/errors.json`, `agent/errors.go`, `agent/errors_gen.go`, `agent/main.go` skeleton, `agent/testhelpers_test.go`, and `Makefile` integration (`GO_MODULES`, `build-agent-go`, `unit-go`, `lint-docs`, `check-generated`, `unit-bats-agent-go`, `check-agent-parity`).
- **Verify:** `make generate`, `cd agent && go test -race ./...`, `make check-agent-parity`, `make gate`, and `git diff --exit-code <base> -- bin/agent tests/unit`.

### Round 1 — Configuration Evaluator, Logging, Locks, `meta.env` Codec, and `agentbus` Client
- **Files:** `agent/config.go`, `agent/config_shell.go`, `agent/types.go`, `agent/logging.go`, `agent/lock_linux.go`, `agent/metadata.go`, `agent/bus.go`, plus `*_test.go`.
- **Port & Test:** Bounded `bash --noprofile --norc` config loader (`agents.conf`, `secrets.env`), JSONL/journal logging (`system.jsonl`), `syscall.Flock` locks, atomic `metaSet` / `metaLoad` (`printf '%q'` round-trips, unknown fields, `C.UTF-8` locale, `job_transitions.jsonl`), and typed `agentbus` CLI wrapper.
- **Bats Parity (`Lane 2` & `Lane 3`):** `meta.bats`, `gate.bats`, `lint_shell_memory.bats`, plus cross-process Go/Bash `meta.env` concurrency tests.

### Round 2 — Security Boundary (`agent/gh.go`) and Provider Authentication (`agent/authenticated.go`)
- **Files:** `agent/gh.go`, `agent/authenticated.go`, `agent/gh_test.go`, `agent/authenticated_test.go`.
- **Port & Test:** Symlink `argv[0] == "gh"` dispatch before normal config loading, `gh api` allowlist, GraphQL comment/review refusal, trusted author/fork filtering (`github_trusted`, `trusted_prs`), `review_byline` injection, and `authenticated` / `_authcheck` probes using loopback HTTP/stubs (never real accounts).
- **Bats Parity (`Lane 2` & `Lane 3`):** `agent_gh.bats`, `github_trust.bats`, `review_byline.bats`, `authenticated.bats`.

### Round 3 — Archiving, Tokens, Transcripts, and Read-Only Inspection CLIs
- **Files:** `agent/archive.go`, `agent/tokens.go`, `agent/transcript.go`, `agent/status.go`, plus `*_test.go`.
- **Port & Test:** `archive` (`7z` group packing, verification before deletion, `archive.jsonl`), `discard`, `tokens`, `transcript`, `status` (`ls`), `show`, `log`, `clean`, `agents`, `notes`, `helpers`, `bus`, `say`, `live`, `web`, `help`.
- **Bats Parity (`Lane 2` & `Lane 3`):** `tokens.bats`, `web.bats`, `web_jobs.bats`, and the inspection/archive/discard cases in `agent.bats`.

### Round 4 — Prompts, Git Identity, Questions, Remote Question Staging, and Notifications
- **Files:** `agent/prompt.go`, `agent/questions.go`, `agent/notify.go`, plus `*_test.go`.
- **Port & Test:** `buildPrompt` (`{{PLACEHOLDERS}}` with zero unresolved tokens), `gitIdentity`, `commitTrailers`, `checkpointsLatest`, `roundTextStrip`, question validation/recommendations, `**ANSWERED**` path unit watching, VS Code opener, `_remote_questions` / `_remote_questions_ready`, and `notify` / `notice` (`notify-failures.jsonl`).
- **Bats Parity (`Lane 2` & `Lane 3`):** `identity.bats`, `prompt_file.bats`, `prompt_checkpoints.bats`, `round_only.bats`, `context_hygiene.bats`, `autopilot_questions.bats`, `remote_questions_atomic.bats`, `remote_questions_preparation_retry.bats`, `remote_answer_round.bats`, `pause_notify.bats`, `remote_control_notifications.bats`.

### Round 5 — Provider Limits, Quota Reset Parsing, Wake-Loop Guard, and CLI Adapters
- **Files:** `agent/limits.go`, `agent/provider.go`, `agent/limits_test.go`, `agent/provider_test.go`.
- **Port & Test:** `limitKind` (`quota`, `rate`, `network`, `auth`, `overloaded`), `quotaResetSeconds`, `wakeLoopApplies` / `wakeLoopCheck`, `pauseTask` / `scheduleWake`, and per-provider CLI launchers (`runClaude`, `codexMCPArgs`, `runCodex`, `runGemini`, `opencodeConfig`, `runOpencode`, `isIndependentInvestigation`, `cliPrompt`).
- **Bats Parity (`Lane 2` & `Lane 3`):** `limits.bats`, `network.bats`, `auto_stage.bats`, `run_notes.bats`, `remote_fixes.bats`, `investigation_mcp_off.bats`, `investigation_team_isolation.bats`, `investigation_team_opencode_schema.bats`.

### Round 6 — Background Task Runner (`_runner`), Watchdog, Verification Loop, and Fresh Sessions
- **Files:** `agent/runner.go`, `agent/runner_test.go`.
- **Port & Test:** `cmdRunner`, `startRunner`, `watchRun` process-group watchdog (`killTree`), `activityProgress`, `verifyGaps`, `finalStatus`, missing-status question fallback, `freshSessionDue`, `longRunEnd`, and `runnerExit` generation fencing.
- **Bats Parity (`Lane 2` & `Lane 3`):** `long_run_cut.bats`, `fresh_session.bats`, `fresh_relative.bats`, `verify_loop.bats`, `resilience.bats`.

### Round 7 — Task Allocation, Worktrees, `run`, `resume`, `stop`, `wake`, and `kick`
- **Files:** `agent/run.go`, `agent/resume.go`, `agent/run_test.go`, `agent/resume_test.go`.
- **Port & Test:** `allocateRunIDs` under `task-ids.lock`, `createTask`, worktree rollback on failure, `prepareJobTask`, `resumeRun`, dirty-worktree stash handling (`--intent-to-add`), `cmdRun`, `cmdResume`, `cmdStop` / `stopAll`, `cmdWake`, `cmdKick`.
- **Bats Parity (`Lane 2` & `Lane 3`):** `resume_alloc.bats`, `resume_members.bats`, `resume_stash.bats`, `kick.bats`, `wake_cmd.bats`, and the remaining solo run/resume/stop tests in `agent.bats`.

### Round 8 — Rounds, Skipped Tests, PR Format Checks, and Autopilot Review/Merge
- **Files:** `agent/rounds.go`, `agent/autopilot.go`, `agent/rounds_test.go`, `agent/autopilot_test.go`.
- **Port & Test:** `roundLimit`, `skipsInRepo`, `skipsAdded`, `prSkippedSection`, `roundCheck`, `cmdRoundSize`, `prFormatIssues`, `roundOpenPRs`, `roundPRSupersede`, `mergeBlockers`, `findingsCount`, `reviewVerdict`, `cmdPrecheck`, `cmdMerge`, `cmdFixpass`, `cmdAutopilot`, `startReviewPair`, `handoverFailed`.
- **Bats Parity (`Lane 2` & `Lane 3`):** `rounds.bats`, `round_handover.bats`, `round_reports.bats`, `one_pr_per_round.bats`, `skips.bats`, `github_format.bats`, `findings.bats`, `autopilot_merge.bats`, `autopilot_review.bats`, `review_pair_random.bats`, `review_resilience.bats`, `handover_failed.bats`.

### Round 9 — Teams, Workers/Helpers, Legacy Investigations, and Supervisor (`_supervise`)
- **Files:** `agent/team.go`, `agent/helpers.go`, `agent/investigate.go`, `agent/supervise.go`, plus `*_test.go`.
- **Port & Test:** `teamAfterRun`, `finishTeam`, `memberOut`, `cmdWakeInternal`, `cmdTeam`, `teamClose`, `cmdRemoteMember`, worker delegation (`dispatchHelpers`, `startHelper`, `helperFinish`), investigator rotation (`pickInvestigator`, `rememberInvestigator`, `dispatch`), and `_supervise` / `restartInterrupted`.
- **Bats Parity (`Lane 2` & `Lane 3`):** `coordination.bats`, `team_close.bats`, `member_out.bats`, `rotation.bats`, `restart.bats`.

### Round 10 — Investigation Teams Part 1: Validation, CLI, Admission, Phase Selection & Queue
- **Files:** `agent/investigation_team.go`, `agent/investigation_team_test.go`.
- **Port & Test:** `validateInvestigationTeam`, `validateInvestigationTopic`, `investigationTeamAdmit` under `dispatch.lock`, `phase1.json` skeleton before `INVESTIGATION_LEASE=1`, `investigationPhaseChoose` / `investigationPhasePick` (uniform seeded selection), `investigationPhaseStart`, `investigationPhaseFailed`.
- **Bats Parity (`Lane 2` & `Lane 3`):** `investigation_team_validation.bats`, `investigation_team_stdin.bats`, `investigation_team_cli.bats`, `investigation_team_admission.bats`, `investigation_team_phase_selection.bats`, `investigation_team_phase_queue.bats`.

### Round 11 — Investigation Teams Part 2: Publication Retry, Handoff, Advisory Resume & Recovery
- **Files:** `agent/investigation_team.go`, `agent/investigation_team_recovery_test.go`.
- **Port & Test:** `investigationPublished` (`git ls-remote` transient retry vs invalid commit), `investigationPhase1Recover` (`investigation_manifest_recovered`), `investigationHandoff`, `investigationResume` / `investigationResumeAdmit` / `investigationResumeTick` (`resume.json` v1), and stop/reboot cleanup.
- **Bats Parity (`Lane 2` & `Lane 3`):** `investigation_team_handoff.bats`, `investigation_team_publication_retry.bats`, `investigation_team_advisory_resume.bats`, `investigation_team_resume_boundaries.bats`, `investigation_team_resume_recovery.bats`, `investigation_team_runtime.bats`, `investigation_team_supervision.bats`, `investigation_team_reboot_regression.bats`, `investigation_team_recovery_regressions.bats`, `investigation_team_regressions.bats`.

### Round 12 — Multi-Instance Lead CLI, Job Submission, Node Inventory & Checkout
- **Files:** `agent/node.go`, `agent/node_test.go`.
- **Port & Test:** `cmdLead`, `cmdJobs`, `cmdNodes`, `cmdAck`, `cmdAgentsJSON`, `submitRemoteJob`, `githubRepoURL`, `nodePollRemoteHelpers`, `nodeHeartbeatJSON`, `nodeTransition`, `nodeLock`, and `prepareJobTask` checkout/rollback.
- **Bats Parity (`Lane 2` & `Lane 3`):** `lead.bats`, `lead_cli.bats`, `lead_cli_context.bats`, `delegate_jobs.bats`, `job_attribution.bats`, `job_attribution_paths.bats`, `node_agents.bats`, `node_activity.bats`, `node_report_agent.bats`, `node_safety.bats`, `node_large_assignment.bats`, `node_checkout.bats`.

### Round 13 — Node Daemon Loop, Pending-Launch & Recovery Fences, and Helper Hooks
- **Files:** `agent/node.go`, `agent/node_daemon_test.go`.
- **Port & Test:** `cmdNode`, `nodePoll`, `nodeStartAck`, `nodeProcessPendingLaunches`, `nodeRecoverTasks`, `nodeHandleStop`, `nodeJobResult`, and helper hook lifecycle.
- **Bats Parity (`Lane 2` & `Lane 3`):** `node_daemon.bats`, `node_cancel_exit.bats`, `node_prelaunch_cancel.bats`, `node_recovery_fence.bats`, `node_review_boundaries.bats`, `node_helper_hooks.bats`.

### Round 14 — Node Synchronization, Remote Controls, and Multi-Home End-to-End Parity
- **Files:** `agent/sync.go`, `agent/remote_control.go`, `agent/sync_test.go`, `agent/remote_control_test.go`.
- **Port & Test:** `nodeScanTasks`, `nodeSyncTasks`, `nodeSyncTaskPayload`, `nodeFlushReports`, `nodeHandleReply`, `nodeHandleControls`, `nodeApplyAnswer`, `nodeApplyKick`, `nodeClearControlAcks`.
- **Bats Parity (`Lane 2` & `Lane 3`):** `node_sync*.bats`, `node_two_homes.bats`, `remote_controls.bats`, `remote_control_*.bats`, `remote_proxy.bats`.

### Round 15 — Full 462-Case Parity Audit & Mixed-Version Recovery Rehearsal
- **Scope:** Verify that all `462` ledger rows have passing native Go tests in `agent/*_test.go` (`Lane 3`), all `369` process-level Bats tests pass unchanged against `build/agent-go` (`Lane 2`), and all `462` original Bats tests pass in `Lane 1`. Test mixed-version recovery (tasks created by Bash `bin/agent` resumed/supervised by `build/agent-go` and vice versa).

### Round 16 — Final Cutover, `install.sh` & `Makefile` Switch, and Rollback Verification
- **Scope:**
  1. Preserve the legacy Bash script for `Lane 1` via a pure Git rename (`git mv bin/agent tools/legacy/agent.bash` — verify with `git diff -M --numstat` so `round-size` recognizes the rename rather than counting an 8,523-line deletion).
  2. Ignore the compiled `/bin/agent` binary in `.gitignore` and update `Makefile` (`build` compiles `cd agent && go build -o ../bin/agent .`; `SHELL_SCRIPTS` replaces `bin/agent` with `tools/legacy/agent.bash`; `unit-bats` runs both the historical `Lane 1` export against `tools/legacy/agent.bash` and `Lane 2` process contracts against the compiled `bin/agent`).
  3. Update `install.sh` to build and atomically install the Go `agent` binary into `~/.local/bin/agent` alongside `agentbus` and `agentlive`, and add native Go installer tests (`lead_install.bats`, `node_install.bats` equivalents testing real binary output and build-failure rollback in an isolated temporary `HOME` — **never** run `install.sh` against the live installation per `AGENTS.md` Rule 3).
  4. Update `README.md` to document `agent/` (Go, standard library only) and the rollback procedure (reinstalling `tools/legacy/agent.bash` with full on-disk compatibility).

---

## 6. Per-Round Verification Checklist

Before committing and pushing any round:

```sh
# 1. Regenerate and verify error types
make generate
git diff --exit-code agent/errors_gen.go

# 2. Run Go vet, unit tests, and race detector
cd agent && go vet ./... && go test -race -count=1 ./...

# 3. Run linters and doccheck (every constant, variable, type, and struct field commented)
make lint-go lint-docs

# 4. Run staged Candidate Bats Contracts (Lane 2) and parity ledger check
make unit-bats-agent-go
make check-agent-parity

# 5. Verify untouched Bats suite and bin/agent (Rounds 0–15) and run the full gate
git diff --exit-code <pinned-base> -- bin/agent tests/unit
make gate

# 6. Check PR diff size against the 3,000-line hard limit (aim for <= 2,000 lines)
agent round-size <task-id>
```
