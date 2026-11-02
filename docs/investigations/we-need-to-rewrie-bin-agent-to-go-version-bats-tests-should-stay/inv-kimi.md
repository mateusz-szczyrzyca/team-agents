# Investigation: rewrite `bin/agent` in Go without changing the Bats tests

## Recommendation

Replace the Bash orchestrator with a standard-library-only Go executable, developed in a new `agent/` Go module and
split by responsibility. Do **not** translate the 8,583-line file top to bottom in one round, do not put a shell
wrapper around a Go program, and do not edit the existing Bats tests to make the rewrite pass.

Use a strangler migration with three independent verification tracks:

1. **Legacy baseline:** all 480 existing Bats cases, byte-for-byte unchanged, against a preserved copy of the current
   Bash implementation (the oracle).
2. **Go black-box compatibility:** every existing Bats case that only executes `agent`, also byte-for-byte unchanged,
   against the compiled Go executable in an isolated staged repository root.
3. **Go unit compatibility:** an equivalent Go test for every Bats case that sources `bin/agent` or extracts Bash
   functions. A compiled executable cannot satisfy those cases: `source` requires Bash source text, and `sed`
   extraction requires the old function bodies.

A helper-aware audit at base `940f6fdc067069f92a8023c84ed1b5eb011b95ae` found **121 source-coupled cases in 49
files** and **359 black-box cases**. The earlier count of 86 considered only direct references in `@test` bodies; it
missed helpers such as `setup_isolation`, `control_fixture`, `pair_of`, and the attribution extraction helpers.
Round 0 must turn the helper-aware inventory into a checked, per-test manifest. The counts above describe the current
base; the manifest, not a hard-coded total in a test, is the source of truth as tests are added in the future.

The final `make test` should run all three tracks. The legacy track remains a regression oracle; it is not evidence
that the Go binary works. The Go black-box track and the mapped Go tests are the Go compatibility evidence.

## Current shape and constraints

- `bin/agent` is 8,583 lines, 434,335 bytes, and 317 Bash functions (including the two subshell-bodied functions).
  Its SHA-256 at the base is `52fefa2be198cca4a57512a9236c2dbcd159a6a42a1e65dcb7d8b9b69df8073b`.
- There are 120 `tests/unit/*.bats` files, 480 cases, and 12,953 lines in those files (13,070 including
  `helpers.bash`). They use real temporary Git repositories, stub CLIs, stub systemd, and a temporary `HOME`.
- `tests/unit/helpers.bash` installs `$REPO_ROOT/bin/agent` into each temporary home. Many tests also install it into
  a second node home or invoke it through a `gh` symlink.
- Forty-nine files contain source/extract coupling. The 121 affected cases break down as follows; this is why simply
  replacing `bin/agent` with an ELF binary and running `make test` is not a viable migration strategy.

| Area | Source-coupled cases | Bats files |
|---|---:|---|
| Core, prompts, limits, metadata | 13 | `agent`, `auto_stage`, `fresh_relative`, `fresh_session`, `long_run_cut`, `meta`, `network`, `prompt_checkpoints`, `round_only`, `run_notes`, `skips` |
| Investigations and investigator selection | 17 | `investigation_mcp_off`, `investigation_prompt_clarification_resume`, `investigation_team_admission`, `investigation_team_isolation`, `investigation_team_opencode_schema`, `investigation_team_recovery_regressions`, `investigation_team_runtime`, `rotation` |
| Reviews, rounds, autopilot | 14 | `autopilot_merge`, `autopilot_review`, `findings`, `github_format`, `one_pr_per_round`, `review_byline`, `review_pair_random`, `review_resilience`, `round_handover` |
| Node state, admission, recovery | 47 | `node_activity`, `node_agents`, `node_cancel_exit`, `node_checkout`, `node_daemon`, `node_prelaunch_cancel`, `node_recovery_fence`, `node_report_agent`, `node_review_boundaries`, `node_safety` |
| Remote jobs and controls | 30 | `handover_failed`, `job_attribution`, `job_attribution_paths`, `remote_control_cancel_order`, `remote_control_heartbeat_limits`, `remote_control_kick_exit`, `remote_control_notifications`, `remote_control_review`, `remote_control_validation`, `remote_fixes`, `wake_cmd` |

Other important constraints:

- `agents.conf` is explicitly a Bash file. It uses scalar assignments, indexed and associative arrays, expansions,
  and legacy single-investigator variables. `secrets.env` is sourced with `set -a`. A Go parser for a small-looking
  subset would silently change supported configuration behavior.
- `lead_install.bats` and `node_install.bats` put a no-op `go` stub first on `PATH`. An installer that unconditionally
  runs `go build` for `agent` and then expects an output binary will fail those unchanged tests. The test harness must
  supply a prebuilt candidate explicitly; a real installation must still build and propagate compiler failures.
- `meta.env` uses `KEY=<bash printf %q value>`, is also read by `agentlive`, and is updated by concurrent processes
  under `.lock` files. `live/data.go` already has a compatible decoder to use as a reference, not as an import from a
  `package main` module.
- The public contract includes CLI output and exit statuses, systemd unit names and argv, prompt files, logs,
  `activity.jsonl`, notifications, worktrees/branches, archives, bus command interactions, remote JSON, and recovery
  after partial writes. Compatibility is therefore broader than successful Bats assertions alone.
- The new module should use Go 1.22, matching `agentbus`, and no third-party dependencies. `agentbus` remains a
  separate process and the owner of bus state and MCP behavior; importing or duplicating its state machinery would
  create a second writer implementation.
- Real agent CLIs and the running installation are not migration test targets. All tests continue to use temporary
  homes and stubs. `make integration` remains opt-in and is not required for these rounds.

## Target architecture

Use one `package main` module, with small files by responsibility rather than one large file or a deep package tree.
The initial layout should evolve toward:

```text
agent/
  go.mod                 # module agent; go 1.22; standard library only
  main.go                # os.Args[0] gh dispatch, CLI dispatch, exit status
  app.go                 # paths and narrow process/clock dependencies
  cli.go                 # command table and shared argument handling
  config.go              # Bash-compatible agents.conf loading and defaults
  envfile.go             # secrets.env environment loading without logging secrets
  shellquote.go          # Bash %q-compatible meta.env codec and golden tests
  meta.go                # locked, atomic metadata reads/updates and task lookup
  logging.go             # existing JSON schema, journal and runner.log sinks
  process.go             # argv/env/cwd/process-group boundary; no shell interpolation
  task.go                # IDs, allocation, worktrees, cleanup, resume chains
  run.go                 # run/resume command and runner launch
  prompt.go              # template rendering, checkpoints, repository instructions
  questions.go           # question/answer files, validation, editor/watchers
  runner.go              # runner state machine and exit finalization
  claude.go              # Claude CLI adapter
  codex.go               # Codex CLI adapter
  gemini.go              # Gemini/Antigravity CLI adapter
  opencode.go            # OpenCode config and CLI adapter
  limits.go              # quota/network classification and pause/wake decisions
  team.go                # team lifecycle, member transitions, board coordination
  helper.go              # worker admission and completion
  investigation.go       # single-investigator queue and lifecycle
  investigation_team.go  # independent investigation phases and recovery
  review.go              # review parsing, pairs, prechecks, fix passes
  autopilot.go           # merge and next-round orchestration
  remote.go              # lead listener, proxies, remote jobs and questions
  node.go                # node state, admission, heartbeat, recovery, polling
  node_sync.go           # bounded remote control and state/log synchronization
  authenticated.go       # provider/model checks and error classification
  transcript.go          # chronological task/team output and token accounting
  archive.go             # archive grouping, 7z creation, atomic removal
  errors.go              # shared concrete storage/value/process errors
  errors.json            # per-operation causes, consumed by existing errgen
  errors_gen.go          # generated; never edited by hand
```

This is a responsibility map, not a requirement to create empty files in Round 1. Add a file with the first behavior
that belongs in it. If a file grows too large, split along the existing state-machine boundary (for example,
`node_admission.go` and `node_recovery.go`) rather than introducing shared global state.

### Design rules

1. **Pure core, thin process boundary.** Parse CLI JSON, classify statuses, select investigators, compute prompt
   inputs, validate controls, and plan recovery as functions of values and byte streams. Isolate `os/exec`, systemd,
   Git, HTTP, clocks, and process groups behind small dependencies. Use real temporary files for filesystem tests and
   `t.TempDir()` everywhere; do not build a large mock filesystem.
2. **Typed errors and stable exits.** Follow the repository's per-operation error model. A local `agent/errors.json`
   can use the existing `tools/errgen` output conventions with local `OpError` support; `make generate` and
   `check-generated` must cover it. Map errors to the current `agent` exit behavior rather than accidentally adopting
   `agentbus`'s different CLI exit codes. Check causes in tests, never message text except where the text is the CLI
   contract.
3. **No mutable global command state.** An `App` value owns configuration, paths, stdout/stderr, process runner,
   clock, and runner log sink. Pass explicit state between runner phases; do not reproduce Bash globals such as `RC`,
   `FINAL`, `NEW_ID`, and `FRESH_WHY` as package globals. Use small result structs.
4. **Preserve configuration through a narrow Bash loader.** On startup, use a fixed, non-interactive Bash invocation
   to source `agents.conf` and serialize only the declared scalar/indexed/associative settings over a NUL-delimited
   protocol. Apply defaults in Go, preserving the distinction between unset and empty where Bash uses `${VAR-…}`
   rather than `${VAR:-…}`. Use the same approach to source `secrets.env` with `set -a` and return the exported
   environment via `env -0`, but only at the current runner/authentication boundaries—not on every CLI invocation.
   This retains the documented Bash configuration contract without using a shell for normal command execution. Never
   log the serialized secrets. Characterize the current `AGENT_EXTRA_ARGS` splitting before changing it. If a
   no-Bash-at-runtime requirement is added later, that is a separate configuration migration, not part of this
   compatibility rewrite.
5. **Keep disk formats compatible.** Implement and golden-test `%q` encoding/decoding, including empty values,
   whitespace, quotes, backslashes, dollar signs, Unicode, tabs, and newlines. Read existing unquoted/simple values
   too. Update `meta.env` under the same lock path with a 30-second timeout, stale-`RUN` no-op behavior, unknown-line
   preservation, same-file atomic rename, and cleanup on every failure. Use Linux `flock` through the standard
   library so old and new processes can overlap safely during migration. Add mixed Bash/Go writer tests.
6. **Preserve process semantics.** Build argv arrays directly; never concatenate shell commands. Retain environment
   allowlists/overrides, working directories, systemd unit names, process-group termination, watchdog polling,
   `agent` invoked as `gh`, and best-effort cleanup/logging behavior. Dispatch the `gh` fast path with
   `filepath.Base(os.Args[0]) == "gh"` before configuration loading, because tests and runners invoke an absolute
   symlink path. Replace Bash's `readlink -f` `SELF` behavior with `os.Executable` plus `filepath.EvalSymlinks` as
   needed. Retain narrow, documented shell boundaries for user-configured command strings such as
   `AGENT_REVIEW_GATE` (currently `bash -c`) and characterize their quoting; the no-shell rule applies to internally
   constructed argv, not to a configuration contract that explicitly is shell code.
7. **No production cutover until feature complete.** Early rounds build and test `build/agent` but `install.sh`
   continues installing the oracle. Unported commands return a typed, unreachable-development `not implemented`
   error in the development binary; no skipped tests or partial production dispatch are added. The final round moves
   the unchanged oracle out of `bin/agent` with a 100% rename and installs the Go binary. This avoids a large deletion
   in the round-size budget and avoids running a half-ported installation.

## Test architecture in detail

### A. Byte-identical legacy Bats baseline

Add a staging runner, for example `tools/agent-bats.sh`, which copies the current worktree needed by the tests into a
temporary root and places the Bash oracle at `STAGED_ROOT/bin/agent`. It then invokes the existing
`tools/unit-bats.sh` on the staged test paths. Copy the working tree rather than using `git archive`, so RED/GREEN
changes are testable before commit. Do not symlink the staged `bin/agent` to the Go executable.

