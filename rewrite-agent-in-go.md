# Rewrite `bin/agent` in Go — unified implementation plan

Merged by **inv-kimi** from the Phase 1 reports by `inv-kimi`, `inv-astra`, and `inv-gemini-argon`.
The three original reports remain unchanged beside this file. This document is the execution plan for
later implementation rounds; it does not itself implement the replacement.

Examined base: `940f6fdc067069f92a8023c84ed1b5eb011b95ae`.

## 1. Decisions

1. Build a standard-library-only Go 1.27 command in a new `agent/` module, initially one `package main`
   split into small files by responsibility. `agentbus` and `agentlive` remain separate programs and
   retain their interfaces.
2. Keep the current Bash program as the immutable behavioral oracle throughout development. Build the
   candidate as `build/agent-go`; do not select it for production until feature parity is complete.
3. Use the original **56 dependency-ordered round IDs**, R01–R56, with explicit A/B splits below.
   These IDs are not a promise of 56 execution rounds after splits. Each round has a working cap of **1,500 added
   plus deleted lines**, including implementation, tests, fixtures, documentation, and deletions. This is
   stricter than the repository's normal 3,000-line hard limit and honors the requested roughly 1,500-line
   review size. Generated and vendored files retain the repository's existing exclusions; handwritten code
   is never relabeled to evade the cap.
4. Keep every existing file under `tests/unit/` byte-for-byte unchanged. New Go tests, harness tests, and
   cutover tests are additive and live outside that baseline tree.
5. Verify three different things: the unchanged Bash oracle, unchanged black-box Bats cases executing the
   real Go binary, and Go tests representing **every** Bats case. A green Bash run is never presented as
   evidence that Go works.
6. Do not use a production Bash launcher, an `agent _fn` bridge, exported shell mocks, or implicit Bash
   fallback. Those mechanisms would preserve shell global state, make test routing ambiguous, and would
   not deliver the requested Go executable. The final `bin/agent` is a generated, ignored Go executable,
   not committed binary content and not a script.
7. Port configuration and disk formats compatibly rather than redesigning them. A narrow Bash evaluator is
   allowed for the documented executable-Bash configuration contract; orchestration, metadata, and normal
   process execution move to Go.
   The selected R05B compatibility decision also preserves one configuration evaluation per process,
   secrets-script context, and ordinary stdout/stderr behavior. It does not authorize dropping access
   to internal Bash functions or caller locals. Investigation #163 establishes a conflict between that
   full contract and decision 6 for a fixed data-only evaluator. Preserve production behavior and keep
   candidate activation/cutover blocked; do not resolve the conflict by silently narrowing either rule.
8. Follow RED → GREEN → REFACTOR within every behavior round. Split an over-budget subsystem into A/B
   rounds before writing its tests; never defer tests, add placeholders, or add skips for later work.
9. Use `golangci-lint` to asses if Go code is in good shape. Always use default formatter `gofmt` for
   each *.go file.

## 2. Verified baseline and the source-coupling problem

At the examined base:

- `bin/agent` has 8,583 lines and SHA-256
  `52fefa2be198cca4a57512a9236c2dbcd159a6a42a1e65dcb7d8b9b69df8073b`.
- `tests/unit/` has 120 Bats files and 480 `@test` declarations. These are inventory counts; R01 records
  the actual baseline result and recounts if the base has advanced.
- The tests copy `$REPO_ROOT/bin/agent` into isolated homes. Setting an invented `AGENT_BIN` environment
  variable therefore cannot select a candidate without changing the tests.
- Some cases execute `agent` only as a subprocess. Others source the Bash file, extract function bodies
  with `sed`, inspect `declare -f`, or replace Bash functions and variables. An ELF executable cannot
  satisfy those source-level operations.
- The helper-aware Phase 1 audit classified **359 black-box cases** and **121 source-coupled cases in 49
  files**. A direct-reference-only audit misses calls made through helpers such as `setup_isolation`,
  `control_fixture`, `pair_of`, and attribution helpers. R01 must validate this classification per exact
  test name; the generated ledger, not a permanent hard-coded denominator, is authoritative thereafter.
- `lead_install.bats` and `node_install.bats` replace `go` with a successful no-op stub. Their assertions
  concern installed units and configuration, not the agent binary. New cutover tests must independently
  prove a real Go artifact and compiler-failure behavior without changing those old tests.

The correct response to source coupling is a preserved oracle plus real Go equivalents. It is not a shell
shim that serializes Bash functions and variables into Go, and it is not a candidate lane with hidden
fallback to Bash.

## 3. Target architecture

The initial responsibility map is below. Create a file when its first behavior is implemented; do not
create a tree of empty files in R02. Split a file again when it no longer has one clear owner.

