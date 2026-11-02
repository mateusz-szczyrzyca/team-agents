# Plan: replace the agent Bash runner with a Go executable

Investigator: inv-astra. Investigation date: 2026-11-02.
Baseline: `f6a602465a7d30e4b6a6737515e67e056f757a05`.
This deliverable is a plan; it does not implement or install the replacement.

**Baseline prerequisite:** the inspected commit does not have a green Bats suite. Two
reproducible investigation-isolation failures conflict with newer MCP configuration tests.
See the findings and question at the end. Resolve this before accepting a frozen migration
oracle; this plan does not authorize fixing, editing, or skipping those tests.

## Recommendation

Build a separate, standard-library-only Go command in `agent/`, organized by responsibility,
and port behavior in small, test-first rounds. Keep the Bash implementation operational
until the Go candidate covers every existing scenario. Keep all existing Bats tests,
helpers, and stubs unchanged. Add Go equivalents; do not replace or delete the Bats suite.

There is an important qualification: the existing suite is not entirely a command-line
test suite. Many tests source Bash functions, mutate shell globals, or override shell
functions. An ELF executable cannot satisfy those Bash interfaces. Therefore use three
explicit verification lanes: the complete unchanged Bash baseline, unchanged applicable
Bats cases executed against Go, and Go equivalents of **every** baseline case, including
the source-dependent cases. Never report a passing legacy lane as proof that Go works.

The final release contains a compiled `bin/agent`, with the installed command still at
`~/.local/bin/agent`. No command, runner, daemon, or unsupported operation falls back to
the old Bash orchestration. A narrowly scoped Bash configuration reader may remain to
preserve existing executable `agents.conf` and `secrets.env` files. Removing support for
that syntax is a separate compatibility decision, not a prerequisite for a Go runner.

## What was inspected

All paths below refer to the baseline above. There are no nested `AGENTS.md` files in this
checkout. Read [AGENTS.md](../../../AGENTS.md), [README](../../../README.md), and
[the TDD rules](../../tdd.md) before implementing a round.

| Evidence | Consequence for the rewrite |
| --- | --- |
| [bin/agent](../../../bin/agent): 8,523 lines, 316 function definitions | Splitting only the command dispatcher would leave most complexity in one file. Port supporting state machines and process boundaries too. |
| `tests/unit`: 110 `.bats` files, 462 `@test` declarations | Preserve a case-level coverage ledger, not just a list of files or a Go coverage percentage. |
| 44 Bats files contain direct sourcing of an `agent` path | Some files mix command tests and function tests. Classify each case and its setup dependencies. |
| [helpers.bash](../../../tests/unit/helpers.bash): setup copies `$REPO_ROOT/bin/agent` into a temporary home | Merely changing `PATH`, setting a new `AGENT_BIN`, or building `build/agent` will not make these tests execute Go. No general candidate-binary override exists. |
| [node_agents.bats](../../../tests/unit/node_agents.bats), [node_report_agent.bats](../../../tests/unit/node_report_agent.bats) extract a function with `sed` | A shell wrapper which only `exec`s Go cannot preserve these tests. |
| [node_checkout.bats](../../../tests/unit/node_checkout.bats) overrides `printf` and `grep`; [node_recovery_fence.bats](../../../tests/unit/node_recovery_fence.bats) overrides launch/acknowledgement functions | A subprocess bridge would lose the fault injection. Equivalent Go tests need explicit injected boundaries. |
| [node_safety.bats](../../../tests/unit/node_safety.bats) checks a shell function exists and explicitly releases descriptor 8's lock | Observable shell execution context is part of these historical tests; do not imitate it with production Go debug commands. |
| [Makefile](../../../Makefile): explicit module, test, shell syntax, and doccheck lists | Adding `agent/go.mod` alone does not add its tests or documentation checks to the gate. |
| [install.sh](../../../install.sh): copies the script, separately builds agentbus/live | Installation and artifact creation need their own migration round. Never run this installer against the working agent's home. |
| [lead_install.bats](../../../tests/unit/lead_install.bats), [node_install.bats](../../../tests/unit/node_install.bats) stub `go` with a program that creates no output | A new installer that builds a temporary executable then renames it cannot be verified by these tests without a separate candidate installer harness. Preserve the old tests as baseline checks. |
| `agentbus` and `live` are separate `package main` modules, both declaring Go 1.22 | Do not import them as libraries or combine all modules as part of this rewrite. Keep their process interfaces. |
| `bin/agent` dispatches on executable name `gh` before loading normal configuration | The trust-filtering `gh` symlink is another entry point, not an optional command. |

These are local repository findings, not assumptions about current upstream model CLIs.
Port the CLI invocations tested here first; an upstream API/model upgrade belongs in a
different change.

## Preserve tests without pretending Bash source can be Go

### Verification lanes

1. **Legacy baseline:** execute all 462 original cases with the original Bash code and
   original fixture environment. Initially this is ordinary `bats tests/unit`. When the
   executable path changes, run the same bytes in a disposable export of the pinned
   baseline repository. This includes the old installer/Makefile for tests which inspect
   them. Log the baseline commit, test count, and that this lane runs Bash.
2. **Candidate Bats contracts:** create a disposable export of the current candidate tree,
   put the compiled Go binary at that export's `bin/agent`, and run the cases that use
   compatible command/file interfaces. Copy test files, helpers, and stubs byte-for-byte.
   Their computed `REPO_ROOT` then points at this export, including secondary-home copies.
   Select cases explicitly from the ledger, not by guessing from a filename. This is a
   growing candidate subset; it never replaces the complete baseline run.
