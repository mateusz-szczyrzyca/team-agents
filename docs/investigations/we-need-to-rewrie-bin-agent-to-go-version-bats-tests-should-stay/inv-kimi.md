# Migrating `bin/agent` from Bash to Go without losing the Bats safety net

## Scope and recommendation

This is an implementation plan, not a request to rewrite the script in one change. The current
`bin/agent` is the executable specification: it contains task creation, runners, provider-specific CLI
setup, teams, investigations, remote nodes, authentication, archiving, installation-facing behaviour and
the `gh` trust filter. A big-bang rewrite would make a failure difficult to localize and would encourage
rewriting the tests to match new behaviour.

Use a **parallel, test-first strangler migration**:

1. Keep the Bash `bin/agent` and `tests/unit/` byte-for-byte unchanged during all development rounds.
2. Build a new, standard-library-only Go module in `agent/` as `build/agent`.
3. Port one domain at a time. Run the existing Bats scenarios against the Bash implementation and the
   same, unmodified process-level scenarios against the Go binary in a disposable test root.
4. Add equivalent Go tests before each Go implementation change, reusing the existing stubs and their
   scenario protocol without editing them.
5. Do not make the Go binary call the Bash implementation as a fallback. A fallback hides unported
   behaviour and makes a passing Go test meaningless.
6. Cut over only after the complete compatibility ledger is green, the white-box test issue below has an
   explicit human decision, installation and rollback have been tested in an isolated HOME, and
   `make gate` passes.

The suggested end state is a Go executable with domain files such as `agent/run.go`, `agent/team.go`,
`agent/archive.go` and `agent/authenticated.go`; Bash is no longer the implementation of agent operations.
The existing Bash configuration syntax remains a compatibility boundary, discussed below.

## A hard test constraint that needs a decision before cutover

The current suite has **462 tests in 110 Bats files (12,372 lines)**. Of these:

- **369 are process-level tests**: they can execute a Go `agent` without changing the test body.
- **93 are white-box tests**: they source `bin/agent` or extract Bash functions and call them directly.

A pure Go executable cannot be sourced as Bash. Therefore “replace `bin/agent` with Go” and “keep every
Bats file byte-for-byte unchanged forever” are incompatible requirements for those 93 tests. Keeping a
second Bash implementation only for those tests would test dead compatibility code, not the Go program,
and is not an acceptable solution.

Recommended resolution, to be approved explicitly before the cutover round:

- Keep all 110 Bats files and all 462 test names as the executable behaviour specification.
- Leave the 369 process-level tests unchanged.
- Convert the 93 white-box tests, one at a time, to equivalent Go unit coverage while retaining every Bats
  file and test name. Each Bats test either becomes a public-CLI scenario or a thin invocation of its named
  Go unit test. Do not add production `_test-*` commands merely to make internal tests pass.
- Review the 93 conversions as a separate test-only round while the Bash implementation is still present.
  Every converted assertion must remain represented; no skipping or weakening is allowed.
- If byte-for-byte preservation of those 93 tests is mandatory, stop before cutover: a pure Go replacement
  cannot satisfy that constraint. This is a requirements decision, not something the implementation should
  work around silently.

Until that approval, the normal repository rule applies: no existing test, helper or stub is modified.

## Baseline to protect

The investigation base is `f6a602465a7d30e4b6a6737515e67e056f757a05`.

Recorded inventory:

| Item | Baseline |
|---|---:|
| `bin/agent` | 8,523 lines, 312 functions |
| Bats suite | 110 files, 462 tests, 12,372 lines |
| White-box Bats tests | 93 |
| Process-level Bats tests | 369 |
| `bin/agent` SHA-256 | `461b7f65ecacd9496302fd351f8acc9d0aafdc8fde06b6ff2312f570c8ca7413` |
| Ordered test-content aggregate | `fca84f91a741e8ae428814aefab4824af711ae70b18101776996ad47aca5550e` |

Baseline verification on 2026-11-02: `check-generated`, `lint-shell`, `lint-go`, `lint-docs` and `unit-go`
passed. The full `unit-bats` step failed two tests, and both failures reproduced when run individually:

- `investigation_team_isolation.bats`: `codex invocation excludes agentbus args for investigation team task
  but includes them for legacy investigator` (the first exclusion assertion);
- `investigation_team_opencode_schema.bats`: `opencode config for independent investigation emits strictly
  valid disabled mcp object with enabled false only`.

No implementation or test files were changed by this investigation. These are pre-existing baseline failures,
not Go migration results. Restore a clean baseline before implementation rounds; do not weaken either test or
describe the overall gate as green while they fail.

Before every migration commit, and again after it, verify that development rounds did not touch the
specification:

```sh
git diff --exit-code f6a602465a7d30e4b6a6737515e67e056f757a05 -- bin/agent tests/unit
make gate
```

The final approved cutover is the only round allowed to differ for `bin/agent` and the specifically approved
white-box test conversions.

### What is part of compatibility

Compatibility is broader than command names. Treat each item below as a checklist entry in the ledger.

1. **CLI protocol**: public and internal subcommands, aliases, option order, `--`, missing arguments, stdout
   versus stderr, output whitespace, exit status and signal behaviour. The dispatcher begins at the end of
   `bin/agent`; do not design a new CLI from the README alone.
2. **Self-invocation**: `$SELF` is used for `_runner`, `_wake`, `_precheck`, answers, node work and other
   deferred commands. Invocation through a symlink named `gh` selects the trust filter before normal
   configuration loading. `os.Executable`, `os.Args[0]` and symlink resolution need explicit parity tests.
3. **Configuration**: `agents.conf` and `secrets.env` are sourced Bash, including associative and indexed
   arrays, defaults and expansions. Unset, empty and malformed values are not interchangeable.
4. **On-disk state**: `tasks/*/meta.env` uses Bash `printf %q` quoting; lock files, prompts, questions,
   answers, checkpoints, `activity.jsonl`, runner logs, node state and notification records are read by
   other components and existing installations. Old files must remain readable.
5. **Process boundaries**: `systemd-run`, `systemctl --user`, `git`, `gh`, `jq`, `flock`, `setsid`, `7z`,
   `curl`, `logger`, `ssh`, provider CLIs and `agentbus` have observable argv, environment and exit-code
   contracts. Large task and JSON payloads that currently use stdin must not move into argv.
6. **Concurrency**: metadata updates, task ID allocation, dispatch, answers, wake and node-daemon locks;
   atomic file replacement; stale PID/unit handling; stop-versus-completion races; watchdog and resume
   fencing. File descriptors 3 and 7–10 have current redirection/locking roles.
7. **Security**: the `gh` wrapper fails closed for untrusted forks/authors, filters comments and reviews,
   refuses protected GraphQL reads and adds review bylines. Authentication must never print secrets.
8. **Cross-component consumers**: `agentlive`, the web interface, remote leads/nodes and the MCP bus depend
   on stable JSON field names, timestamps, metadata values and log event names.
9. **Installation**: `install.sh` currently builds `agentbus` and `agentlive` but installs `bin/agent`
   directly. The final Go binary, its permissions and its self-invocation path must work from the installed
   location, not only from the repository.

Do not tighten validation while porting. An old configuration or state value rejected by new Go code is a
breaking change even if the stricter behaviour looks cleaner.

## Target Go design

Create `agent/go.mod` with the same Go language version as `agentbus`, package `main`, and no third-party
dependencies. Build with:

```sh
cd agent && go build -o ../build/agent .
```

Keep the command dispatcher thin. An `App` value should own the resolved configuration, stdout/stderr,
self path and an external-command runner; pass it explicitly to operations. Use `context.Context` for
cancellation. Use real temporary directories and real local Git repositories in tests rather than mocking
the whole filesystem. Interfaces are justified at process and clock boundaries, not around every function.

`main` should call `run(ctx, args, environment) int` and only `os.Exit` once. Operations return typed
per-operation errors following the existing `agentbus` model. Expected non-error answers remain output.
There must be no ignored errors and no package-level mutable workflow state.