```text
agent/
  go.mod                              # module agent; Go 1.22; standard library only
  main.go, cli.go, help.go            # argv[0]/gh dispatch, commands, usage, exit status
  app.go, types.go                    # owned dependencies, typed statuses and result values
  errors.go, errors.json              # concrete error core and per-operation causes
  errors_gen.go                       # generated by tools/errgen; never edited by hand
  config.go, config_shell.go          # typed settings and narrow Bash compatibility loader
  envfile.go                          # secrets.env loading only at existing trust boundaries
  meta.go, shellquote.go              # native legacy metadata codec and task lookup
  locks_linux.go                      # flock-compatible interprocess locks
  logging.go, bus.go                  # stable JSON events and agentbus subprocess adapter
  process.go, process_linux.go        # argv/env/cwd, pipes, groups, reaping
  systemd.go                          # existing units, timers, paths, self-invocation
  task.go, task_ids.go, git.go        # allocation, admission, rollback, worktrees
  run.go, resume.go                   # run/resume/stop/clean command flows
  prompt.go, prompt_context.go        # templates, checkpoints, fresh-session inputs
  questions.go, answer.go, notify.go  # clarification and delivery lifecycle
  cli_claude.go, cli_codex.go         # provider argv/config/result adapters
  cli_gemini.go, cli_opencode.go
  watchdog.go, limits.go, wake.go     # activity, quota/network classification, wake fences
  runner.go, runner_result.go         # explicit execute/persist/deliver finalization phases
  team.go, team_finish.go             # membership, closure, member transitions
  helpers.go, resume_team.go          # worker admission and team continuation
  rounds.go, precheck.go              # size/test/skip accounting and verification
  review.go, review_pair.go           # findings, bylines, paired review and retries
  autopilot.go                        # merge eligibility and next-round orchestration
  investigation.go                    # single-investigator queue and rotation
  investigation_team.go               # independent cohort admission and leases
  investigation_phase1.go             # publication and recovery
  investigation_pipeline.go           # merge/review phases and handoff
  investigation_resume.go             # clarification and advisory completion
  jobs.go, remote.go                  # lead listener, proxies, remote jobs and controls
  remote_questions.go                 # remote clarification transport
  node.go, node_state.go              # daemon state, inventory, durable admission
  node_launch.go, node_recovery.go    # launch fences and crash recovery
  node_report.go, node_controls.go    # reports, acknowledgements, bounded controls
  node_sync.go, node_sync_limits.go   # mirrors, cursors, tails, payload limits
  authenticated.go                    # provider/model probes and classification
  archive.go                          # archive grouping, verification, atomic removal
  status.go, transcript.go, tokens.go # presentation and usage accounting
  *_test.go, testdata/                # additive parity tests and synthetic old-format fixtures
```

### Design rules

- **Owned state, pure decisions, thin effects.** An `App` owns paths, configuration, output streams,
  process execution, clock, random source, and runner log sink. A per-run value owns task/run identity,
  session, cancellation, and delivery state. Do not recreate Bash globals as package globals. Parse,
  classify, select, and plan with pure functions; isolate filesystem and process effects behind small
  boundaries. Use real temporary files and Git repositories rather than a large mock filesystem.
- **Typed failures and existing exits.** Add each operation and its causes to `agent/errors.json` in its
  RED round and generate its error types with the existing `tools/errgen` conventions. Convert storage,
  value, and process errors explicitly; never ignore an error or assign a typed nil to an `error`. Preserve
  the current `agent` command's numeric exits instead of adopting `agentbus` exit classes.
- **No new bus owner.** Continue invoking `agentbus` through its stable CLI. Do not import its `main`
  package, duplicate its state machine, or create a second writer for bus state.
- **No framework or new dependency.** Use the standard library, Linux `flock`, existing external commands,
  and test fixtures already in the repository. No real provider CLI or paid integration test is part of
  these rounds.
- **Be careful with concurrency** If your go code uses multiple goroutines, explain in a report synchronization
  techniques, primitives and potential problems that may arise because of this code. Remember to use `-race` option
  in your `go run`, `go test` and `go build` commands to test against races.

## 4. Compatibility boundaries to characterize before porting

### Configuration and secrets

`agents.conf` is executable Bash with scalars, indexed and associative arrays, expansions, and legacy
investigator settings; it is not a dotenv file. Use a fixed, non-interactive Bash invocation to source it
and serialize only declared settings over a NUL-delimited protocol on a dedicated descriptor. Apply
defaults in Go, preserving unset versus explicitly empty and `${VAR-…}` versus `${VAR:-…}` behavior.
Characterize `AGENT_EXTRA_ARGS` splitting and `RUN_PATH` before implementing them.

Load `secrets.env` with `set -a` only at the current runner/authentication boundaries, not on every CLI
call. Return the exported environment with `env -0`; never log serialized secrets. Preserve exported hook
variables and removal of empty provider keys. User-configured gates and hooks that are documented as shell
code retain that narrow shell boundary; internally constructed commands use argv arrays without shell
interpolation.

Dispatch the trusted `gh` fast path using `filepath.Base(os.Args[0]) == "gh"` **before** ordinary
configuration loading. Replace `readlink -f` self-path assumptions with `os.Executable` and
`filepath.EvalSymlinks` as needed.

### R05B compatibility disposition (investigation #163)

R05A is merged internal configuration/defaults machinery, not complete executable-script parity. Its
current evaluator discards ordinary output and has no `bin/agent` internal functions. It remains
unwired. Investigation #162's repeated configuration source, omitted internal context, and discarded
secrets output are rejected as implementation choices under the selected preserve-contract decision.

A retained Bash evaluator can source configuration once, publish its configuration snapshot, wait for
fixed typed runtime data, source secrets at the caller's boundary, publish the result, and exit. This
preserves unexported configuration variables, arrays, and configuration-defined functions without an
hours-long shell for scripts whose effects end at that boundary. It is a mechanism, not full parity:

- Runner secrets can call `meta_set`, read and mutate `id`, `mode`, `dir`, `n`, and see `main`'s `cmd`.
  They can replace `git_identity`; the original runner invokes that replacement after secrets return.
- Auth secrets run with `p`, `model`, `tmp`, `rc`, caller arguments, and enclosing call frames. The
  `--agent` command-substitution path has different errexit behavior from direct provider auth.