3. **Go parity:** add native Go tests for every original case, preserving its setup,
   trigger, assertions, failure boundaries, and cleanup. Pure behavior uses unit tests;
   CLI, daemon, process, installation, and recovery behavior also needs subprocess tests.
   Running Bats from a Go test is not a native Go equivalent.

Add a staging/verification tool under `tools/`, with new tests of its own. It must use
temporary directories outside the live installation, reject missing baselines/candidates,
preserve executable modes and relative fixture paths, and clean up children and exports.
The candidate export must contain the code under test, including current staged/unstaged
changes and newly added test files; an export of an older `HEAD` is not sufficient during
RED/GREEN development. Test that a changed working-tree fixture reaches the candidate
export. Record the candidate binary hash and verify every temporary-home installation,
including node homes, uses that artifact for the selected cases.
Do not override `source`, `sed`, `bash`, or `install`, rewrite test text, change existing
stub behavior, or add a Bats-only production dispatcher. Do not leave a sourceable wrapper
at the final executable path and call that a Go binary.

Store a manifest of the original files' Git blob IDs or SHA-256 values. Verify both the
tracked originals and staged copies. Track every original case; additive tests get new
entries. A filtered candidate lane must print its selected case count and the remaining
source-coupled cases. No `skip`, hidden passing placeholder, or automatic fallback to Bash.

For the final baseline export, `git archive` from the immutable commit avoids a second
checked-in copy of the entire test suite. The commit object must be available offline:
check it with `git cat-file -e` and fail with a clear setup instruction if missing. Document
the developer checkout requirement; the gate must not silently fetch or access another
agent's checkout. An operator-supplied, checksummed baseline archive is another possible
offline input. A production installation does not need this historical source.
Here the inspected commit is evidence, not yet an acceptable green oracle: round 0 must
pin a reviewed, green successor after the baseline conflict is resolved. Record that
successor explicitly and retain the original findings; never silently substitute a base.

Before cutover, the ordinary gate still runs the unchanged suite at the original path.
At cutover, explicitly wire `unit-bats` to run the complete historical lane **and** the
candidate contracts. Keep new current-tree Bats tests discoverable as well; freezing the
original baseline must not silently omit tests added later. `unit-go` must run Go parity.
Thus `make gate` continues to mean the complete verification set, with labeled results.

### Meaning of the requirement

This design keeps all original Bats checks executable and unchanged, while the equivalent
Go tests and applicable original Bats contracts verify the final Go implementation.
It does **not** claim every original Bats case can directly execute a Go binary.

If the intended acceptance criterion instead demands *all original cases directly target
Go through their existing source/function interfaces*, that criterion conflicts with the
requested binary and unchanged-test rule. Record a question before final cutover: may
only those test seams be adapted after parity is demonstrated? The recommendation is to
retain the three lanes above, requiring no existing-test edits. Do not assume permission
to change tests from permission to rewrite production code. This decision does not block
the independent implementation rounds or this planning deliverable.

## Target source design

Use `agent/go.mod` with module name `agentcli`, the repository's supported Go version, and
no third-party requirements. Start with one `package main`, split into domain files;
this matches the existing modules and lets tests call internal functions. A new local
module is a build boundary, not a reason to add Cobra, a shell parser, an SDK, or a system
package. Any new external dependency requires separate human approval.

| Files under `agent/` | Responsibility |
| --- | --- |
| `main.go`, `cli.go` | Small entry point, executable-name dispatch, command/flag parsing, output and exit policy |
| `app.go`, `config.go`, `config_shell.go` | Explicit application dependencies, paths, configuration/defaults, bounded legacy syntax adapter |
| `types.go`, `errors.go`, `errors.json`, `errors_gen.go`, `logging.go` | Typed statuses/kinds/modes/events; operation-specific errors; stable structured logs |
| `process.go`, `process_linux.go`, `systemd.go`, `bus.go` | External argv/environment, process ownership, signals, units, typed agentbus command results |
| `storage.go`, `lock_linux.go`, `metadata.go` | Atomic files, flock-compatible locks, lossless old metadata, unknown-field preservation |
| `git.go`, `identity.go`, `task.go`, `allocation.go` | Git/worktrees, commit identity, task creation, collision-safe cohort admission and rollback |
| `run.go`, `resume.go`, `runner.go`, `runner_finish.go`, `supervise.go` | User start/resume, runner lifecycle, completion/recovery and dead-run detection |
| `provider_claude.go`, `provider_codex.go`, `provider_gemini.go`, `provider_opencode.go` | Existing provider argv, session/MCP config, prompt delivery and result extraction |
| `prompt.go`, `questions.go`, `notify.go` | Prompt rendering, questions/answers and notifications |
| `watchdog.go`, `limits.go`, `wake.go`, `session.go`, `tokens.go` | Hang/step limits, quota/network classification, pause/wake, session refresh, usage |
| `team.go`, `helpers.go`, `investigation.go` | Team membership, delegation queue, legacy investigation queue |
| `investigation_team.go`, `investigation_phase.go`, `investigation_resume.go` | Independent investigation admission, publication, phase transitions and recovery |
| `round.go`, `review.go`, `autopilot.go` | Round accounting/handoffs, reviews, precheck/fix/merge state machines |
| `github.go`, `github_filter.go` | GitHub trust policy, special `gh` entry point, bylines and review formatting |
| `lead.go`, `jobs.go`, `node.go`, `node_state.go`, `node_launch.go` | Lead wrappers, remote job commands, node polling/acknowledgement/launch/recovery |
| `node_sync.go`, `remote_controls.go`, `remote_questions.go` | Bounded transfer, durable control acknowledgements, remote answers and notices |
| `status.go`, `transcript.go`, `archive.go`, `authenticated.go`, `ui.go` | Inspection commands, transcripts, archival, authentication probes, live/web forwarding |