Suggested file boundaries (split further when a round approaches its size limit):

| File(s) | Responsibility |
|---|---|
| `main.go`, `cli.go` | invocation-name dispatch, exact argument parsing, exit-code mapping |
| `config.go`, `environment.go` | Bash-compatible configuration loading, defaults, agent/tier lookup |
| `logging.go`, `events.go` | structured JSONL/journal/runner logging and stable event names |
| `errors.go`, `errors.json`, `errors_gen.go` | typed operation errors; generated with the existing errgen pattern |
| `process.go`, `signals.go` | argv-safe execution, stdout/stderr/exit preservation, process groups |
| `fs.go`, `lock.go`, `meta.go` | atomic files, `flock` semantics, legacy `meta.env` read/write |
| `task.go`, `worktree.go` | IDs, metadata, repositories, branches and worktrees |
| `prompt.go`, `provider.go` | prompt rendering, provider environment and CLI configuration |
| `run.go`, `runner.go`, `watchdog.go` | task start, `_runner`, activity, completion, hang/watchdog handling |
| `lifecycle.go`, `resume.go` | stop/wake/kick/supervise/restart/resume and fencing |
| `questions.go`, `notify.go` | human rounds, remote answers, notifications and editor integration |
| `authenticated.go` | SSH/GitHub/provider authentication checks |
| `team.go`, `review.go`, `autopilot.go` | team lifecycle, review rounds, verify/fix/merge automation |
| `investigation.go`, `helper.go` | investigations, workers, dispatch and result publication |
| `github.go` | the `gh` symlink trust boundary and filtering |
| `lead.go`, `node.go`, `remote.go` | lead CLI, heartbeat, assignments, sync and remote helpers |
| `archive.go`, `log.go`, `ui.go` | archives, task display/log rendering, `live`/`web` exec |
| `testenv_test.go`, domain `*_test.go` | one-to-one test ports and focused Go unit tests |

### Configuration: preserve Bash syntax through a narrow evaluator

Do not write a partial Bash parser and call it compatible. Existing configuration deliberately supports
Bash arrays and expansion. At the configuration boundary, run a fixed `bash --noprofile --norc` helper that
sources the same files in the same order and serializes only an allowlisted schema as length-safe records
(NUL-delimited or length-prefixed, including scalar/array type and unset versus empty). Decode that into Go
configuration structs. Never log the serialized secrets.

This is not a Bash fallback for operations: only the established configuration language is interpreted.
Removing Bash configuration syntax would be a separate, versioned configuration migration with its own
compatibility tests and documentation. `meta.env`, in contrast, should be parsed as data in Go rather than
executed.

### Processes, state and JSON

- Use `exec.Cmd` with argument slices, never shell concatenation. Preserve exact environment allowlists and
  working directories. Convert `exec.ExitError` to the original exit status; do not flatten every failure to 1.
- Keep Linux locking with `syscall.Flock` (standard library) or an initially wrapped `flock` process if that
  makes a round smaller. Preserve lock paths, blocking/non-blocking behaviour and timeouts; always handle
  close/unlock failures.
- Implement atomic sibling-temp-file writes with the current permissions and rename behaviour.
- Port `printf %q` metadata encoding with golden fixtures and differential tests against Bash, including
  empty values, whitespace, quotes, backslashes, Unicode, newlines and control bytes except NUL. Reuse the
  `live/data.go` decoder as a compatibility reference; old data must also load in `agentlive`.
- Prefer typed `encoding/json` for fixed internal JSON. Keep `jq` for user-supplied `--jq` expressions and
  initially for complicated fixed transformations where replacing it would add unneeded risk. Output
  compactness, array shape and error behaviour must match current tests.
- Model statuses, tiers, provider types and event names as typed constants. Time values need explicit
  formats and local-versus-UTC behaviour. Keep lock-protected ID allocation and test timestamp collisions.
- Handle process groups deliberately. `exec.CommandContext` alone only kills the direct child and is not an
  adequate port of the runner/stop behaviour. Test TERM, KILL, exited-PID races and orphaned children with
  the existing systemd stubs.