This staging is necessary because `helpers.bash` derives `REPO_ROOT` from the Bats file location. Running staged,
unchanged test files makes all existing relative paths, stubs, prompts, and module builds point at the intended
implementation. The canonical `tests/unit` directory is never rewritten or filtered.

### B. Unchanged black-box Bats against Go

Build `build/agent` and materialize it as `STAGED_ROOT/bin/agent` in a second temporary root. Run only manifest rows
marked `blackbox`. For files mixing black-box and source-coupled cases, invoke Bats with `--filter` and an anchored,
regex-escaped allowlist of the exact black-box test names. Do not create pruned copies of Bats files and do not add
`skip` statements. Preserve the existing serial-file rules and JUnit reports.

At final cutover, the selection check must prove 359 current black-box cases were selected and executed, with zero
new skips. During intermediate rounds it executes `active` and `done` rows; future rows remain `planned`, not
skipped. Activation is per test case, never per file: a file name or wildcard in `BATS_FILES` does not override that
selection. A failure caused by a missing source-coupled classification means the manifest/runner is wrong: fix the
manifest after auditing it, never weaken the test.

### C. Equivalent Go tests for source-coupled cases

Create `tests/agent-go-map.tsv` outside `tests/unit`, with one row for every current Bats case:

```text
bats_file<TAB>test_name<TAB>blackbox|source<TAB>implementation_round<TAB>go_test_or_dash<TAB>planned|active|done
```

Every `source` row must name a real Go test before it can become `done`. Every Bats file/test name must occur exactly
once, and every source row must have a nonempty Go mapping. `planned` rows are future work; `active` rows are the
current RED/GREEN work and run together with all previously `done` rows; `done` rows have passed their GREEN check.
`BATS_FILES` may narrow files but must never pull a `planned` row into a run. Add a small coverage-check test/tool
that derives names from the Bats files dynamically rather than asserting 480 forever. Include helpers in the audit: a
source call inside a helper makes each calling test source-coupled. Review all indirect calls when a new Bats case is
added. Harness tests must prove the lifecycle `planned` (not run) → `active` (RED fails, then GREEN runs) → `done`
(passes on later rounds), including a previously failed row remaining active rather than silently becoming planned.

Equivalent Go tests must assert the behavior, not call the old Bash function and declare success. Reuse fixtures and
expected values where useful, but write Go tests at the new function/process boundary. For example:

- `meta.bats` becomes concurrent Go metadata updates plus a mixed Bash/Go lock interoperability test.
- `node_checkout.bats` becomes table-driven task-admission tests with fake process results and injected read, write,
  and rename failures, plus real temporary Git worktree tests.
- `review_pair_random.bats` becomes deterministic seeded pair-selection tests checking distribution, distinctness,
  order, and reuse of an existing pair.
- `investigation_team_isolation.bats` becomes table-driven adapter configuration tests for all four CLIs, both with
  and without an investigation-team marker.
- `job_attribution*.bats` becomes pure origin-URL canonicalization tests and a fake-bus submission test.

### D. Mandatory test-first protocol for every implementation round

For each behavior in the round:

1. **RED:** add only that round's Go tests and any zero-value signatures/error causes needed to compile. Run the
   focused command, confirm failures are behavioral and only in the new tests, and commit the tests with the failing
   output as required by `docs/tdd.md`. Mark the corresponding unchanged black-box Bats rows `active` only where the
   Go command boundary and every dependency of that exact case are ready; the RED run must include and fail them.
2. **GREEN:** implement the smallest compatible behavior. Run focused Go tests and the round's staged black-box Bats
   cases. Commit with the passing output.
3. **REFACTOR:** improve structure without changing tests. Check that the refactor diff contains no `*_test.go`
   changes and rerun the same checks.
4. Change manifest rows from `active` to `done` only after their Go test and, for black-box rows, the staged Bats case
   pass. A failed row stays `active`. Do not write tests for later rounds and do not introduce skips. Any bug
   discovered during a later round gets a new failing Go regression test before its fix.

### Common verification and migration commands

Define `BASE=940f6fdc067069f92a8023c84ed1b5eb011b95ae` in each round's shell. Every round runs the relevant focused
commands plus:

```bash
make gate                                      # lint, all ordinary Go tests, full legacy Bats baseline
make unit-agent-go                             # completed black-box rows against build/agent
cd agent && go test -race ./...                # required after concurrency changes and at final cutover
git diff --exit-code "$BASE" -- tests/unit     # no existing Bats/helper/stub change
git status --short -- tests/unit               # also catches untracked changes there
```

Until the final move, verify the oracle with:

```bash
cmp <(git show "$BASE":bin/agent) bin/agent
```

After the final move, use `cmp <(git show "$BASE":bin/agent) tests/oracle/agent`. Check each implementation round
with `agent round-size <round-task>`: target roughly 1,500 added lines including tests, stay below the repository's
3,000-line hard limit, and split a round before implementation if its estimate exceeds that. Generated files and the
100% oracle rename are handled by the normal round-size rules; do not hide hand-written code in generated files.

## Round-by-round implementation plan

The line estimates are planning budgets, not permission to omit tests. A round that reaches its budget early stops at
a coherent boundary; a round that needs substantially more is split before RED tests are written.

### Round 0 — Freeze the oracle contract and build the staging harness (~900–1,200 added lines)

**Implementation steps**

1. Record the base SHA, oracle hash, command/function inventory, Bats names, and helper-aware 121/359 classification
   in `tests/agent-go-map.tsv`; assign each row to Round 1–15.
2. Add the staged-root runner and manifest validation. Support legacy and Go modes, exact per-test filters, existing
   `BATS_FILES`/`BATS_SERIAL` behavior, and JUnit case counting.
3. Add harness self-tests for mixed files, regex characters in test names, indirect source helpers, staged second
   homes, and the `gh` symlink. Update `Makefile` and README development commands without changing anything under
   `tests/unit`.

**Tests and verification**

```bash
make unit-bats                                 # all 480 against the staged Bash oracle
tools/check-agent-test-map.sh                  # Bash/awk only: no new test dependency
bats tests/agent_go_harness.bats               # new harness tests, not edits to existing Bats
make gate
```