- Configuration output precedes the runner redirection. Runner secrets stdout, stderr, and fd 3 go to
  `runner.log`; auth stdout can become part of a command-substitution result. Preserve ordinary script
  output at its existing sink. The prohibition on logging serialized secrets is not permission to
  suppress that output or copy it into new diagnostics.
- Input-only runtime variables and a returned environment cannot preserve arbitrary function calls,
  changed function bodies, dynamic scope, or shell effects needed by later orchestration. Importing the
  Bash orchestration functions or exposing equivalent generic callbacks would be a forbidden bridge.

The current round is **R05B-DESIGN**, documentation only: update this plan, the README status, and the
round report. Change no implementation, configuration, installer, tests, or parity-ledger state. Keep
`bin/agent` and production selection unchanged. Do not mark secrets parity complete, wire the R05A
loader into a command, activate affected candidate rows, or schedule cutover on this evidence. Budget
at most 250 added+deleted lines for plan/README edits and at most 250 for the round report; total at
most 500, inside the unchanged 1,500-line cap. Run `make gate` before committing and record
`Test-First: not possible — documentation-only compatibility and round-plan revision`.

A future implementation plan must first reconcile the full context requirement with decision 6.
This revision grants neither a bridge exception nor a restricted script contract. More implementation
rounds alone cannot remove the contradiction. If both remain binding, keep the production Bash command
and the Go candidate in development; the native replacement cannot claim completion.

Only after that reconciliation is recorded may the following work be scheduled. Each behavior round
has its own 1,500-line cap, RED/GREEN tests for its implemented behavior, and no future placeholder or
skipped test. Split again before RED if the estimate does not fit:

| Deferred package | Concrete scope | Acceptance boundary |
|---|---|---|
| R05B-TRANSPORT | Injected evaluator session start/read/write/wait/cancel/reap, descriptor ownership, framed snapshots, typed failures. No generic command/eval channel, production integration, or borrowed orchestration functions. | Fake-session tests plus isolated Bash protocol/cancellation tests; no bypass of denied execution. Budget implementation 650, tests 650, docs/report 200 changed lines. |
| R05B-STATE | Source-once config/defaults snapshot; ordered typed boundary data; delayed secrets, exports/pruning, post-secrets settings, and script-output sinks. Preserve startup input/arguments and distinguish unset from empty. | Additive source-count/timing/state/output/failure tests against the actual oracle contexts. Budget implementation 600, tests 700, docs/report 200 changed lines. Data-only fixtures do not establish full context parity. |
| R22 / R51 integration | Runner/auth adapters at the exact existing boundaries, including caller context, mutation propagation, stream routing, and command-substitution semantics under the reconciled contract. | Tests exercise full relevant oracle call paths and their continuations, with fake providers; no real accounts. Unresolved internal functions, caller scope, or later shell effects block activation. Split each adapter into additional 1,500-line rounds if needed. |

Do not renumber the original R06–R56 IDs or rewrite ledger ownership to hide unfinished work. The
next execution-round count must reflect approved splits; this revision does not start a later round.
Independent native work may proceed only within its own authorization and without activating a
command that depends on unresolved configuration/secrets semantics.

### Metadata, locks, and persisted state

Implement a native Go codec for the existing Bash `%q` metadata representation, with golden and
interoperability tests. Read existing simple/unquoted values and preserve unknown lines. Cover empty
strings, whitespace, quotes, backslashes, dollar signs, Unicode, tabs, and newlines. Writes must remain
readable by Bash and `agentlive`; do not replace `meta.env` with JSON or leave a permanent Bash metadata
subprocess in the final orchestrator.

Use Linux file locks compatible with `flock`, on the same lock paths, with the same timeouts, lock order,
and failure policy. A Go mutex does not coordinate with Bash or another process. Mark descriptors
close-on-exec except for intentionally inherited ones, and do not hold locks while polling. Preserve atomic
same-filesystem rename, permissions, journal ordering, cleanup after every failed write, and stale-`RUN`
successful no-ops. Test Bash-write/Go-read, Go-write/Bash-read, goroutine contention, and separate-process
contention.

Snapshot synthetic legacy fixtures for task metadata, investigation phases, node jobs, reviews, logs, and
archives. Do not copy real host tasks or secrets. Preserve JSON field types and large assignment IDs (do
not decode them through `float64`), and do not reject an old persisted value through newly stricter enum
validation without a compatibility fixture.

### Processes, CLI, and GitHub

- Preserve argv, environment allowlists/overrides, working directories, systemd unit names and flags,
  process groups, signals, raw logs, normalizer pipes, session extraction, final-message selection, and
  exactly-once finalization. Close every pipe end and wait for producer and normalizer. Increase buffers
  for large JSON lines rather than inheriting `bufio.Scanner`'s small default token limit. Finish deferred
  cleanup and result delivery before `main` calls `os.Exit`.
- Preserve public and hidden commands, aliases, stdout/stderr separation, output text consumed by scripts,
  JSON event names, callback paths, and executable self paths. Characterize side effects of apparently
  read-only commands, such as acknowledgement, rather than assuming purity.
- Preserve arbitrary `--jq` expressions where the current CLI accepts them; do not replace that interface
  with a fixed filter.
- Preserve every trusted-GitHub read filter, rejected path, review byline, fork/unknown-author decision,
  and write format. Use only fake `gh` fixtures; mutation assertions must never reach a real account.

## 5. Test migration: three explicit lanes

### Lane A — unchanged legacy oracle

Add a staging runner, for example `tools/agent-bats.sh`, that copies the required working tree into a
private temporary repository root and places the Bash oracle at `STAGED_ROOT/bin/agent`. Copy the working
tree rather than using `git archive`, so uncommitted RED/GREEN work can be tested. Run the existing test
runner from that root so `REPO_ROOT`, stubs, prompts, fixtures, and second-home copies resolve naturally.
Never symlink this path to the Go candidate.