Add `*_test.go` beside the implementation, with historical fixtures in `agent/testdata/`.
Split files further when their responsibilities diverge. Do not pre-create empty files or
write all future tests in the first round. Keep the public dispatcher small; a giant
`helpers.go` that merely reproduces the script's globals is not this design.

Use an application instance with explicit configuration, clock, random source, command
runner, task store, logger, and notification dependencies. Tests substitute dependencies
per instance; avoid package-global mutable fakes and process-wide `os.Chdir`/environment
changes during concurrent tests. Request/result structs replace shell output globals such
as `NEW_ID`, `RC`, `FINAL`, `FRESH_WHY`, and `NODE_STATE`.

Keep operation-specific error types and causes. `tools/errgen` already accepts `-spec` and
`-out` and emits `package main`; reuse it with an agent-local specification and compatible
error support, without altering `agentbus/errors.json` for unrelated CLI operations.
Extend generation/freshness checks for the new specification. Convert standard-library
errors immediately at boundaries. A generic `error` return for every service would violate
repository conventions. Map causes to the **existing agent CLI exit codes**, not blindly
to agentbus's different exit-code policy. Document declarations as required by doccheck.

## Compatibility contracts to write down before porting

1. **Entry points:** inventory all arms of `main`, aliases `ls`, `help`, `-h`, `--help`,
   unknown/missing arguments, and `gh` basename behavior. Internal commands are interfaces:
   `_runner`, `_wake`, `_supervise`, `_agents-json`, `_dispatch-helpers`, `_remote_member`,
   `_precheck`, `_merge`, `_fixpass`, `_answers_saved`, `_remote_questions`,
   `_remote_questions_ready`, `_authcheck`, and `_investigation-team` are called elsewhere.
   Preserve exact argv, stdout/stderr separation, successful no-ops and failure statuses.
   Go's default flag parser alone does not establish compatibility with the existing
   positional/option ordering; write parsing cases first.
2. **Configuration:** `agents.conf` is executable Bash with associative arrays, ordered
   lists, expansions and legacy investigator fields. Preserve source order, environment
   precedence, empty versus unset semantics (`:-` versus `-`), exported custom variables,
   PATH, locale, defaults and extra-argument splitting. `AGENT_EXTRA_ARGS` currently uses
   Bash `read -ra`; treating it as shell-quoted command syntax would change behavior.
3. **Legacy shell adapter:** execute trusted configuration in a fixed Bash program with
   paths passed as argv, never interpolated shell code. Return an explicit typed map/list
   encoding over a dedicated pipe (NUL framing is suitable for shell strings). Keep config
   stdout separate from serialization. Load secrets only in the runner/auth contexts that
   load them now, preserve environment exports/unsets, and never log the serialized secret
   channel. Include computed values and legacy defaults in tests. This is configuration
   compatibility, not `bash bin/agent "$@"` hidden inside Go.
4. **Metadata:** `meta.env` is shell data written with Bash `%q`, then sourced by multiple
   components; it is not dotenv. Preserve escaped spaces, backslashes, quotes, dollar
   signs, multiline and non-ASCII text, empty values and unknown keys. Preserve omission
   defaults and previously accepted values. Add fixtures written by the baseline, and
   test old-to-new reads and new-to-old reads under both ordinary and `C.UTF-8` locales.
   Start with a narrowly scoped Bash codec if needed; only replace it with a non-executing
   parser after proving grammar coverage. Do not silently reject historical metadata or
   switch it to JSON. `node-jobs.json`, transitions and sync manifests also need old fixtures.
5. **Locks:** inventory lock paths, shared/exclusive mode, ownership, deadlines, ordering,
   and child descriptor closure. In particular: `meta.env.lock`, `task-ids.lock`,
   `wake.lock`, answer/ready locks, `dispatch.lock`, `helpers.lock`, review pair locks,
   investigation locks, and `node-daemon.lock`. A Go mutex cannot replace a process lock.
   Linux flock support can use the standard library's OS interface, isolated in a Linux
   file; test with real independent Bash/flock and Go processes. Preserve nonblocking
   cases and successful contention no-ops. Never hold unrelated locks across child waits.
6. **Atomic writes:** retain read-modify-write serialization, stale-run guards, sibling
   temporary files and atomic replacement. Preserve unknown fields, modes and state on
   read/write/rename failure. Keep job transition recording consistent with status
   updates. Go goroutines must not share the script's PID-only temporary filename scheme.
7. **Processes:** carry argv, working directory, inherited versus explicit environment,
   stdin behavior, process session/group isolation, logs and exit codes. Capture provider
   exit separately from normalizer/tee failures, drain streams and wait for all children.
   Long JSON lines and prompts must not fail at an implicit scanner/argument limit.
   Cancellation must terminate the correct descendants and not kill the runner or peers.
   Close admission/metadata locks before spawning reentrant bus calls or runners.