The new checker must report 480 unique current rows, 121 source rows with planned Go mappings, and 359 black-box
rows. Run the staged legacy suite twice once to detect state leaking between roots.

**Migration checks:** both oracle `cmp` and `tests/unit` diff commands above; no production installation change; no
Go Bats run until a Go binary exists.

### Round 1 — Go foundation: configuration, metadata, logging, process boundaries (~1,400–1,600)

**Implementation steps**

1. Add `agent/go.mod`, minimal `main.go`, `App`, typed error infrastructure, and build/lint/doccheck integration.
2. Implement the NUL-delimited Bash configuration loader, arrays/defaults, secret environment loading, path
   resolution, and no-secret diagnostics.
3. Implement the `%q` codec, `meta.env` reader/writer, lock timeout/stale-run behavior, job transition journaling,
   task lookup, structured logs, and the narrow process/clock interfaces.

**Tests introduced first**

- Golden configuration fixtures covering documented arrays, empty vs unset defaults, legacy investigator settings,
  expansions, malformed configuration, and secret values with whitespace/newlines.
- Golden `%q` fixtures and legacy metadata fixtures; concurrent different-key updates, stale `RUN`, read/write/rename
  failures, lock timeout, unknown lines, and mixed Bash/Go writers.
- Exact log JSON fields and process argv/env/cwd/exit propagation. Port `meta.bats` and the three metadata-failure
  cases from `node_checkout.bats` now; leave the other node rows at their later rounds.

**Verification**

```bash
cd agent && go test -run 'Test(Config|Secrets|Meta|ShellQuote|Log|Process)' ./...
cd agent && go test -race -run 'TestMeta|TestMixedMeta' ./...
make gate
```

**Migration checks:** read the legacy `agentbus/testdata` and live metadata fixtures where applicable, prove old
`meta.env` remains readable, run all common immutability checks, and confirm no new dependency appears in `go.mod`.

### Round 2 — CLI shell, read-only commands, and the trusted `gh` boundary (~1,300–1,500)

**Implementation steps**

1. Add exact command dispatch/usage and exit handling, including `filepath.Base(os.Args[0]) == "gh"` before
   configuration loading.
2. Port `status`/`ls`, `agents`, `show`, `log`, `inv` listing, `notes`, `helpers`, `bus`, `_agents-json`, `ack`, and
   pass-through `live`/`web` process execution. Mutating `team --close`, `inv` dispatch, and runner behavior remain in
   their own rounds.
3. Port the `agent as gh` trust filter: list/view/api behavior, GraphQL refusal, fork/unknown-author filtering,
   marked-comment filtering, byline injection boundary, logging, and real-`gh` argv passthrough.

**Tests introduced first**

- Table-driven stdout/stderr/exit tests and fake `agentbus`/`agentlive` argv tests.
- Existing `agent_gh.bats`, `web.bats`, and genuinely read-only CLI black-box rows in the staged Go suite.
  `github_trust.bats` needs teams, prompts, reviews, and autopilot, so its black-box rows stay planned until Round 10;
  only its standalone trust-classification logic receives Go unit tests in this round.
- Go tests for malformed/missing GitHub JSON, paginated arrays, ignored content, no real-`gh` configuration, and the
  absence of configuration loading for `gh` invocation.

**Verification**

```bash
cd agent && go test -run 'Test(CLI|Status|Show|AgentGH|GitHubTrust)' ./...
make unit-agent-go BATS_FILES='tests/unit/agent_gh.bats tests/unit/web.bats tests/unit/agent.bats'
make gate && make unit-agent-go
```

**Migration checks:** compare command output and exit statuses against the oracle on identical temporary fixtures;
verify a `gh` symlink resolves to the same executable in both staged roots; run the common diff/hash checks.

### Round 3 — Task allocation, Git worktrees, run/resume creation, and cleanup (~1,400–1,600)

**Implementation steps**

1. Port task/run ID allocation and lock ordering, `create_task`, atomic failure cleanup, worktree/branch creation,
   base/commit pinning, task metadata, and agent reservation checks.
2. Port `run` argument validation, resume-chain construction, same-round behavior, WIP preservation, `resume`,
   `discard`, `clean`, and local stop/admission fences needed by creation.
3. Keep actual CLI execution behind the not-yet-wired runner boundary; test task creation directly and through
   commands whose runners are stubbed at the process dependency.

**Tests introduced first**

- Real temporary Git repositories for detached/pinned bases, moved branches, missing refs, non-GitHub origins,
  existing worktrees, and cleanup after each creation stage.
- Fake process/filesystem faults for every partial write, concurrent allocation, stop-during-admission, duplicate
  resume, and branch/worktree preservation.
- Port the generic `create_task`/worktree source rows from `node_checkout.bats`; prepare-job-specific rows remain in
  Round 12. Keep `resume_alloc` and `resume_stash` black-box rows planned: their scenarios wait for real (stubbed)
  runners, so solo rows belong to Round 6 and team rows to Round 7. Round 3 verifies allocation/resume construction at
  its own boundaries instead of activating those end-to-end cases early.

**Verification**

```bash
cd agent && go test -run 'Test(CreateTask|Allocate|Worktree|RunCommand|Resume|Discard)' ./...
cd agent && go test -race -run 'Test(Allocate|TaskAdmission|StopFence)' ./...
make gate && make unit-agent-go
```

**Migration checks:** compare `meta.env`, branch names, worktree paths, and cleanup trees with the oracle; prove a
failed creation leaves neither task nor worktree; run all immutable-baseline checks.

### Round 4 — Prompt rendering, questions, checkpoints, and fresh-session inputs (~1,300–1,500)

**Implementation steps**

1. Port placeholder rendering and all prompt variants, repository/AGENTS instructions, environment notes, identity
   trailers, question history, checkpoints, round-only stripping, and long-prompt file handoff.
2. Port question counting/validation/expansion, editor and answer watchers, notification text, and local answer/resume
   transitions.
3. Port usage-history calculations and fresh-session decision inputs; the runner consumes them in Round 5–6.

**Tests introduced first**

- Golden prompts from all 25 current templates with every placeholder populated, no `{{` remaining, exact ordering,
  resumed answers, no duplicated clarifications, and checkpoint truncation.