Before cutover the oracle is the unchanged tracked `bin/agent`. At cutover it moves byte-for-byte to
`tests/oracle/agent`. All current Bats cases always run in this lane, including source-coupled cases.

### Lane B — unchanged black-box Bats against the real Go binary

Build `build/agent-go` and place that actual executable at `STAGED_ROOT/bin/agent` in a second private
root. Run only ledger rows classified `blackbox` whose state is `active` or `done`. For mixed files, use
Bats `--filter` with an anchored, regex-escaped allowlist of exact test names. Do not create pruned test
copies, add `skip`, or let `BATS_FILES` activate a `planned` case.

There is no bridge and no fallback: if the candidate cannot execute a selected case, that case fails.
Harness self-tests must prove that a deliberately broken candidate cannot pass, that copied executables
and second homes use the candidate, and that `gh` basename and self-callback paths still work. Preserve
existing serial-file scheduling and JUnit reports, and assert selected/executed case counts rather than
trusting the runner exit alone.

### Lane C — Go tests for every Bats contract

Create `tests/agent-go-map.tsv` in R01 with one row per `(Bats relative path, exact test title)`:

```text
bats_file<TAB>test_name<TAB>blackbox|source<TAB>implementation_round<TAB>go_test_or_subtests<TAB>planned|active|done
```

Every row eventually names one or more real Go tests or subtests, including black-box rows. One Bats loop
may require a table of Go subtests; one end-to-end case may require both a pure unit test and a subprocess
contract test. Preserve every assertion, outcome, notification, log, and failure injection—not merely the
case title.

- Pure parsing and transitions become Go unit tests at the new function boundary.
- CLI, runner, worktree, lock, signal, and installer behavior becomes Go subprocess contract tests against
  the compiled executable, using temporary homes and fail-closed stubs.
- Calling Bats from `go test` is not a translated Go test. Reusing unchanged shell stub programs as process
  fixtures is acceptable.
- Do not turn asynchronous crash/recovery scenarios into mocks that remove the behavior being tested.
  Use injected clocks/random sources for decisions and real subprocesses for locking, signals, and crash
  windows.

The parity checker derives Bats names dynamically, rejects duplicate/missing rows, validates classifications
and mappings, and proves `planned` → `active` → `done` lifecycle handling. `planned` rows are not run and
are not skips. `active` rows are this round's RED/GREEN work; a failed row stays active. `done` rows run in
all later rounds. New Bats cases require a helper-aware classification review before the ledger check can
pass.

### Differential and safety rules

Run Bash and Go in separate temporary homes/repositories with identical scenarios. Compare exit status,
stdout/stderr, persisted fields, external argv, typed bus effects, and logs. Normalize only declared
nondeterministic values such as temporary roots, PIDs, and timestamps while retaining their relationships;
never normalize away ordering, run numbers, assignment identity, or failure causes. Expected results come
from the oracle and existing assertions, never regenerated from the candidate.

All tests use `t.TempDir()` or Bats temporary homes, local bare Git remotes, isolated Git configuration,
loopback HTTP only, and process fakes that fail on unregistered commands or destinations. Tests own their
processes and wait on conditions with deadlines. No real `~/agents`, systemd, provider, account, network
service, or running installation is touched.

## 6. Mandatory protocol for every implementation round

1. **Plan and inventory.** Start from the previous merged green round. Read the applicable `AGENTS.md`,
   README, `docs/tdd.md`, the ledger rows, and named Bash functions. Estimate implementation + tests +
   fixtures + docs + deletions. Split before RED if the total cannot fit under 1,500 changed lines.
2. **RED.** Add only this round's Go tests, ledger activation, error causes, and zero-value signatures
   needed to compile. Unfinished production paths remain unreachable or return a typed not-implemented
   error; do not use `panic("TODO")`. Activate unchanged black-box rows only when their complete command
   path and dependencies are the behavior under test. Run focused tests and observe assertion failures,
   not compilation failures. Run `make gate`; its only failures may be those new tests. Commit the tests
   with the failing command/output. No future tests and no skips.
3. **GREEN.** Implement the smallest compatible behavior. Run focused Go tests and the exact active
   black-box Bats cases, then all checks. Commit with the passing command/output. A failed active row is
   never moved back to planned to make the run green.
4. **REFACTOR.** Improve structure without changing established tests. Confirm the refactor diff contains
   no `*_test.go` changes, rerun affected checks, and run the full gate before another commit. Any newly
   discovered bug gets its own failing regression test first.
5. **Close the round.** Mark rows `done` only after their mapped Go tests and, for black-box rows, their
   actual-binary Bats cases pass. Update README and parity evidence for visible changes. Report size,
   compatibility, routing evidence, test results, and any permitted suppression. Do not edit workflows or
   the running installation.

### Commands before the new harness exists

```bash
make gate
make unit-bats BATS_JOBS=4 BATS_FILES='tests/unit/meta.bats tests/unit/node_prelaunch_cancel.bats'
bats -f 'quota reset' tests/unit/agent.bats
git diff --check
```

### Targets to add in R02–R04

```bash
make build-agent                         # (cd agent && go build -o ../build/agent-go .)
(cd agent && go vet ./... && go test -count=1 ./...)
(cd agent && go test -race -count=1 ./...)  # concurrency rounds and final qualification
make unit-bats                           # Lane A: complete unchanged legacy suite
make unit-agent-bats BATS_JOBS=4         # Lane B: active/done rows, actual Go executable
make check-agent-parity                  # ledger, mappings, hashes, execution counts
make gate                                # all ordinary checks plus all required lanes
```