8. **Runner completion:** preserve status parsing, missing-status questions, final message
   files, notifications and team/helper/investigator delivery. Go `os.Exit` bypasses
   deferred cleanup: return through finalization before exiting. Use one finalization
   owner and run/assignment fences to prevent old callbacks changing newer work. Reboot
   and SIGKILL recovery belong to supervision, not an assumed deferred handler.
9. **Git:** retain task/branch naming, base immutability, worktree location, collision
   admission, retained branches, resume chains, dirty work and rollback behavior. Tests
   must isolate Git configuration/signing and use local bare remotes. Never inspect or
   reset unrelated worktrees, including those of other investigators.
10. **Bus/live boundaries:** keep agentbus subprocess commands, output/exit semantics,
    normalizer records and `--agent` callback paths. Do not write the bus state directly
    from Go agent or reimplement MCP/HTTP protocols already owned by agentbus. Preserve
    `live`/`web` process replacement or equivalent signal/exit propagation.
11. **Remote workflows:** preserve assignment generations, prepare-before-start-ACK,
    cancel-before-launch fences, boot recovery, durable outbox/retries, control ordering
    and stale-round answers. Preserve transfer caps, clipping, hashes, cursor advancement,
    mtime freshness, path allowlists and exact JSON field capitalization.
12. **Data products:** preserve structured event names and field types, journal/file
    destinations, activity and full-activity logs, transcripts, checkpoint and notification
    paths, summary throttling and archive layout. An archive may delete tasks only after
    packing and validation succeed. Keep `7z`/`7za`/`7zr` compatibility; do not invent a Go
    archive format to avoid an existing dependency.
13. **Trust/authentication:** preserve the `gh` filtering and bypass refusals before normal
    config loading, review bylines and real-command exit forwarding. Authentication tests
    use stubs or loopback HTTP only, never the investigator's accounts or real model CLIs.
    Preserve per-agent model probes and provider error classification. Keep seed-based
    investigation selection reproducible according to its existing algorithm; an arbitrary
    Go PRNG with the same numeric seed does not establish parity.

## How to translate the tests

Create `docs/migrations/agent-go-cases.md` (or a machine-readable ledger with a generated
Markdown view) in the first implementation round. Each row needs:

- Stable baseline identity: file and exact `@test` title, plus the pinned commit.
- Setup/helper dependencies and classification: command contract, source-coupled unit,
  source-extraction test, installer/build contract, or mixed process scenario.
- Observable assertions and injected failures; owning Go file and planned round.
- Go test/subtest name, RED evidence, GREEN evidence and candidate Bats eligibility.
- Progress: inventoried, baseline verified, Go test red, Go passing, candidate verified.

Every baseline case needs a Go equivalent by cutover, including tests about build scripts
and installer unit contents. One case may become several Go tests. A table-driven Go test
may cover several cases only if each maps to a named subtest with all its assertions.
Do not replace detailed safety assertions with a generic `exit == 0` check. Keep a ledger
checker that fails on missing/duplicate mappings, vanished baseline files, stale test
names, or unimplemented entries at release. Record later additive cases too.

For a round:

1. Read each selected Bats body, setup and relevant stub implementation. Reproduce its
   failure injection in Go before writing production behavior. Record which assertions
   concern external contracts and which merely use a Bash-specific testing seam.
2. Write compiling zero-value signatures and the new tests. Run the new test and confirm
   an assertion fails for missing behavior. Run `make gate`; only these new tests may
   fail. Commit RED with the command/output, as required by `docs/tdd.md`.
3. Implement the smallest behavior; run the new tests and focused original Bats cases.
   For supported command contracts, run the staged Go lane too. Require it to fail if
   the candidate is removed; test this property in the staging tool's own tests.
4. Run the full gate and commit GREEN. Refactor without changing those tests, run relevant
   concurrency checks, and update the ledger and documentation. Never push an unfinished
   RED branch as a completed implementation round.

Examples of faithful translations:

| Original scenario | Required Go verification |
| --- | --- |
| `meta.bats`: concurrent writers preserve all keys | Independent subprocess writers against one temporary file; all keys and original status survive. Also mix a Bash writer with Go while compatibility is required. A goroutine-only test is insufficient. |
| `node_checkout.bats`: metadata output/read fails | Inject writer/read failure; assert typed failure, original bytes unchanged, no partially created task/branch/worktree. Do not keep calling `grep` solely to retain its test seam. |
| `node_agents.bats`: all agents inventoried, only available ones free | Fake availability/model/config dependencies; compare the complete JSON inventories and missing-CLI event, including ordered contents expected by callers. |
| `node_report_agent.bats`: chosen agent survives completion | Apply the two transitions; assert agent identity in both reports and both persisted transition records. |
| `agent.bats`: watchdog kills a hung session and resumes | A real stub subprocess plus controlled watchdog input; assert termination, resumed session and stored state. Use a fake clock for policy unit tests and a bounded real-process test for cleanup. |
| `agent.bats`: archive removes finished tasks | Create temporary tasks/worktrees/checkpoints; use the available 7z tool; inspect archive contents and retained branches. Add failed-pack/failed-validation tests before porting destructive cleanup. |
| `resume_alloc.bats`: concurrent resumes avoid collisions | Concurrent independent CLI processes with a fixed timestamp source; preserve predecessors and claim unique task, worktree, branch and team IDs. |
| `node_recovery_fence.bats`: stop arrives during start ACK | Inject the stop at the same transition boundary; prove no launch, no unexpected run increment, and durable stopped/recovered state. |
| Installer tests with a no-output `go` stub | New candidate harness supplies controlled build artifacts and failing-build cases; checks unit text, preservation of config/secrets, real binary installation and cleanup on build failure. Keep original installer tests in the baseline lane. |