- Source-equivalent tests for `prompt_checkpoints`, `round_only`, `fresh_relative`, question validation, and prompt
  size boundaries. Investigation-specific prompt rows remain in Round 8–9.
- Keep end-to-end `prompt_file`, `context_hygiene`, and local-answer Bats rows planned here: they require the Round 5
  adapters, Round 6 runner, and in some cases Round 7 team closure. This round verifies prompt/question behavior at
  its Go boundaries rather than claiming those process-level rows early.

**Verification**

```bash
cd agent && go test -run 'Test(BuildPrompt|Render|Questions|Checkpoints|FreshSession|RoundText)' ./...
make gate && make unit-agent-go                 # no new end-to-end Bats activation in this round
```

**Migration checks:** diff rendered prompts against the Bash oracle over a shared fixture matrix, verify no secret
appears in prompts/logs, and check all common baseline/hash commands.

### Round 5 — CLI adapters, MCP configuration, activity, and watchdogs (~1,400–1,600)

**Implementation steps**

1. Port Claude, Codex, Gemini/Antigravity, and OpenCode argv/environment/config generation, including independent
   investigation MCP isolation and the full disabled OpenCode server schema.
2. Port per-run files, prompt argument/file selection, activity normalization calls, usage records, process-group
   startup/termination, hang detection, and long-run soft/hard limits.
3. Port investigator-independent adapter helpers without implementing investigation queue policy yet.

**Tests introduced first**

- Fake CLI scripts that record exact argv/env and emit each supported stream shape; malformed/truncated stream and
  nonzero-exit cases.
- Table-driven MCP tests for all four CLIs and legacy/independent modes, replacing source rows in
  `investigation_mcp_off`, `investigation_team_isolation`, and `investigation_team_opencode_schema` now because they
  test adapter behavior, not team orchestration.
- Deterministic fake-clock watchdog tests and the two `long_run_cut` Go equivalents; race tests for output/watchdog
  coordination and kill-only-once behavior.

**Verification**

```bash
cd agent && go test -run 'Test(Claude|Codex|Gemini|OpenCode|MCP|WatchRun|LongRun)' ./...
cd agent && go test -race -run 'Test(WatchRun|LongRun|Activity)' ./...
make gate && make unit-agent-go
```

**Migration checks:** compare adapter argv/config with oracle golden files, ensure no real CLI is invoked, verify MCP
commands still execute `agentbus serve`, and run all baseline/oracle checks.

### Round 6 — Runner state machine, limits, notifications, and solo lifecycle (~1,500–1,700)

**Implementation steps**

1. Port `cmd__runner`, startup/resume session handling, final `STATUS:` parsing, runner exit traps, final-message and
   run-note persistence, and exactly-once finalization.
2. Port quota/rate-limit/workspace/network classification and reset-time parsing, pause/wake scheduling, restart
   handling, notification files/hooks, and basic `wake`/`kick`/`stop` behavior.
3. Wire solo task end-to-end behavior; team callbacks remain behind the explicit Round 7 boundary.

**Tests introduced first**

- Source-equivalent tests for quota reset (`agent.bats`), local error classification (`auto_stage`), network errors,
  `run_notes`, fresh-session transitions, and long-run finalization.
- Table-driven final statuses, missing/fallback questions, crashed CLI, dead runner, paused task, stale exit from an
  old run, notification failure, and partial `RUNNER_DONE` recovery.
- Activate solo black-box rows from `agent.bats`, `limits.bats`, `resilience.bats`, `restart.bats`, `pause_notify`,
  `prompt_file`, `run_notes`, and local answer/wake/kick tests only after each complete command path works.

**Verification**

```bash
cd agent && go test -run 'Test(Runner|FinalStatus|Limits|Pause|Notify|Wake|Kick)' ./...
cd agent && go test -race -run 'Test(Runner|Stale|Watchdog)' ./...
make unit-agent-go BATS_FILES='tests/unit/agent.bats tests/unit/limits.bats tests/unit/resilience.bats …'
make gate && make unit-agent-go
```

**Migration checks:** compare task status/final/note files and structured events for every terminal outcome; verify
old pause metadata resumes unchanged; confirm no real systemd or notification hook is touched; run common checks.

### Round 7 — Teams, workers, board coordination, and local supervision (~1,400–1,600)

**Implementation steps**

1. Port team creation/listing/closing, coordinator/member post-run transitions, sleeping/waking, member exclusion,
   board-item release, and team completion.
2. Port worker selection/provider balancing, helper admission/completion, delegation wakeups, and local investigation
   wait relationships needed by teams.
3. Port the local parts of `_supervise`, stop-all, wake-loop protection, stale-member handling, and remote exclusion
   rules without adding remote/node policy yet.

**Tests introduced first**

- Team state-machine tables for each member status, coordinator handoff, worker failure, board progress return, team
  close, and abandoned team.
- Concurrent dispatch/admission tests, provider balancing, sleeping reservation, and stop-all wake prevention.
- Activate black-box `lead*`, `coordination`, `context_hygiene`, `member_out`, `team_close`, `delegate_jobs` local
  paths, `resume_members`, and team-related `agent.bats` rows once their exact team dependencies are present. Port
  `remote_fixes`' local/remote wake-loop predicate as a Go unit test here, while its remote integration remains in
  Round 11.

**Verification**

```bash
cd agent && go test -run 'Test(Team|Member|Helper|Worker|Supervise|WakeLoop)' ./...
cd agent && go test -race -run 'Test(TeamDispatch|HelperAdmission|StopAll)' ./...
make unit-agent-go BATS_FILES='tests/unit/lead.bats tests/unit/coordination.bats tests/unit/delegate_jobs.bats …'
make gate && make unit-agent-go
```

**Migration checks:** compare bus commands/notices and team/member state with the oracle, verify sleeping agents stay
reserved and failed creation releases them, and run all baseline/oracle checks.

### Round 8 — Single investigations, investigator rotation, and clarification resumes (~1,400–1,600)

**Implementation steps**