`make unit-go` must include `agent` once the module exists. `make gate` must execute every required lane;
add `unit-agent-bats` to the explicit step list and failure-excerpt handling in `tools/gate.sh` when the
lane is introduced. Full gates never use a narrowed `BATS_FILES`.

### Immutability, compatibility, and size checks each round

```bash
BASE=940f6fdc067069f92a8023c84ed1b5eb011b95ae   # immutable oracle/baseline base
cmp <(git show "$BASE":bin/agent) bin/agent      # before cutover
git diff --exit-code "$BASE" -- tests/unit
git status --short -- tests/unit                 # also catches untracked baseline changes
git diff --check
agent round-size <implementation-task>
git diff --numstat -M <round-base>...HEAD        # inspect added + deleted and rename detection
```

After cutover, replace the `cmp` target with `tests/oracle/agent`. Run `go test -race` whenever concurrency
changes and over the entire `agent` module before cutover. Race tests supplement, not replace,
separate-process lock/crash tests. Keep all existing agentbus/live tests unchanged on every gate.

## 7. Ordered rounds

Every row inherits the complete RED/GREEN/REFACTOR protocol, full gate, immutable-baseline checks, and the
1,500-added-plus-deleted working cap. Named Bats families are focused verification, not permission to omit
other tests. R01 assigns each exact case and assertion; mixed files are never activated as whole files.
Rounds likely to need A/B splits include R20–R22, R30, R38, and R43–R49.