## Test strategy

### 1. Build a coverage ledger before implementation

Create a machine-readable ledger with one row for every current Bats `@test`:

```text
ID | Bats file:line | exact test name | process/white-box | Go test name | round | Bash result | Go result | notes
```

The initial ledger must total 462 rows, 369 process-level and 93 white-box. A CI/check script should compare
its IDs with Bats output so a newly added test cannot be omitted. The ledger tracks coverage; it does not
justify writing tests for future rounds early.

For each port, use names that make the mapping obvious, for example
`TestBatsMeta/printf_q_values_round_trip`. Keep a comment linking the Bats file and original test name.
A domain is done only when every one of its rows maps to a green equivalent assertion, not merely when its
Go file exists.

### 2. Reproduce the Bats environment in Go

Port `tests/unit/helpers.bash` mechanics into `agent/testenv_test.go`:

- temporary `HOME`, `AGENTS_HOME`, `STUB_HOME` and unit directory;
- the same generated `agents.conf`, prompts, empty secrets file, local bare remote and initial repository;
- `build/agent` under test and a freshly built `agentbus`;
- the existing files in `tests/unit/stubs/`, copied unchanged onto the test PATH;
- the existing `scenario` JSON protocol for fake agent CLIs;
- cleanup that terminates only process groups created by that test;
- condition polling with a deadline, equivalent to `wait_status`, `wait_grep`, `wait_member_state` and
  `wait_jq`, rather than fixed sleeps.

Use `t.TempDir` and `t.Cleanup`; tests need no network or real account. Do not share a HOME between tests.
Do not initially mark process tests parallel. Assert exit status, stdout/stderr, files, Git state and JSONL
records as the original test does. A Go port that only checks “no error” is weaker and does not count.

### 3. Run unmodified Bats against both executables

Continue to run the real suite against Bash:

```sh
bats tests/unit
bats -f 'metadata' tests/unit/meta.bats
```

For Go, use a disposable test root because the helper derives `REPO_ROOT` from the test location and installs
`$REPO_ROOT/bin/agent`. Copy the required directories without editing their contents, replace only the copy
of `bin/agent` with `build/agent`, then run Bats there:

```sh
rm -rf build/go-bats
mkdir -p build/go-bats/bin build/go-bats/tests
cp build/agent build/go-bats/bin/agent
cp -a agentbus prompts build/go-bats/
cp -a tests/unit build/go-bats/tests/unit
bats build/go-bats/tests/unit
```

During intermediate rounds, run only the process-level tests selected by the ledger; white-box copies still
try to source the executable and are not expected to work. This selection must be explicit, never a hidden
skip. The source suite remains the full 462-test Bash gate. At final cutover, after the approved white-box
conversions, the complete Bats suite runs against Go.

The shadow root is generated test material under `build/`; do not commit it and do not edit the copied tests
to make them pass.

### 4. Test-first sequence for every domain

1. **Characterize**: run the selected existing Bats test against Bash and save its exact observable contract
   in the new Go test. Where coverage is missing, add a new test rather than changing an old one.
2. **RED**: add only the current round's Go equivalent and run it against the incomplete Go implementation.
   Commit the failing test and its failing output as required by `docs/tdd.md`.
3. **GREEN**: implement the smallest Go code that satisfies the test and its existing Bats counterpart.
4. **Cross-check**: run the targeted Go test, the same process-level Bats test in the shadow root, and the
   original Bats test against Bash. Compare the ledger assertions.
5. **REFACTOR**: improve the Go structure without changing tests or behaviour.
6. **Gate**: run the domain checks, full Go tests, full Bats tests and `make gate`; verify the baseline diff.

Typical commands:

```sh
cd agent && go test -run 'TestBatsMeta' ./...
cd agent && go test -race ./...
bats -f 'wake' tests/unit
make gate
```

Do not run `make integration` or real provider CLIs as routine migration verification. They cost quota and
are not needed for the deterministic port. A final human-controlled smoke test can be requested separately.

## Migration rounds