1. Port investigation queue admission/listing/dispatch, free-investigator selection, provider-aware rotation, request
   persistence, start/finish, requester wakeup, and legacy single-investigator configuration.
2. Port investigation resume/clarification state, independent prompts, terminal completion, publication retry, and
   the investigation portions of dispatch/supervision.
3. Connect team/helper wait resolution without implementing independent investigation-team phases.

**Tests introduced first**

- Source-equivalent `rotation` tests, capacity/free-list tests, investigator busy/waiting/quota states, queue limits,
  duplicate completion, missing result, and requester wake exactly once.
- Golden clarification/resume prompts and recovery at every persisted phase; port the source row in
  `investigation_prompt_clarification_resume` if not already covered by adapter tests.
- Activate `investigation_single_*`, `investigation_clarification`, `investigation_terminal_completion`,
  `investigation_pipeline_clarification` single-investigator paths, and `investigation_completed_legacy` fixtures.

**Verification**

```bash
cd agent && go test -run 'Test(Investigation|Investigator|Rotation|Clarification)' ./...
cd agent && go test -race -run 'Test(InvestigationAdmission|InvestigationFinish)' ./...
make unit-agent-go BATS_FILES='tests/unit/investigation_single_pipeline.bats tests/unit/investigation_clarification.bats …'
make gate && make unit-agent-go
```

**Migration checks:** load legacy investigation metadata/results, compare queue and rotation order against the oracle,
verify independent MCP settings from Round 5 remain intact, and run common immutability checks.

### Round 9 — Independent investigation teams and phase recovery (~1,500–1,800)

**Implementation steps**

1. Port topic/team validation, phase leases and capacity admission, phase 1/2 files, deterministic seed handling,
  merger/reviewer selection, publication, handoff, and cleanup.
2. Port `_investigation-team`, stop/dispatch/supervise interactions, reboot recovery, partial child creation, stopped
  parent cleanup, and ambiguous outcome handling.
3. Complete independent clarification prompts and investigation-team CLI/schema behavior left after Round 5/8.

**Tests introduced first**

- Source-equivalent tests for `investigation_team_admission`, `investigation_team_runtime`, and all five helper-coupled
  `investigation_team_recovery_regressions` cases, with deterministic seeds and fake clocks.
- Crash-window tests around lease, phase file, child task, worktree, publication, and handoff writes; non-decimal seed,
  stopped parent, lingering lease, precheck cancellation, and capacity release.
- Activate only the `investigation_team_*` and independent resume/phase-selection rows whose dependencies are now
  complete. Cases involving review findings, prechecks, or failed-check handoff stay planned for Round 10 even when
  they share a file with activated cases.

**Verification**

```bash
cd agent && go test -run 'Test(InvestigationTeam|InvestigationPhase|InvestigationRecovery)' ./...
cd agent && go test -race -run 'Test(InvestigationTeamAdmission|InvestigationLease)' ./...
make unit-agent-go                              # manifest selects only dependency-complete Round 9 rows
make gate && make unit-agent-go
```

**Migration checks:** replay legacy phase artifacts and completed handoffs, compare phase JSON and bus notices, prove
capacity is neither leaked nor double-admitted, and run all common baseline/oracle checks.

### Round 10 — Rounds, reviews, prechecks, fix passes, and autopilot (~1,500–1,800)

**Implementation steps**

1. Port round limits/size/test-change/skip analysis, PR discovery/trust/format checks, review verdict/findings/byline,
  investigator review pairs, seeded reuse, and review retry/resilience.
2. Port gate prechecks, fix passes, PR supersession, `--verify`, `--autopilot`, merge blockers, merge, next-round
  creation, and related supervision.
3. Complete `round-size`, `autopilot`, `_precheck`, `_fixpass`, and `_merge` command paths.

**Tests introduced first**

- Go equivalents for every source row in `autopilot_merge`, `autopilot_review`, `findings`, `github_format`,
  `one_pr_per_round`, `review_byline`, `review_pair_random`, `review_resilience`, `round_handover`, and `skips`.
- Pure parser tests, real temporary Git test-change/gate tests, deterministic review-pair tests, fake `gh` trust/PR
  fixtures, merge hold paths, and no-duplicate-PR/next-round tests. Add compatibility goldens for configured
  `AGENT_REVIEW_GATE` strings executed through the retained narrow `bash -c` boundary, including quoting, exit status,
  output, and `auto`/`off`.
- Activate `verify_loop`, `rounds`, `round_only`, `round_reports`, `one_pr_per_round`, `review_*`, `autopilot_*`,
  `gate`, `github_trust`, and `round_handover` black-box rows, plus the investigation-team review/precheck/handoff
  rows deliberately left planned in Round 9.

**Verification**

```bash
cd agent && go test -run 'Test(Round|Review|Autopilot|Findings|Skip|PRFormat|Precheck)' ./...
make unit-agent-go BATS_FILES='tests/unit/rounds.bats tests/unit/verify_loop.bats tests/unit/autopilot_*.bats tests/unit/review_*.bats …'
make gate && make unit-agent-go
```

**Migration checks:** compare review verdicts, stage strings, PR commands, and merge decisions with the oracle; ensure
existing-test changes still hold autopilot by default; verify untrusted PR content never reaches an agent; run common
checks.

### Round 11 — Lead listener, remote members/jobs, and remote questions (~1,400–1,600)

**Implementation steps**

1. Port lead/node HTTP command wrappers, remote job submission and canonical GitHub attribution, capability/instance
  selection, `jobs`, `nodes`, and `lead` foreground lifecycle.
2. Port remote-member proxy creation/start/status, handover failure, commit-pushed checks, remote wake/kick/stop, and
  lead-side remote supervision.
3. Port remote question preparation, validation, atomic queueing, notifications, answer watchers, and ready/saved
  commands.

**Tests introduced first**

- Go equivalents for `handover_failed`, `job_attribution`, `job_attribution_paths`, and lead-side remote control source
  rows. Use loopback HTTP/fake bus servers; never use a real lead or node.
- Job attribution origin matrices, queue/cancel retries, proxy status idempotence, missing lead checkout, stale run
  controls, question size/digest/path validation, and notification links.