Build the candidate once per Go subprocess suite, using existing test facilities or a
new test helper process. Use `t.TempDir`, per-child environment, injected clocks/random
sources, and `t.Cleanup`. Test subprocess helpers must fail closed for any unconfigured
real CLI/systemd/GitHub invocation. Reuse existing stub files unchanged where applicable;
add new fakes only for new test seams. Loopback servers and local Git remotes are allowed.
Poll an observable condition with a deadline; use no bare sleeps as synchronization.
On timeout, include bounded logs, terminate child groups and wait for them.

## Ordered implementation rounds

The following are dependency-ordered work packages, not permission to put each package
in one large PR. Split each into subrounds at roughly 2,000 changed lines, hard maximum
3,000 including tests. Typical units are one provider, one state machine transition family,
or one sync/control family. Run `agent round-size <task>` for each actual round. Main must
pass the full gate after every GREEN round; unfinished candidate code stays unreachable
from the installed command. Do not add future skipped tests.

### 0. Freeze the baseline and specify acceptance

- Recount commands, configuration variables, persisted files and all 462 cases from the
  chosen base. Record hashes, setup dependencies and source-coupling in the ledger.
- Run the untouched gate. Capture tool versions and any pre-existing failures without
  changing tests. Resolve a failing baseline before treating later differences as regressions.
- Add the ledger/staging design and README development instructions. State the three lanes
  and the exact future executable path. No runtime or installer change in this round.
- Check: full `make gate`, original test hashes, and review of every dispatcher arm.

### 1. Add the Go command and verification plumbing

- Add `agent/go.mod`, minimal entry/dispatch, typed error foundations, application
  dependencies and a safe test environment. Begin with help/unknown-command behavior,
  including the existing configuration prerequisite; do not expose unimplemented commands.
- Build to `build/agent-go`. Add `agent` to `GO_MODULES`, explicit `unit-go`, `build`,
  `lint-docs` inputs, generation and `check-generated`. Merely extending `GO_MODULES`
  does not change `unit-go` or doccheck in this repository.
- Add the staging tool and tests that prove binary selection, immutable test copies,
  failure propagation, cleanup and secondary-home routing. Proposed target names below
  become real here; leave normal `unit-bats` pointing at Bash until cutover.
- Check: Go tests, generation freshness, doccheck/lint and the unchanged full gate.

### 2. Port configuration and logging

- Add shell-config decoding and defaults with no provider invocations; test arrays,
  legacy investigator settings, empty values, expansions, exports, paths with whitespace,
  malformed input and both supported config/secrets contexts.
- Port JSON log shape, stable event constants, file/journal/runner destinations and
  side-effect logging. Preserve human stdout separately from diagnostics.
- Check: new config/log tests and relevant assertions from `agent.bats`, `identity.bats`,
  `run_notes.bats`, `pause_notify.bats`; gate. Configuration secret data must not appear in logs.

### 3. Port storage, metadata and process locks

- Add historical metadata/node-state fixtures, atomic replacement, typed failures, and
  old/new codec round trips. Preserve unknown keys and missing legacy fields.
- Implement real process locks with explicit ownership and deadline behavior; test
  concurrent writes, stale-run no-ops, reader/write/rename faults and descriptor inheritance.
- Check: Go subprocess tests, `go test -race`, `meta.bats` and storage/fencing cases from
  `node_checkout.bats`, `node_safety.bats`, `remote_control_kick_exit.bats`; full gate.

### 4. Port process, systemd, bus and Git boundaries

- Add argv/env/result types, streaming execution, signal/exit forwarding and tested
  cleanup. Keep systemd-run/systemctl and agentbus as subprocesses, using current argv
  conventions understood by the existing stubs.
- Port Git identity and worktree operations with temporary local repositories; exercise
  partial failure and ownership-aware rollback. Port live/web/lead forwarding separately.
- Check: process-tree tests, long stream/normalizer failure tests, `identity.bats`,
  `lead_cli.bats`, `lead_cli_context.bats`, `web.bats`, and relevant Go agentbus/live tests.

### 5. Port inspection commands and authentication

- Implement status/show/agents/ack, bus/inv/helpers/notes, nodes/jobs inspection,
  `_agents-json`, transcript/log rendering, and token calculations in small subrounds.
- Implement `authenticated` and `_authcheck` using controlled processes/loopback APIs;
  test per-agent models, provider mode, unknown agents and error bodies/status codes.
- Check: `authenticated.bats`, `tokens.bats`, `run_notes.bats`, relevant `agent.bats`
  inspection cases, plus candidate contract runs for each completed command. No real probes.

### 6. Port admission, task creation and resume preparation

- Port validation before side effects, ID/cohort allocation, task/worktree creation,
  job markers and rollback. Then solo/team resume chains, dirty changes/stashes and
  same-round/new-round decisions. Test rejected requests leave no new artifacts.