Every row below may become multiple lettered rounds to stay within the 3,000-line PR limit (aim for 2,000).
Do not combine domains merely to reduce the number of PRs. Unimplemented commands in the development binary
return a typed “not implemented” error; the installed Bash implementation remains untouched.

### Round 0 — characterization and harness only

1. Add the 462-row ledger, invocation/exit-code inventory and persisted-format fixtures from the current
   implementation. Include legacy files, not just newly generated ones.
2. Add `agent/go.mod`, the minimal dispatcher/error skeleton, the Go test environment and the shadow-Bats
   runner. No workflow is ported yet.
3. Prove the shadow root runs selected process-level tests against Bash before substituting Go, so the
   harness itself is not mistaken for a production regression.
4. Checks: ledger totals and unique IDs; `go test ./...` in `agent`; full `bats tests/unit`; `make gate`;
   baseline diff.

### Round 1 — foundations and read-only compatibility

1. Port configuration evaluation, logging, error mapping, atomic files, locks and `meta.env` first.
2. Port simple read-only/task-metadata commands and exact usage/error handling. Establish self-path and
   invocation-name handling here.
3. Add golden tests for old metadata, malformed input, empty/unset values, concurrent writers and log JSON.
4. Representative checks: `meta.bats`, `agent.bats`, `identity.bats`, `limits.bats`, `tokens.bats`; race
   tests for metadata and ID allocation; full gate.

### Round 2 — the `gh` security boundary

1. Port symlink dispatch and the wrapper before workflows depend on it. Preserve pass-through argv and exit
   codes, `--jq`, pagination, review bylines, list/view filtering and GraphQL refusal.
2. Test trusted, untrusted, forked, marked and malformed responses; missing `AGENT_REAL_GH` must retain its
   exact failure behaviour. Confirm filtered content never reaches stdout.
3. Checks: `agent_gh.bats`, `github_trust.bats`, `github_format.bats`, `review_byline.bats` in both suites;
   no real `gh`; full gate.

### Round 3 — task creation, worktrees and prompts

1. Port task ID allocation, metadata initialization, repository/base-ref validation, branch/worktree setup,
   prompt rendering and provider-independent run argument construction.
2. Port prechecks and task text/file handling with large input through stdin. Preserve local Git identity
   and no-use-of-human-config behaviour.
3. Checks: prompt-file, run-notes, fresh-session/relative, round-only and limits scenarios; path names with
   spaces; local-only Git tests; full gate.

### Round 4 — runner and provider adapters

1. Port `_runner`, provider CLI configuration, MCP setup, activity normalization handoff, prompt/run files,
   status transitions and final-result parsing.
2. Keep `claude`, `codex`, `gemini` and `opencode` adapters separate behind a small provider interface, but
   preserve their individual arguments and schemas rather than forcing a false common denominator.
3. Port output/runner logging and notification handoff only to the depth needed for complete runs; later
   rounds own advanced lifecycle and autonomous review.
4. Checks: fake CLI scenarios for every provider, malformed/missing `STATUS`, activity and prompt golden
   files, `run_notes.bats`, `context_hygiene.bats`, `fresh_session.bats`, `long_run_cut.bats`; full gate.

### Round 5 — lifecycle, watchdog and resume

1. Port start/stop/wake/kick, systemd unit handling, supervision, hang detection, stop-request fencing and
   terminal status handling.
2. Port resume allocation, stash handling, member resume and restart paths. Preserve process-group cleanup
   and races between a natural finish and a stop.
3. Checks: `kick.bats`, `wake_cmd.bats`, `pause_notify.bats`, `restart.bats`, `resilience.bats`,
   `resume_alloc.bats`, `resume_stash.bats`, `resume_members.bats`; race-enabled Go tests; full gate.

### Round 6 — questions, notifications and authentication

1. Port question rounds, answer files and locks, remote-question preparation, editor opening and notification
   commands including legacy `NOTIFY_SCRIPT`.
2. Port `authenticated.go`: provider checks, SSH/GitHub handling, parallel systemd checks and sanitized
   diagnostics. No token or secret may appear in a test log.