| Round | Implement after writing these tests | Focused verification and activation boundary |
|---|---|---|
| **R01** | Documentation-only inventory: every public/hidden command, function family, configuration variable, state file, log event, lock, all 480 Bats cases, helper-aware classification, original hashes, baseline results, and round ownership. | `make gate`; ledger reports 120 files/480 current cases with no unknown or unassigned row. Record `Test-First: not possible` for the documentation-only commit. |
| **R02** | Module, thin CLI/help skeleton, local typed error core, build/lint/generate/doccheck wiring. Tests for empty argv, help aliases, unknown-command exit, causes, and typed nil. | Go skeleton tests; production remains Bash. Add `agent` to Go build/test/lint/doc/generation coverage without claiming unfinished commands. |
| **R03** | Injected command executor, clock/random boundaries, isolated Go subprocess support. Test spaces/newlines in argv, env/cwd/stdin, stderr, exits, failed start, and cleanup. | Unexpected external execution fails closed. No real CLI, systemd, or network call. |
| **R04** | Staged legacy and real-binary lanes plus parity checker. Test oracle source/extract behavior, candidate copy, `gh`, self callbacks, multi-home, exact filters, JUnit counts, and rejection of a bad candidate. | Full unchanged legacy suite; initially empty or skeleton-safe candidate selection. Test hashes identical; no bridge, fallback, helper edit, or stub edit. |
| **R05A** | Merged configuration/defaults machinery; retain its internal, unwired status. | Existing configuration fixtures stay unchanged; no claim of complete script/output/context parity. |
| **R05B-DESIGN** | Current documentation-only compatibility disposition and revised round plan; at most 500 changed lines including the report. Secrets implementation remains incomplete. | Full gate; no code/test/ledger activation changes. R05B-TRANSPORT/STATE remain unscheduled pending the compatibility reconciliation in section 4. |
| **R06** | Native metadata readers/codec: old shell encodings, optional/unknown fields, task-ID resolution, ambiguous/missing IDs. | Synthetic legacy fixtures read by both Bash and Go; writes accepted by current live readers. No JSON migration. |
| **R07** | Metadata updates, journals, and Linux locks: concurrent writers, stale-run fence, read/write/rename failures, lock timeout, descriptor inheritance. | `meta`, relevant `node_checkout`/`node_prelaunch_cancel`; race plus separate-process and mixed Bash/Go writer tests. |
| **R08** | `bus.go`, `logging.go`: exact bus argv/status mapping, structured schema, typed notices, warning-only side effects, output-write failures. | `agent` log/notice and `resilience` cases; existing agentbus compatibility tests unchanged. |
| **R09** | Read/display CLI: status/ls, agents, show, ack, bus, inv listing, notes, helpers, dispatch parsing. | Exact output/status/missing-ID tests; `run_notes`, `investigation_independent_labels`. Activate only completed read entry points, not dispatch effects. |
| **R10** | Git identity/trailers and GitHub setup: per-run environment, signing, author format, wrapper location, and `gh` fast path before config. | `identity`, `agent` identity, `github_trust`; no identity mutation outside a run. |
| **R11** | `gh` read filtering: trusted/unknown authors, marked comments, forks, API/list/view, GraphQL refusal, `--jq`, failed `gh`. | `agent_gh`, `github_trust`; additive Go tests for every allow/reject path. |
| **R12** | GitHub write formatting: review byline, multiline bodies, PR links/attribution, trusted current-round PR selection, canonical origins. | `github_format`, `review_byline`, `job_attribution`, `job_attribution_paths`, `one_pr_per_round`; all writes stubbed. |
| **R13** | `git.go`: worktree/branch creation/removal, fetch/ref selection, failed-checkout cleanup, no dirty-work loss. | Core `node_checkout`; local temporary remotes, spaces in paths, failing Git boundary. |
| **R14** | `task.go`, `task_ids.go`: allocate task/cohort IDs, collision retry, rollback partial writes, concurrent admission. | `resume_alloc`, rejected starts in `agent`; compatible ID locking and no leaked branches/tasks. |
| **R15** | `systemd.go`: exact runner/timer/path/check units, env/cwd/self path, active checks, launch-failure rollback. | Stub systemd subprocess tests; callback paths and no unintended inherited lock descriptors. |
| **R16** | `prompt.go`: template rendering, placeholders, task text/file/stdin, language, instructions, privileges, identity trailers. | `prompt_file`, `context_hygiene`, `agent` prompt cases; missing/large/multiline input and no `{{` remaining. |
| **R17** | `prompt_context.go`: checkpoints, trimming, resume notes, fresh-session decisions, round-only and long-run handoff text. | `prompt_checkpoints`, `fresh_session`, `fresh_relative`, `round_only`, `run_notes`; investigation-specific prompts remain in their rounds. |
| **R18** | Claude and Codex adapters: new/resume/fresh argv, models/extra args, MCP configuration, output/session/final extraction. | `prompt_file`, `investigation_mcp_off`; provider fakes only, including independent no-MCP runs. |
| **R19** | Gemini/OpenCode adapters: configuration, presets, standalone environment, disabled MCP for independent runs, terminal errors. | `investigation_team_opencode_schema`, `investigation_mcp_off`, adapter isolation rows; full disabled-server schema. |
| **R20** | `process_linux.go`, `watchdog.go`: streaming normalizer, activity, hung groups/descendants, signals, drain/wait errors, large lines. | `agent` watchdog, `resilience`, `long_run_cut`; fake clocks plus real fake-process groups and race tests. |
| **R21** | `runner_result.go`: precedence for status/exit/quota/network/questions/helper/team roles; valid/invalid ROUND_DONE and bounce decisions. | `limits`, `network`, `agent`, `findings`; pure tables cover every branch before orchestration consumes them. |
| **R22** | `runner.go`: start → execute → persist outcome → deliver, stop markers, terminal preservation, delivery retries. | `node_cancel_exit`, `remote_control_kick_exit`, `resilience`; crash/signal injection around every durable phase. |
| **R23** | Questions and notifications: parsing/recommendations/answers, missing-STATUS conversion, files/hooks, editor, delivery failures. | `agent` question/notify, `pause_notify`; hook subprocess and formatting tests. |
| **R24** | `answer.go`: `_answers_saved`, answer/resume validation, watches, answer race/round fencing, replay protection. | `agent`, non-autopilot `autopilot_questions`, shared `remote_answer_round` rules. |
| **R25** | `limits.go`, `wake.go`: reset parsing, pauses/timers, wake-loop guards, manual wake/kick, expected-run fencing. | `limits`, `network`, `kick`, `wake_cmd`, `pause_notify`, `long_run_cut`; no timing sleeps in Go unit tests. |
| **R26** | Supervisor/reboot recovery: grace windows, dead units, boot ID, resume limits, orphan/terminal handling. | `restart`, `resilience`, `member_out`; stale actions cannot affect a later run. |
| **R27** | Solo resume/cleanup: dirty/untracked stash transfer, same/new round, interrupted rollback, question history. | `resume_stash`, `round_only`, `handover_failed`, solo `agent` resume/discard. |
| **R28** | `team.go`: atomic team admission, worker-only rejection, registration rollback, stop/clean/close across local members. | `team_close`, team/legacy-state `agent`, `resume_alloc`; old bus state remains readable. |
| **R29** | `team_finish.go`: coordinator/member outcomes, question routing, exclusion, abandoned teams, remote-member cancellation. | `member_out`, `coordination`, `team_close`, `pause_notify`; typed bus effects and finish ordering. |
| **R30** | `resume_team.go`: checkpoints, same-round branches, new-round IDs, last-team, membership, remote handover and rollback. | `resume_members`, `resume_alloc`, `resume_stash`, `rounds`, `handover_failed`. |
| **R31** | `rounds.go`: round limits, size/test/skip accounting, bounced completion, round reports, old PR supersession. | `rounds`, `skips`, `round_reports`, `round_handover`, `one_pr_per_round`; use existing agentbus size/check commands. |
| **R32** | Precheck/verify: gate selection/progress, settled-team wait, findings, fix-pass limits and retry. | `gate`, `verify_loop`, `findings`, `auto_stage`, `review_resilience`; failed checks never advance to merge. |
| **R33** | Review pairs: investigator availability/rotation, deterministic injected draws, distinct slots, capacity waits and pair recovery. | `review_pair_random`, `rotation`, `autopilot_review`, `review_resilience`. |
| **R34** | Autopilot: question bounces, protected-path/test-change holds, trusted PR merge eligibility, next-round launch. | `autopilot_questions`, `autopilot_merge`, `round_handover`; all GitHub writes stubbed. |
| **R35** | Queued investigation/helper dispatch: capacity/provider balancing, request/result linkage, queue exits, worker completion wake. | `rotation`, `delegate_jobs`, `coordination`; independent investigation teams remain outside bus messaging. |
| **R36** | Investigation-team CLI/admission: topic sources, names/roles, immutable base, cohort manifests/leases, launch rollback. | `investigation_team_cli`, `investigation_team_validation`, `investigation_team_stdin`, `investigation_team_isolation`, `investigation_team_admission`. |
| **R37** | Phase 1: independent completion, quota holds, manifest recovery, published-ref verification, transport retries. | `investigation_team_runtime`, `investigation_team_supervision`, `investigation_team_regressions`, `investigation_team_publication_retry`, `investigation_team_reboot_regression`, `investigation_team_recovery_regressions`. |
| **R38** | Pipeline phases 2/3: seeded selection, replacement limits, phase queue, PR-base validation, precheck bounce, reviewer result. | `investigation_team_phase_selection`, `investigation_team_phase_queue`, `investigation_team_handoff`, `investigation_single_investigator`; transition tables for every state. |
| **R39** | Investigation completion/clarification: terminal handoff, pending delivery, stop cleanup, legacy phase-4 completion, single pipeline. | `investigation_terminal_completion`, `investigation_completed_legacy`, `investigation_single_pipeline`, `investigation_pipeline_clarification`, `investigation_clarification`. |
| **R40** | Investigation advisory resume: manifests/pass counters, admission/recovery, prompts, independence, merged/closed PR refusal. | `investigation_team_advisory_resume`, `investigation_team_resume_recovery`, `investigation_team_resume_boundaries`, `investigation_independent_resume_prompt`, `investigation_prompt_clarification_resume`, `investigation_resume_independence`. |
| **R41** | `jobs.go`, `remote.go`: remote submission, proxies, lead/jobs/nodes commands, attribution, remote cancellation routing. | `lead`, `lead_cli`, `lead_cli_context`, `remote_proxy`, `remote_fixes`, `job_attribution_paths`; loopback/fake bus only. |
| **R42** | Node inventory/heartbeat: agent presence/busy state, capabilities, activity, exact assignment IDs, state mapping. | `node_agents`, `node_activity`, `node_large_assignment`, `node_report_agent`; pure JSON/decision tests first. |
| **R43** | Node task admission/launch: allow-list checkout, binding, start acknowledgement, pending launch, failure cleanup. | `node_checkout`, `node_prelaunch_cancel`, `node_recovery_fence`, `node_safety`; cancellation between ack and launch. |
| **R44** | Node recovery/stop: daemon lock, stale boot/assignment, recovered tasks, terminal cancellation, interrupted finalization. | `node_recovery_fence`, `node_cancel_exit`, `node_prelaunch_cancel`, `restart`; restart against old node fixtures. |
| **R45** | Node reporting/outbox: transition journal, replay, retained outcomes, result text, assignment fencing. | `node_report_agent`, `node_two_homes`, `run_notes`, `node_large_assignment`; lost acknowledgement and duplicate reply. |
| **R46** | Remote controls: validation, identity/heartbeat bounds, stop/wake/kick order, acknowledgement fences, local notification. | `remote_controls`, `remote_control_validation`, `remote_control_heartbeat_limits`, `remote_control_cancel_order`, `remote_control_proxy`, `remote_control_kick_exit`, `remote_control_notifications`. |
| **R47** | Remote questions: prepare/publish ready marker, atomic transfer, failed-preparation retry, round fencing, delivered answers. | `remote_answer_round`, `remote_questions_atomic`, `remote_questions_preparation_retry`, `remote_control_web`. |
| **R48** | Node sync core: mirrored metadata/task files, change detection, run identity, freshness, replacement, two-home flow. | `node_sync`, `node_sync_state`, `node_sync_freshness`, `node_sync_two_homes`; exact numeric identities. |
| **R49** | Sync limits/review: byte budgets, tails/partial lines, cursor advance only on accepted data, review paths/visibility. | `node_sync_limits`, `node_sync_tails`, `node_sync_review`, `node_review_boundaries`, `remote_control_review`. |
| **R50** | Node daemon/doorbell and remote helpers: poll/cancel/exit, no inherited admission lock, helper delivery and retries. | `node_daemon`, `node_helper_hooks`, `delegate_jobs`, `node_two_homes`; owned-process cleanup tests. |
| **R51** | `authenticated.go`: per-agent/provider selection, probe argv/HTTP bodies, response classification, timeout, temp cleanup. | `authenticated`; `httptest` loopback and fake provider CLIs, never real provider requests. |
| **R52** | `archive.go`: relationship/resume grouping, active-task refusal, pack/verify before delete, worktree cleanup. | Archive/discard `agent` cases and fault tests for pack/verify/removal; real 7z only on temporary data. |
| **R53** | Transcript/tokens/log and live/web/lead forwarding: ordering/usage parsing, raw/rendered modes, argv and self executable. | `tokens`, `web`, `web_jobs`, `remote_control_web`, transcript/log `agent`; agentlive remains separate. |
| **R54** | Close ledger gaps one bounded round at a time; complete nested callbacks and aliases; remove every development not-implemented path. | All non-installer command rows done, actual-binary Bats and mapped Go tests green, race suite. No unmatched rows or Bash fallback. Installer rows remain R55. |
| **R55** | Installer/build cutover and byte-identical oracle rename. Add Go equivalents of installer Bats plus new clean-home artifact, compiler-failure, `gh`, service-path, and reinstall tests first. | `lead_install`, `node_install`, `lint_shell_memory`; all lanes and full ledger except release-only R56 additions. Verify `-M` rename accounting; never install on the live host. |
| **R56** | Release qualification: add missing upgrade/rollback/crash regressions first, old-task resume, rollback reads, mixed legacy writers and mixed lead/node versions. Finish developer/release docs. | Full gate, module race tests, complete parity report, binary acceptance, no orchestration fallback. Split discoveries into further rounds; never waive them. |