- Keep actual runner launch behind an injected boundary until round 9 completes.
- Check: `resume_alloc.bats`, `resume_stash.bats`, `resume_members.bats`, `resilience.bats`,
  `node_checkout.bats`, `round_only.bats`; independent-process collision tests and gate.

### 7. Port prompts, questions, identity and notifications

- Port placeholder substitution, repository/environment instructions, language settings,
  commit trailers, checkpoint selection and question validation. Preserve all current
  question labels, ANSWERED behavior and editor semantics.
- Port notification files, command/legacy-script behavior, failures and summaries. Keep
  preparation separate from actual launch so failures cannot strand partial tasks.
- Check: `prompt_file.bats`, `prompt_checkpoints.bats`, `context_hygiene.bats`,
  `identity.bats`, question/notification cases in `agent.bats`, `pause_notify.bats`.
  Render every prompt kind with no unresolved `{{...}}`; gate.

### 8. Port provider adapters one at a time

- Implement Claude, Codex, Gemini/Antigravity and OpenCode invocation/result handling as
  four subrounds. Preserve model/extra args, session creation/resume, prompt-file fallback,
  stdin closure, output files and agentbus normalization.
- Preserve independent investigation MCP isolation and exact per-provider configuration.
  Test no hidden interactive stdin wait, no installed personal MCP fallback and large prompts.
- Check: new provider argv/stream tests plus `prompt_file.bats`, `investigation_mcp_off.bats`,
  `investigation_team_isolation.bats`, `investigation_team_opencode_schema.bats`,
  `investigation_team_stdin.bats`; gate. Only existing/new fake CLIs execute.

### 9. Port the solo runner and recovery lifecycle

- Connect admission, prompt, adapter and finalization for solo runs. Port status results,
  missing-status questions, pause/quota/network/local-resource classification, fresh
  sessions, hang strikes and step limits. Split these policies into separate subrounds.
- Implement stop/kick/wake, answer resume, supervisor/reboot recovery and failed starts.
  Explicitly model run-generation fences, exactly-once local finalization and retryable
  external delivery; do not promise crash-proof exactly-once external side effects.
- Check: solo cases in `agent.bats`, `limits.bats`, `network.bats`, `resilience.bats`,
  `restart.bats`, `kick.bats`, `wake_cmd.bats`, `fresh_session.bats`, `fresh_relative.bats`,
  `long_run_cut.bats`, `handover_failed.bats`; candidate Bats contracts and race tests.

### 10. Port teams, helpers and ordinary investigations

- Port creation/coordinator/member completion, sleeping/wake behavior, member exclusion,
  team close, checkpoint handoff and verify gaps. Keep bus-owned state changes in agentbus.
- Port investigator rotation/provider preference, capacity, queue admission, helper
  balancing, requester delivery and resuming after results. Test concurrent dispatch and
  that waiting/paused agents keep the intended reservations.
- Check: `team_close.bats`, `member_out.bats`, `rotation.bats`, `coordination.bats`,
  `verify_loop.bats`, `delegate_jobs.bats`, team/helper cases in `agent.bats`; full gate.

### 11. Port independent investigation teams

- Separate subrounds for CLI validation/isolation, phase-1 admission/supervision,
  publication and retry, merger/reviewer selection, phase queue/handoff, then resume and
  reboot recovery. Persist the same boundaries before external side effects.
- Preserve immutable bases, no-bus/MCP isolation, publication readiness, availability
  fences, seeded selection and advisory resume. The current investigation runs through
  this machinery; exercise only synthetic temporary homes.
- Check: every `investigation_team_*.bats` case and `investigation_mcp_off.bats`, with
  explicit Go mappings for all source-coupled regressions; candidate lane and gate.

### 12. Port rounds, reviews, GitHub trust and autopilot

- First port the `gh` executable-name path and trust filters, bylines and output formatting.
  Then port round reports/size/test checks, PR reuse/supersession, review pair admission,
  precheck/fix passes, merge blockers, human questions and next-round handoff.
- Keep all GitHub interactions stubbed. Check trusted author/fork/GraphQL restrictions,
  merge gates, test-change holds, random/list review selection and recovery after partial
  delivery. Preserve refusal conditions rather than simplifying them into generic retries.
- Check: `agent_gh.bats`, `github_trust.bats`, `github_format.bats`, `review_byline.bats`,
  `findings.bats`, `skips.bats`, `gate.bats`, `round*.bats`, `one_pr_per_round.bats`,
  `review_*.bats`, `autopilot_*.bats`, `auto_stage.bats`; gate and candidate contracts.

### 13. Port remote job admission and node lifecycle

- Port lead/jobs/nodes commands, member proxies, node inventory/capabilities, repository
  allowlists, persistent node state and daemon lifetime locking.
- Split preparation, start ACK, final launch fence, cancellation, reports/outbox,
  heartbeat reconciliation and boot recovery into independently tested transitions.
  Preserve assignment numbers, task IDs and actual selected-agent identity end to end.
- Check: `lead*.bats`, `job_attribution*.bats`, `node_agents.bats`, `node_report_agent.bats`,
  `node_daemon.bats`, `node_checkout.bats`, `node_safety.bats`, `node_prelaunch_cancel.bats`,
  `node_cancel_exit.bats`, `node_recovery_fence.bats`, `node_two_homes.bats`,
  `node_large_assignment.bats`, `node_helper_hooks.bats`, `node_review_boundaries.bats`.
  Installer cases remain baseline-only until their Go equivalents are added in round 17.