3. Checks: `authenticated.bats`, `remote_answer_round.bats`, `remote_questions_*.bats`,
   `remote_control_notifications.bats`, notification failure records and answer races; full gate.

### Rounds 7–8 — investigations and helpers

1. Port investigation admission, queue/dispatch, investigator startup, stdin/team behaviour, result files,
   publication retries, resumption and supervision in size-limited sub-rounds.
2. Port local and remote helper preparation/finish flows and their bus interactions. Preserve agentbus
   subcommand output parsing and exit-code cases exactly.
3. Checks: the relevant `investigation_team_*.bats`, `investigation_mcp_off.bats`, helper hook and remote
   helper scenarios, reboot/recovery regressions; no real investigator; full gate after every sub-round.

### Round 9 — teams and coordination

1. Port team creation, member lifecycle, checkpoints, consultations, pair/review coordination and close.
2. Preserve bus event payloads, member states, wake mechanics, Git identities and report/commit trailers.
3. Checks: `coordination.bats`, `team_close.bats`, `member_out.bats`, `rotation.bats`, `findings.bats`,
   `review_pair_random.bats`, team resume scenarios; full gate.

### Round 10 — verify, review and autopilot

1. Port review pairing, investigator review requests, findings/fix passes, auto-stage transitions, question
   handling, PR creation and merge gates in separate sub-rounds if needed.
2. Preserve retry/backoff, one-PR-per-round, round reports, failed handover and resume boundaries. Keep all
   GitHub traffic on stubs.
3. Checks: `verify_loop.bats`, `auto_stage.bats`, `autopilot_*.bats`, `review_resilience.bats`,
   `handover_failed.bats`, `one_pr_per_round.bats`, `round_reports.bats`; full gate.

### Round 11 — archive, display and UI entry points

1. Port `archive.go` with exact `7z` file lists, exclusions, output paths and failure handling.
2. Port list/status/log rendering and the `live`/`web` exec handoff, including arguments consumed by
   `agentlive`. Check that existing metadata and activity fixtures render identically.
3. Checks: archive scenarios, `web.bats`, `web_jobs.bats`, output golden tests and installed-path exec tests;
   full gate.

### Round 12 — lead commands

1. Port lead CLI validation, context, installation-related command behaviour and job attribution before the
   node daemon. Keep HTTP and JSON wire types separate from local task types.
2. Checks: `lead.bats`, `lead_cli*.bats`, `lead_install.bats`, `job_attribution*.bats`, `delegate_jobs.bats`;
   full gate.

### Rounds 13–14 — nodes and remote synchronization

1. Port node installation/checkout, agent selection, daemon locking, heartbeat validation and control
   ordering. Use a local HTTP test server; no external node.
2. Port assignment preparation, cancellation/recovery fencing, large payloads, state persistence, sync
   batching/truncation, reviews and remote helpers. Split by ledger size, not by arbitrary file count.
3. Checks: all `node_*.bats`, `remote_control_*.bats`, `remote_fixes.bats`, `remote_proxy.bats`, large-job and
   two-HOME scenarios; `go test -race`; repeated local runs for concurrency confidence; full gate.

### Round 15 — full parity and approved white-box conversion

1. Confirm all 369 process-level Bats tests pass unchanged against `build/agent` and all 462 ledger rows have
   equivalent Go coverage. There must be no unexplained legacy-only behaviour.
2. Present the 93-test conversion list to the human and obtain the explicit exception required to modify
   existing tests. Without it, stop here.
3. In a test-only round, convert those tests as described above while Bash is still available for oracle
   comparisons. Retain test names and every assertion; do not silently delete the old Bash test before its
   Go equivalent is green.
4. Run the entire shadow Bats suite against Go, the complete Go suite including race checks, and the legacy
   suite where it still applies. Review the ledger line by line.

### Round 16 — cutover, installation and rollback

1. Remove the Bash implementation only after Round 15 is green. Build the Go program as an ignored generated
   `bin/agent` for the unchanged process-level Bats helper, and as `build/agent` for normal development.
   Never commit a compiled binary. Another layout is acceptable only if it does not require unapproved test
   helper changes or leave a Bash compatibility implementation.