- Activate `remote_proxy`, `remote_controls`, `remote_questions_*`, `remote_answer_round`, `lead_cli*`, `web_jobs`,
  `job_attribution*`, and lead-side `remote_control_*` black-box rows.

**Verification**

```bash
cd agent && go test -run 'Test(Lead|RemoteJob|RemoteMember|RemoteQuestions|Attribution)' ./...
cd agent && go test -race -run 'Test(RemoteQuestion|RemoteControl)' ./...
make unit-agent-go BATS_FILES='tests/unit/remote_proxy.bats tests/unit/remote_questions_*.bats tests/unit/job_attribution*.bats …'
make gate && make unit-agent-go
```

**Migration checks:** compare remote JSON field names and bounded payloads with the oracle, verify no human requester
 task is invented, check old proxy metadata remains usable, and run common baseline/oracle checks.

### Round 12 — Node state, admission, launch fences, recovery, and heartbeat inventory (~1,500–1,800)

**Implementation steps**

1. Port node state JSON loading/validation/atomic saving, daemon lifetime and assignment locks, `prepare_job_task`,
  repository/capability admission, and agent reservation.
2. Port prelaunch cancellation, start acknowledgement, stop/admission ordering, recovery fences, ambiguous task
  binding, partial identity recovery, and final result preservation.
3. Port heartbeat agent inventory/activity and node transition/result generation; polling and synchronization remain
  in Round 13.

**Tests introduced first**

- Go equivalents for all source rows in `node_activity`, `node_agents`, `node_cancel_exit`, `node_checkout`,
  `node_daemon`, `node_prelaunch_cancel`, `node_recovery_fence`, `node_report_agent`, `node_review_boundaries`, and
  `node_safety`, except generic creation/metadata rows already marked done in Rounds 1/3.
- Malformed JSON/shape/path tests, lock contention, every launch/recovery crash window, cancellation outcome matrix,
  unknown/investigator/busy agent refusal, missing CLI inventory, and ambiguous identity matrices.
- Activate only node admission/recovery black-box rows whose paths are complete; use temporary homes and stub
  systemd/bus. Heartbeat polling, stop controls, retries, and transition reporting in `node_daemon.bats` stay planned
  for Round 13. The source-equivalent `prepare_job_task` unit test is not a reason to activate the whole file.

**Verification**

```bash
cd agent && go test -run 'Test(NodeState|NodeAdmission|NodeRecovery|NodeHeartbeat|NodeCancel)' ./...
cd agent && go test -race -run 'Test(NodeAdmission|NodeRecovery|NodeLock)' ./...
make unit-agent-go BATS_FILES='tests/unit/node_checkout.bats tests/unit/node_recovery_fence.bats tests/unit/node_review_boundaries.bats …'
make gate && make unit-agent-go
```

**Migration checks:** load legacy `node-jobs.json` and task fixtures without stricter rejection, compare recovery
events and terminal statuses, prove ambiguous recovery does not reserve/destroy a task, and run common checks.

### Round 13 — Node polling, bounded controls, replies, and state/log synchronization (~1,500–1,800)

**Implementation steps**

1. Port heartbeat polling, bounded acknowledgement draining, answers/kicks/stops/replies, cancellation ordering,
  remote helper polling, doorbell lifecycle, and `--once`/daemon operation.
2. Port task scanning, metadata/log/activity/report mirroring, freshness/tail limits, obsolete-job handling, large
  assignments, and multi-home/multi-node behavior.
3. Complete remote `wake`/`kick` integration and remaining remote-control source equivalents.

**Tests introduced first**

- Go equivalents for remaining source rows in `remote_control_cancel_order`, `remote_control_heartbeat_limits`,
  `remote_control_kick_exit`, `remote_control_notifications`, `remote_control_review`,
  `remote_control_validation`, and `wake_cmd`.
- Adversarial payload tables (unknown fields, traversal, wrong task, bad digest, >256 KiB, oversized lists), stale
  control/ack tests, cancellation-before-answer, partial sync/retry, bounded logs, and second-home isolation.
- Activate all `node_sync*`, `node_two_homes`, `node_helper_hooks`, `node_large_assignment`, `delegate_jobs` remote
  paths, and remaining `remote_control_*` black-box rows.

**Verification**

```bash
cd agent && go test -run 'Test(NodePoll|NodeControl|NodeSync|RemoteAnswer|RemoteKick)' ./...
cd agent && go test -race -run 'Test(NodePoll|NodeControl|NodeSync)' ./...
make unit-agent-go BATS_FILES='tests/unit/node_sync*.bats tests/unit/remote_control_*.bats tests/unit/node_two_homes.bats'
make gate && make unit-agent-go
```

**Migration checks:** compare mirrored files and acknowledgement retention byte-for-byte on shared fixtures, verify
controls are data rather than executable input, test mixed old-lead/new-node and old-node/new-lead compatibility, and
run all common baseline/oracle checks.

### Round 14 — Authentication, archives, transcripts, tokens, and remaining commands (~1,300–1,500)

**Implementation steps**

1. Port `authenticated`/`_authcheck`, provider/model selection, loopback-testable HTTP probes, CLI/provider error
  classification, and provider-specific reset/error text without exposing keys.
2. Port archive indexing/grouping across resume chains, investigations and helpers; 7z/7za/7zr selection, archive
  verification, atomic removal, `archive --finished`, and `discard` edge cases.
3. Port transcripts, token/run accounting, remaining `round-size` output if not completed in Round 10, `say`, `clean`,
  and any command still present in the dispatch inventory.

**Tests introduced first**

- Loopback HTTP status/body/timeout tests, fake CLI outputs, missing-key/model tests, exact provider diagnostics, and
  no-secret log assertions.
- Real temporary archive tests with nested task groups, existing destination, failed 7z, verification failure,
  read-only/metadata failures, and proof that source tasks are removed only after a valid archive.
- Golden chronological transcript and usage aggregation tests, malformed activity lines, helper/investigation
  inclusion, and human-readable vs raw output.

**Verification**

```bash
cd agent && go test -run 'Test(Auth|Archive|Transcript|Tokens|CommandInventory)' ./...
make unit-agent-go BATS_FILES='tests/unit/authenticated.bats tests/unit/tokens.bats tests/unit/agent.bats'
make gate && make unit-agent-go
```