Completing a pure helper never authorizes activation of its whole parent command. Lower layers are tested
directly until every reachable dependency of the exact black-box case exists. Intermediate `main` remains
safe because the installed production command stays Bash until R55.

## 8. R55 cutover details

**Blocking prerequisite:** resolve the R05B full-script-context/no-bridge conflict and demonstrate the
reconciled contract in runner/auth continuation tests before starting installer cutover. Green
data-only evaluator tests or unchanged Bash tests do not satisfy this prerequisite.

1. **RED installer and gate tests.** Add new tests outside `tests/unit/`, plus mapped Go installer tests.
   Use a clean temporary home, stub systemd, and a test `go` wrapper that delegates only the `agent/`
   build to the real compiler while preserving no-op builds for other modules. Assert an executable Go
   artifact, a working `gh` symlink, preserved configuration on reinstall, stable unit paths, and failure
   with no stale fallback when the agent compiler fails. Add a harness test proving `tools/gate.sh`
   actually executes and excerpts the candidate Bats lane. Commit the failing tests before cutover code.
2. **Preserve the oracle.** Move the unchanged original `bin/agent` to `tests/oracle/agent` as a 100%
   rename and point Lane A there. Confirm its hash and Git's rename detection. Do not delete the oracle in
   a later cleanup or disguise it as generated content.