2. Update `Makefile`: add the `agent` module to build/vet/test/lint/doccheck/generation, build `bin/agent`
   before Bats, and remove `bin/agent` from Bash syntax/shellcheck inputs. Update `install.sh` to build and
   install the Go executable atomically with the other binaries.
3. Update README and configuration/installation documentation: Go architecture, build requirements, the
   retained Bash configuration evaluator, on-disk compatibility and test commands. Do not change workflow
   files; propose any CI adjustment in the PR description.
4. Test installation only in an isolated temporary HOME/systemd environment. Verify installed `agent`,
   self-invocation, the `gh` symlink, `live`/`web`, old task metadata and archive reading. Never restart or
   reinstall the running installation as part of this migration.
5. Record rollback as installing the previous release's Bash `bin/agent` and matching installer, with no
   automatic runtime fallback. Because all persisted formats remain compatible, tasks written by Go remain
   readable by the previous release; prove this with fixtures before cutover.
6. Final checks: clean build, all 462 Bats scenarios against Go (with only the explicitly approved white-box
   conversions), all Go tests and race checks, legacy-format read/write fixtures, `make gate`, round-size
   check and a reviewed compatibility/PR description.

## Per-round completion checklist

A round is not complete when its targeted test merely passes. It is complete when all of these are true:

- [ ] Tests for the new behaviour were written first and the RED evidence was recorded; no existing test was
      changed without the explicit cutover exception.
- [ ] The ledger maps every newly ported scenario to equivalent assertions; no row is marked done based only
      on a command returning zero.
- [ ] Targeted Go tests and targeted original/shadow Bats tests pass.
- [ ] `cd agent && go test ./...` and the other repository Go tests pass; concurrency rounds also pass
      `-race`.
- [ ] Full `bats tests/unit` passes against the still-installed Bash implementation during development.
- [ ] `make gate` has zero findings and the round is within its line limit.
- [ ] `git diff --exit-code f6a602465a7d30e4b6a6737515e67e056f757a05 -- bin/agent tests/unit` passes until
      the approved cutover.
- [ ] No external dependency, real account, real provider CLI or modification of the running installation
      was introduced.
- [ ] README/configuration/prompts are updated when that round changes a visible interface; persisted-format
      compatibility and any generated error code are tested and documented.
- [ ] The PR/report identifies remaining unported commands and any behaviour whose parity was difficult to
      prove.

## Main risks and how the plan controls them

| Risk | Control |
|---|---|
| White-box tests appear to require preserving Bash forever | Explicit requirements decision and separate, assertion-preserving conversion round |
| Go tests accidentally describe new behaviour instead of current behaviour | Existing Bats is the oracle; run both binaries and keep a one-to-one ledger |
| Partial Bash config parser rejects old installations | Narrow Bash evaluator for the existing config language; golden old-config fixtures |
| Metadata becomes unreadable by `agentlive` or an older release | `%q` differential/golden tests and cross-version read/write fixtures |
| Commands differ only in argv, fd or exit-status edge cases | Stub call logs, exact process harness, self-invocation and descriptor tests |
| Background runners orphan processes or race with stop | Process-group tests, systemd stubs, race detector and recovery/fencing scenarios |
| Security filter accidentally passes untrusted content | Port it early with fail-closed tests and no real GitHub traffic |
| A large round hides regressions | Domain rounds, 3,000-line hard limit, full legacy gate after each sub-round |
| Cutover breaks installed self-invocation | Isolated installation test and explicit previous-release rollback rehearsal |

## Bottom line

The safe migration is a sequence of behaviour-preserving, ledger-backed rounds, not a transliteration of
8,523 lines. Keep the Bats suite as the oracle and reuse all of its stubs. Build Go by domain with typed
errors and explicit process/state boundaries. The only requirements conflict is the 93 tests that directly
source Bash internals: obtain an explicit decision to convert those tests without losing their assertions,
or do not claim that a pure Go cutover is complete.