**Migration checks:** compare all 45 `cmd_` functions, `dispatch_helpers`, and the `gh` entry point with the dispatch
inventory; ensure no command remains unimplemented; compare archive indexes and token totals with the oracle; run
common baseline/oracle checks.

### Round 15 — Production cutover and full compatibility gate (~800–1,200 plus regression budget)

**RED tests before any cutover change**

1. Add new cutover tests outside `tests/unit` (for example `tests/agent_go_cutover.bats`). Use a clean temporary
   home with no installed `agent`, unset `AGENT_PREBUILT_AGENT`, stub systemd, and a test `go` wrapper that delegates
   only the `agent/` build to the real compiler while replacing the other module builds as the existing installer
   tests do. Assert that installation creates an executable Go `agent`, that a `gh` symlink still takes the fast path,
   and that reinstall preserves configuration. The test must fail against the Bash-copy installer first.
2. Add a second clean-home case in which the delegated `agent/` compiler fails: installation must fail and must not
   install or fall back to an old binary. Add the complementary prebuilt-mode case needed by the unchanged no-op-`go`
   installer tests. Add a harness/gate test proving the default `tools/gate.sh` step list actually executes
   `unit-agent-go` and applies Bats failure excerpts to it. Commit these tests and their failing output before the
   implementation; the existing install Bats files alone cannot be the RED test because they never assert a Go build.

**Implementation steps**

1. Move the unchanged `bin/agent` to `tests/oracle/agent` as a 100% rename, point the legacy stager at it, and keep
   `bin/agent` as a generated, ignored staging/build output only—never a committed ELF or shell launcher.
2. Change `install.sh` to build `agent` with `go build -trimpath -ldflags='-s -w'` directly into the selected
   temporary/install home, just as it builds the other Go binaries. Also support an explicit
   `AGENT_PREBUILT_AGENT=<path>` input: the staged harness sets it to the actual candidate (or oracle) binary because
   the unchanged install tests stub `go` with `exit 0`. Without that variable, installation always builds and a
   compiler error fails the installation; never fall back silently to a stale binary. Update `Makefile`, `GO_MODULES`,
   doccheck, generated-error checks, `SHELL_SCRIPTS`/`bash -n` (moving the oracle entry with it), README
   installation/testing text, and configuration comments that still call the orchestrator a script.
3. Make `make test` run `agent` Go tests, all 480 legacy Bats cases, and all current black-box Go Bats cases. Update
   the hard-coded `STEPS` list in `tools/gate.sh` to include `unit-agent-go`, and include that target beside
   `unit-bats` in its failure-excerpt selection. Fix any cutover regression only after adding a failing Go regression
   test; never edit an existing Bats test.

**Tests and verification**

```bash
make build
bats tests/agent_go_cutover.bats                 # new clean-home build/failure/prebuilt cases
make unit-bats                                  # 480/480 unchanged legacy cases
make unit-agent-go                              # 359/359 current black-box cases; zero new skips
cd agent && go test ./... && go test -race ./...
make gate
cmp <(git show "$BASE":bin/agent) tests/oracle/agent
git diff --exit-code "$BASE" -- tests/unit
git status --short -- tests/unit
```

Run the install-related unchanged Bats files in both modes and perform an isolated install with stub `systemctl` and a
temporary `HOME`; verify the installed file is executable Go, systemd units still point at `agent`, and reinstall does
not overwrite configuration. Check JUnit totals rather than relying on the process exit alone. Run
`agent round-size <round-task>` and confirm the oracle was detected as a rename, not an 8,583-line deletion.

**Migration checks:** every one of the 121 current source rows is `done` and names a passing Go test; every black-box
row ran against Go; all 25 prompts render without placeholders; old configuration/metadata/node/review/investigation
fixtures load; no real agent CLI, real account, running service, or real `~/agents` was used. Do not run
`make integration` unless the human explicitly requests and funds it.

## Final acceptance matrix

The migration is complete only when all of these are true:

| Contract | Evidence |
|---|---|
| Existing Bats are immutable | `git diff`/`git status` against `tests/unit` are empty; oracle `cmp` succeeds |
| Bash behavior remains a baseline | 480/480 legacy cases pass in the staged root |
| Go process-level behavior | All current black-box cases (359 at the base) pass against the compiled binary |
| Go function-level behavior | All current source-coupled cases (121 at the base) have named, passing Go equivalents |
| Build and static checks | `make gate` passes with zero findings and explicitly runs `unit-agent-go`; `agent` is included in vet/lint/doccheck/generation |
| Concurrency | `go test -race ./...` passes, including metadata, admission, watchdog, controls, and sync tests |
| Persistent compatibility | Legacy metadata, phase, node, configuration, logs, prompts, and archive fixtures are readable |
| Installation | Isolated install/reinstall tests install Go without touching real configuration or services |
| Round discipline | Each RED/GREEN/REFACTOR cycle is visible; no new skips; each round is within the hard size limit |

## Risks and explicit non-solutions

- **Changing Bats to call Go helpers:** rejected. It destroys the immutable baseline and hides migration mistakes.
- **Keeping `bin/agent` as a Bash wrapper:** rejected. It preserves startup/config global-state problems and makes the
  copied executable dependent on a second path; it is not the requested rewrite.
- **Running only the 359 black-box tests:** rejected. That silently drops 121 current behavioral checks.
- **Writing a Go parser for arbitrary Bash configuration:** rejected for this migration. The narrow Bash loader is the
  compatibility boundary; a future versioned configuration format needs its own migration and dual-read tests.
- **Importing `agentbus` internals or rewriting bus access:** rejected. Continue invoking its stable CLI so there is
  one bus locking/state implementation and its exit-code contract remains explicit.
- **A single final translation/big-bang test:** rejected. The 16 rounds above keep reviewable behavior boundaries and
  ensure each new Go behavior has a failing test first.
- **Deleting the oracle at cutover:** rejected. Preserve it unchanged under `tests/oracle/agent`; otherwise the
  original 480-case suite cannot continue to run and future compatibility investigations lose their reference.