3. **Build the real executable.** Make `make build` produce `bin/agent` from `agent/` and ignore that
   generated artifact. Change `install.sh` to run
   `go build -trimpath -ldflags='-s -w'` directly into the selected installation path, as it does for the
   other Go programs. Do not add an implicit fallback, a production prebuilt-binary bypass, or a required
   copy from a file that the existing successful no-op `go` stubs never create. A compiler error fails the
   installation. If an unchanged installer test genuinely forbids a necessary safe behavior, stop and ask;
   never weaken the test.
4. **Update all check wiring.** Include `agent` in build, vet/test, gofmt, golangci-lint, staticcheck,
   doccheck, generation, and generated-file checks. Remove generated `bin/agent` from Bash syntax checking,
   add the retained oracle and any new shell tools, and preserve the existing one-shellcheck-process-per-file
   contract checked by `lint_shell_memory.bats`.
5. **Make acceptance exhaustive.** `make test` runs all `agent` Go tests, all legacy Bats cases, all current
   black-box Bats cases against Go, and parity validation. `make gate` explicitly includes the candidate
   lane. Check JUnit totals and zero new skips rather than relying only on process exits.
6. **Document without touching the live installation.** Update README architecture, build/install,
   configuration compatibility, test lanes, rollback, and remaining external-command dependencies. Do not
   edit `AGENTS.md`; propose updating its Bash component description in the round report for human action.
   Release packaging that ships the Bats suite must also ship the oracle fixture. Deployment and restart
   of the running host remain separate human actions.

## 9. Final acceptance matrix

The migration is complete only when every row is true:

| Contract | Required evidence |
|---|---|
| Immutable baseline | Every original `tests/unit/` file/helper/stub is hash-identical; `git diff` and `git status` are empty there; oracle `cmp`/SHA-256 succeeds. |
| Bash oracle remains useful | All current legacy cases (480 at the base) pass in Lane A. |
| Real Go process behavior | All current black-box cases (359 at the base) pass in Lane B against the actual Go executable, with exact execution counts and zero fallback. |
| Same behavioral coverage in Go | Every Bats case, including every source-coupled case and every branch/loop/failure injection, names passing Go test/subtest coverage in the ledger. |
| Complete command coverage | Every public/hidden command, alias, nested callback, and `gh` basename path works without Bash orchestration or a not-implemented stub. |
| Static and concurrency checks | `make gate` passes with zero findings and explicitly runs Lane B; `agent` is covered by vet/lint/doccheck/generation; `go test -race ./...` passes. |
| Executable configuration/secrets | Exactly one configuration source per process; exact secrets timing, caller context, mutation/continuation behavior, output destinations and failure semantics under an explicitly reconciled contract; no silently dropped Bash internals or orchestration bridge. |
| Persistent compatibility | Old configuration, metadata, phase, node, review, log, and archive fixtures load; new writes remain readable after rollback; mixed writers and mixed lead/node versions interoperate. |
| Failure and recovery semantics | Failed writes preserve prior state; stopped/stale runners cannot overwrite newer work; crash-window, signal, acknowledgement, and retry tests pass. |
| Installation and rollback | Isolated clean-home and upgrade/reinstall tests install a real Go executable; compiler failure cannot install/fall back; stable paths support old persisted callbacks. No live service was touched. |
| Round discipline | RED/GREEN/REFACTOR history is visible, no new skip or future placeholder exists, and each round fits the 1,500 changed-line working cap. |

Any unresolved parity difference blocks cutover, even if aggregate coverage or test counts look green.
`make integration`, real provider authentication, and real agent CLIs remain opt-in and are not required or
authorized by this plan.

## 10. Reconciled Phase 1 alternatives

- **Adopted from `inv-kimi`:** direct real-binary staging, helper-aware 121/359 classification, native
  metadata codec, explicit per-test activation, no production wrapper, dependency-aware activation, and
  preservation of the oracle through cutover.
- **Adopted from `inv-astra`:** Go coverage for every Bats contract rather than only source-coupled cases,
  the finer 56-round schedule, detailed process/storage compatibility boundaries, fail-closed test fakes,
  and upgrade/rollback qualification.
- **Adopted from `inv-gemini-argon`:** the responsibility-oriented file mapping and emphasis on visible
  RED/GREEN commits and generated per-operation errors. Its `_fn` bridge, exported shell mocks, permanent
  Bash launcher, and incremental deletion strategy were rejected for the reasons in Sections 1–2. Its
  inventory totals were not used; the checked base has 480, not 483, Bats declarations.
- **Bridge/fallback lanes were not merged.** They would make Bash-internal assertions appear to be Go
  evidence and add a second production compatibility architecture. The three lanes above keep oracle,
  executable, and Go-unit evidence distinct and auditable.