### 14. Port remote synchronization and controls

- Port task activity/state synchronization, tail clipping, byte limits/hashes,
  unchanged/terminal suppression, freshness, summary/metadata allowlists and checkpoint
  transfer. Use fixtures for missing fields from older nodes.
- Port remote stop/kick/pause/answers/reviews and durable acknowledgements, retry after
  failures, current-round validation, ordering relative to cancellation, and proxy notices.
- Check: all `node_sync*.bats`, `node_activity.bats`, `remote_*.bats`, `web_jobs.bats`,
  plus corresponding agentbus/live Go tests. Run two disposable homes and loopback servers;
  verify stale controls cannot mutate a newer run/assignment and failed transfers retry.

### 15. Port archive and destructive cleanup

- Port complete group discovery (team, resume chain, helper/investigation parentage),
  active-state refusal, transcript/checkpoint staging, archive name collisions, pack/test,
  then task/worktree/checkpoint cleanup. Preserve branches as the current commands do.
- Port clean/discard and stop-all interactions only with synthetic tasks. Test failures
  before each destructive boundary and confirm recoverable data survives.
- Check: archive/discard/stop-all cases in `agent.bats`, additional group/failure tests,
  local archive extraction and full gate. No operations on this installation's tasks.

### 16. Close all parity gaps and rehearse mixed-version recovery

- Audit all dispatcher arms, config keys, files and 462 ledger entries. Add any remaining
  Go tests before filling missing behavior; no unclassified cases or release placeholders.
- Exercise old-produced task/config/node-state fixtures with Go and Go-produced data with
  the old readers. Test a stopped/interrupted old run resumed or supervised by Go in an
  isolated home. Verify old scheduled callbacks still resolve their internal commands.
- Run the full candidate contract subset, all Go parity tests, and the unchanged baseline.
  Compare stable output and persistent state; normalize only declared volatile fields
  such as temporary root, generated ID and time. Never normalize status, error code,
  trust decisions, missing records or lock behavior to hide differences.
- Check: complete ledger, old/new fixture compatibility, race/process tests and gate.

### 17. Switch build/install paths and finish

- Only after round 16 is green, make the release's `bin/agent` a compiled Go executable.
  In the source tree, move the old script unchanged to `tools/legacy/agent.bash` if retaining
  a readable oracle is useful, then ignore the generated `/bin/agent` artifact. Build
  into a temporary sibling and rename; never commit a platform-specific executable.
  The release build and install must require all candidate commands, without fallback.
- Use a pure rename for the old 8,523-line file and verify round-size recognition with
  `git diff -M --numstat`. Do not delete/copy it as an 8,523-line cleanup change or add an
  ad hoc round-size exclusion. If the size checker counts a large change, split/resolve it
  before proceeding; the hard limit still applies.
- Update `install.sh` to compile/install Go, preserve existing config/secrets/prompts policy,
  use the unchanged installed callback path and avoid truncating an executing binary.
  Add isolated native Go installer tests, including build failure, replacement failure,
  preserved configuration and node/lead unit contents. Do not invoke the installer here.
- Remove binary paths from Bash syntax/ShellCheck inputs; retain checks of every remaining
  shell script, including the optional legacy oracle. Include new agent Go sources/tests
  in all gates. Wire the final baseline/candidate Bats lanes as described above and verify
  `lint_shell_memory.bats` behavior through its Go equivalent against the current Makefile.
- Update README requirements, build/test commands, files, development lanes, configuration
  compatibility and rollback instructions. Keep config templates in sync. Propose any
  CI changes in the implementation PR; do not edit `.github/workflows/`. Propose updates
  to the repository's Bash-specific instructions without changing protected instruction files.
- Check: clean-checkout build, real artifact signature, isolated installation, complete
  parity/contract/baseline gates, and no production dependency on the legacy script.
  A real deployment/restart is the operator's later action, not part of an agent's test run.

An isolated installation test must override `HOME`, `AGENTS_HOME`, `XDG_CONFIG_HOME` and
the runtime directory, put fake systemctl/systemd-run and model CLIs first on `PATH`, and
use fresh scratch sources. Checking only a temporary `HOME` is insufficient if an inherited
variable still points at the real installation. Reuse the installer contract assertions,
but add tests of the actual compiled artifact because the historical no-output Go stub
cannot verify that artifact. Never allow the test harness to discover personal credentials.

## Commands and verification schedule

### Available now, at the repository root

```sh
make gate
make unit-bats
bats -f 'watchdog' tests/unit
bats tests/unit/meta.bats tests/unit/resume_alloc.bats
make unit-go
(cd agentbus && go test -run TestLegacyStateLoads ./...)
git diff --exit-code f6a602465a7d30e4b6a6737515e67e056f757a05 -- tests
```

`make gate` runs generated-code checks, shell lint, Go lint, doccheck, Go tests and Bats;
its logs are in `build/gate/`. Read a bounded excerpt (`tail -n 40` or a targeted `rg -n`),
not the whole log. Tests use temporary homes and stubs. Never run real model CLIs,
`make integration`, or live `agent stop`/installer/systemd restart commands for this work.

There is no root Go module: use `(cd agent && go test ./...)` once it exists, not a presumed
root-level `go test ./...`. If a pinned linter is missing, use the prescribed `make tools`;
without sudo, report missing system packages. Do not add another dependency or skip checks.

### Proposed targets to implement in round 1, not commands available today

```sh
make build-agent-go
(cd agent && go vet ./... && go test -count=1 ./...)
(cd agent && go test -race -count=1 ./...)
make check-agent-parity
make unit-bats-agent-go
make gate
```

Define `build-agent-go` to produce `build/agent-go`; define `unit-bats-agent-go` to use the
staged candidate export and ledger-selected contracts. During migration it checks the
completed subset; after round 16 it must require the full compatible subset. Define
`check-agent-parity` to audit unchanged baseline files and mappings, with a strict release
mode requiring all original Go equivalents. Final `make build` produces `bin/agent` as
well as the existing binaries. Final `make gate` invokes the full set automatically.

Use targeted Go/Bats tests while developing a change, then one complete gate before each
commit. Run race tests when implementing concurrency and once for final acceptance;
they supplement cross-process tests, not replace flock tests. Re-run broader tests only
after relevant changes or failures. A RED commit is the documented exception where only
new assertions fail; final round heads and pushed implementation results must be green.

## Release acceptance and rollback

- The artifact at `bin/agent` is a Go-built executable, installed at the original path;
  help, all public/internal commands, callbacks and the `gh` alias select it.
- The ledger accounts for every baseline case with a passing native Go equivalent.
  Applicable original Bats contracts pass against Go, and all original Bats cases still
  run unchanged in the labeled historical lane. No original tests/helpers/stubs were edited.
- No runtime command delegates to legacy Bash orchestration. Configuration/metadata shell
  compatibility, if retained, is named, bounded and independently tested.
- Old files load, newly written files remain readable by supported old components,
  external interfaces keep their statuses and logs, and no uncontrolled live resources
  were used. Required config/template/README changes are present.
- Every required check passes with zero findings, within the round size limit. No new skip
  or suppression is hidden. Generated errors are regenerated, not hand-edited.
- Rollback is an operator-controlled binary/package switch while preserving existing
  data formats. Keep a previous release artifact; do not test rollback against the running
  installation. Validate interrupted-run recovery in temporary homes before recommending it.

## Investigation verification and remaining work

This investigation changes only this Markdown plan. It does not modify production code,
tests, helpers, stubs, configuration, installation, or protected instruction files.
The implementation rounds above remain future work. No hardware was used, no agents were
delegated to, no other worktrees were inspected, and no bus coordination was performed.

Verification performed on 2026-11-02 against the inspected baseline:

| Check | Result |
| --- | --- |
| `make gate`: `check-generated` | PASS |
| `make gate`: `lint-shell` | PASS |
| `make gate`: `lint-go` | PASS |
| `make gate`: `lint-docs` | PASS |
| `make gate`: `unit-go` | PASS |
| `make gate`: `unit-bats` | FAIL: 460 passed, 2 failed, 462 total |
| Focused two-file Bats reproduction below | FAIL: 3 passed, the same 2 failed |
| Original production/test files compared with baseline | Unchanged |
| Staged whitespace check and report-relative links | PASS |

The full gate exited nonzero because of cases 122 and 125, detailed below; both newer
`investigation_mcp_off.bats` cases passed. Full local logs are in `build/gate/`, including
`unit-bats.log` and `investigation-isolation-recheck.log`. The environment provided
Go 1.27.1, Bash 5.3.9, Bats 1.11.0, jq 1.8.1 and Git 2.53.0; no tool installation was needed.
This documentation-only investigation is complete. A green baseline remains a prerequisite
for implementing and accepting the rewrite; this report does not claim a passing gate.

### Reproduced baseline conflicts and question for the human

The working tree's production code and tests are identical to the inspected commit.
The focused command below exits 1 and reproduces both failures:

```sh
bats tests/unit/investigation_team_isolation.bats tests/unit/investigation_team_opencode_schema.bats
```

| Failing baseline assertion | Conflicting current contract |
| --- | --- |
| `investigation_team_isolation.bats:91` requires the independent Codex invocation to contain no `mcp_servers.agentbus.command` | `investigation_mcp_off.bats:8` requires `codex_mcp_args` for an independent investigator to include both its command and `enabled=false`. `run_codex` uses that helper. |
| `investigation_team_opencode_schema.bats:45` requires the disabled object to have only the `enabled` key | `investigation_mcp_off.bats:22` requires the same `opencode_config` output to include `type=local` and a nonempty command as well as `enabled=false`. |

The baseline commit's subject is “An independent investigator's bus server is well formed
and off (green)”; the newer tests describe avoiding a runtime invalid-transport failure.
These are observations of repository history and code, not a verification of upstream CLI
schemas. In particular, changing the Go version to satisfy one OpenCode expectation would
violate the other. Do not make output depend on a test's task name or fixture details.

**Question:** which disabled-MCP contract should be authoritative for the supported CLI
versions, and may the conflicting historical expectation be corrected in a separately
reviewed baseline-repair change if needed? Recommendation: verify the installed/supported
CLI schemas without making paid requests, reconcile the contract in a dedicated change
with explicit human authorization for any existing-test edits, then pin its green commit
for migration. Until then retain every test and report both failures. Under `AGENTS.md`
rule 1 the investigator cannot resolve this by changing an existing test; the human is
not available live, so the question is recorded here for Phase 2.
