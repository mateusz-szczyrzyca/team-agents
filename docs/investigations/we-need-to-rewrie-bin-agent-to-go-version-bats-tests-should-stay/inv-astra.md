# Plan: replace the Bash agent with a Go executable

Investigator: inv-astra. Examined base: `940f6fdc067069f92a8023c84ed1b5eb011b95ae`.
This is an implementation plan, not an implementation of the replacement.

## Recommendation and completion contract

Build a separate, standard-library-only Go command in `agent/`, retaining the existing Bash
implementation as the behavioral reference until the replacement is complete. Port behavior in
small test-first rounds. Keep every existing Bats test, helper, and stub unchanged. Add Go tests
for the same contracts in the round implementing each contract; do not defer test translation
until after the rewrite. Switch installation to the Go executable only after full parity.

The final build produces an actual executable at `bin/agent`; Go sources live in `agent/*.go`.
Do not commit a compiled binary or use `go run` as the installed command. Keep `agentbus` and
`agentlive` as separate processes and preserve their CLI contracts. Replacing their architecture,
changing provider flags, changing state formats, and adding dependencies are outside this rewrite.

There is an important literal incompatibility: **all existing Bats tests cannot run directly
against an ELF binary at their current source path.** Some source the Bash program and replace
its functions. Preserve those tests using a clearly identified Bash reference lane and add Go
counterparts. Use a staging harness for unchanged Bats executable scenarios. Do not call a green
Bash reference run proof that Go works. The proposed staging approach below resolves this without
editing existing tests. If the intended acceptance criterion instead requires every sourced Bash
function assertion to execute Go directly, that requires a separately authorized test-interface
change; an ELF file cannot satisfy Bash `source` or `declare -f`.

This plan has 56 initial rounds, with explicit splitting rules. It is a work breakdown, not a
promise that each subsystem will fit on its first attempt. Aim for 1,000–1,300 changed lines and
stop at **1,500 added plus deleted lines per round**, including tests and documentation. This is
stricter than the repository's usual 3,000-line hard limit. Generated error output retains the
existing exclusion; handwritten implementation or tests must not be excluded to meet the limit.

## Evidence from this repository

At the examined base, `bin/agent` has 8,583 lines; `tests/unit/` has 120 Bats files containing
480 `@test` declarations. These are inventory counts, not passing-test counts. Recount at R01
if another change has landed. Read the entire applicable `AGENTS.md`, `README.md`, and
`docs/tdd.md` before implementing a round.

| Evidence | Consequence for the rewrite |
|---|---|
| `tests/unit/helpers.bash:3–19` computes `REPO_ROOT`, builds agentbus, and copies `$REPO_ROOT/bin/agent` into an isolated HOME. | Changing PATH or setting an invented `AGENT_BIN` variable alone does not select the Go candidate. |
| `tests/unit/meta.bats` sources the program and calls `meta_set` concurrently. | Add an actual Go metadata concurrency test; an executable smoke test is insufficient. |
| `tests/unit/node_agents.bats` and `node_report_agent.bats` extract function bodies with `sed`. | A small launcher alone cannot preserve this baseline; the reference source must remain available in the staged path. |
| `tests/unit/node_prelaunch_cancel.bats` uses `declare -f meta_set` and overrides it to fail one write. | Go needs an injected storage failure at the same step, not only a successful node test. |
| `tests/unit/agent_gh.bats` and `review_byline.bats` symlink the program as `gh`. | Dispatch by invocation basename before loading ordinary agent configuration; preserve the trusted GitHub filter. |
| `bin/agent:288`, `config/agents.conf`, README Configuration | Configuration is executable Bash with associative arrays, expansions, and ordered lists; it is not a dotenv file. |
| `bin/agent:467–522` | Metadata uses Bash `%q`, a separate `.lock` file, atomic replacement, optional run fencing, and a job transition journal. |
| `bin/agent:4507–4943` | The runner has process pipelines, signals, finalization, retry/bounce precedence, and systemd self-invocation. |
| `bin/agent:3215–4135`, `6597–7983` | Investigation pipelines and remote nodes are substantial state machines, not incidental command wrappers. |
| `Makefile`, `tools/gate.sh` | Adding a module to `GO_MODULES` alone does not add its tests, builds, or doccheck coverage. |
| `tests/unit/lead_install.bats`, `node_install.bats` | Installer tests stub `go` with a successful no-op; a new copy-from-build-output step can break them even when real builds work. |
| `agentbus/roundsize.go:169–182` | Round accounting uses `git diff --numstat -z -M`; a genuine byte-identical rename is supported. |

Source references are to the examined base and will move. Use function names when implementing.

## Target design

Use an independent `agent/go.mod` with the repository's supported Go version (currently 1.22).
This is a local module, not a new third-party dependency. Prefer one `package main` split by domain
initially; there is no need to introduce a framework or many import boundaries. Small files and
explicit owned state matter more than the number of packages.

```text
agent/
  main.go, cli.go, help.go                argument dispatch, exit status, argv[0] handling
  app.go, types.go, errors.go             owned dependencies and typed operation errors
  errors.json, errors_gen.go              per-operation causes, generated by tools/errgen
  config.go, config_shell.go              Bash compatibility boundary, typed configuration
  process.go, process_linux.go            argv/env/cwd, pipes, process groups, reaping
  logging.go, bus.go                      stable events and agentbus subprocess adapter
  meta.go, meta_shell.go, locks_linux.go   old files, atomic writes, cross-process locks
  task.go, task_ids.go, git.go             task admission, rollback, checkout operations
  systemd.go                             existing units, timers, watches, self path
  run.go, runner.go, runner_result.go     run commands and explicit finalization phases
  watchdog.go, limits.go, wake.go         inactivity, quotas/network, wake guards
  prompt.go, prompt_context.go            rendering, checkpoints, fresh-session decisions
  cli_claude.go, cli_codex.go              provider argv/config/result adapters
  cli_gemini.go, cli_opencode.go
  questions.go, answer.go, notify.go      question lifecycle and delivery
  resume.go, resume_team.go               session and worktree continuation
  team.go, team_finish.go, helpers.go     membership, closure, helper dispatch
  rounds.go, precheck.go, review.go        round checks and verification
  review_pair.go, autopilot.go            paired reviews, merge eligibility
  investigation.go                       queued investigator requests
  investigation_team.go                  cohort admission and capacity leases
  investigation_phase1.go                publication and recovery
  investigation_pipeline.go              merger/reviewer transitions
  investigation_resume.go                advisory passes and completion
  jobs.go, remote.go, remote_questions.go lead/proxy commands and remote controls
  node.go, node_state.go, node_launch.go   daemon, assignment state, durable admission
  node_report.go, node_controls.go        reports, acknowledgements, cancellation
  node_sync.go, node_sync_limits.go       mirrors, bounded payloads, cursors
  authenticated.go, archive.go            account probes and archive lifecycle
  status.go, transcript.go, tokens.go     presentation and usage summaries
  *_test.go, testdata/                    additive Go tests, legacy-format fixtures
```

Split a file further whenever its behavior no longer has one clear owner. Do not recreate Bash
globals as package variables. `App` owns configuration, output streams, command execution, clock,
random source, and paths. A per-run struct owns task/run identity, session, cancellation, and
delivery state. Pure functions select transitions; explicit effect functions persist or execute
them. Model finalization states such as result committed and delivery pending separately.

Operation methods return their own concrete error types. Reuse the existing errgen tool with
an `agent/errors.json` specification and a local generic error core; it already emits
`package main`. Add generate/check-generated coverage for this module without changing
`agentbus/errors_gen.go` by hand or importing the `agentbus` command package. Add each operation's
causes in its RED round, not all future causes in R02. Standard-library errors are converted at
the boundary. Never assign a typed nil to an error interface. Keep the existing CLI's numeric
exit behavior; agentbus exit classes do not automatically become agent command exit codes.

### Boundaries that must remain compatible

1. **Configuration and secrets.** Use a narrow Bash subprocess to evaluate trusted local
   `agents.conf` at top level and serialize known scalars, associative maps, ordered arrays,
   and necessary exported environment through a dedicated descriptor. NUL-delimited records
   avoid escaping ambiguities and unsolicited stdout. Preserve unset versus explicitly empty
   values, environment/default precedence, legacy investigator settings, and shell expansion.
   Preserve `RUN_PATH` and when secrets are sourced; they are not all loaded for every command.
   Test exported hook variables and removal of empty provider keys. Never log serialized secrets.
   Bash remains a documented compatibility dependency; retaining this evaluator does not mean
   retaining the Bash orchestration program. Shell hooks configured by users remain shell hooks.
2. **Metadata.** Keep `meta.env`, field names, representations, unknown fields, and status
   semantics. Start with a small shell codec for the existing quoted format, behind a typed Go
   API, rather than inventing a partial parser. A native codec is a separate optional project
   after parity. Snapshot synthetic pre-change files and test both Bash-write/Go-read and
   Go-write/Bash-read, including quotes, backslashes, newlines, empty strings, Unicode, and
   optional/unknown fields. Do not replace metadata with JSON in this rewrite. Preserve exact
   simple `STATUS=done` spelling required by existing readers. Check live's readers too.
3. **Locks.** Use Linux file locks compatible with `flock`, on the same lock paths and with the
   same timeout/failure policies. A Go mutex cannot coordinate with other processes. Document
   the existing lock order and deliberate lock coverage of task creation/registration. Mark
   descriptors close-on-exec; retain explicit exemptions for descriptors intentionally passed.
   Do not hold bus-state locks across polling or change agentbus's transaction ownership.
4. **Storage.** Retain manifests, append logs, journal ordering, permissions, and atomic rename
   behavior. Preserve stale-run successful no-ops. Keep large assignment IDs exact: avoid
   decoding untyped JSON numbers through `float64`. Do not apply stricter enum validation to
   an old persisted value without backward-compatible handling and a fixture test.
5. **External commands.** Keep git, agentbus, agentlive, systemd-run/systemctl, gh, and 7z as
   boundaries. Use argv arrays, explicit cwd and environment. Only documented shell commands
   such as configured gates/hooks use a shell. Characterize current EXTRA_ARGS splitting before
   implementing it; substituting a shell-words parser changes semantics. Retain jq where an
   existing public interface accepts arbitrary `--jq` expressions.
6. **Runner processes.** Preserve process groups, raw logs, normalizer pipes, session extraction,
   final-message selection, signal exit behavior, and child cleanup. `CommandContext` alone
   does not replace descendant termination. Close all pipe ends, propagate failures, and wait
   for both producer and normalizer. Bounded streaming must not silently hit Scanner's default
   token limit on large JSON lines. Finalization must finish before `main` calls `os.Exit`.
7. **Interfaces.** Preserve command aliases, hidden commands, CLI text consumed by scripts,
   stderr/stdout separation, JSON field types, event names, unit names and flags, timer/path
   command arguments, and executable self paths. Read-only commands may have documented side
   effects such as acknowledgement; characterize them rather than assuming purity.
8. **GitHub trust.** Preserve every filtered read, rejected path, review byline, and trust decision.
   No provider upgrade or GitHub policy redesign is part of the translation. Use only local
   fake gh fixtures in tests; mutation assertions must never reach a real account.

## Test migration without changing the baseline

### Three verification lanes

**Reference lane:** run all 480 existing cases on the Bash reference in a temporary staged
repository layout. Test files, helpers, stubs, prompts, and fixtures are copied byte-for-byte;
do not edit their contents. Initially the reference is the unchanged `bin/agent`. At cutover,
rename that source byte-for-byte into `agent/testdata/legacy-agent.bash`. Never install it as the
production command. Keep a recorded content hash. This fixture is necessary for source-dependent
regression tests and does not need a checkout of another agent's branch or a network fetch.

**Candidate Bats lane:** construct a separate temporary repository layout containing the exact
same tests. Its `bin/agent` is a generated test-only Bash bridge consisting of a small execution
guard followed by the reference source. When executed, the guard invokes the already-built Go
candidate; when sourced or read with `sed`, the reference functions remain available unchanged.
The bridge must work after `helpers.bash` copies it into a temporary HOME, so embed an absolute
candidate path inside the private stage. Preserve basename invocation using `exec -a` for `gh`.
Put the execution guard before the original script's `gh` fast path and configuration loading.
Keep the shebang first. Run the existing test runner from that staged root so `REPO_ROOT` and
relative extraction paths resolve correctly. Check copied test hashes before and after execution.

During development, the bridge may use an explicit allow-list of completed command entry points;
other executions run the reference. Select complete paths (including hidden callbacks), not just
an apparent top-level command. This is test infrastructure only. Log routing to a separate test
artifact, never into command output. A test with fallback is marked mixed and cannot certify a
Go path. By R54, require zero executable-command fallback. The Go candidate must not contain an
implicit fallback to Bash on errors or unknown commands.

Source calls and function extraction in this lane still test Bash, including tests that override
shell functions. Label them `reference-internal`, even if some child executable invocations use
Go. They require independent Go equivalents before acceptance. In addition, run the classified
command-only Bats cases in a stage whose `bin/agent` is the **actual Go binary**, to prove the
bridge is not hiding installation, basename, signal, or self-invocation differences. Mixed Bats
files require per-case classification; do not drop a whole file because one case sources Bash.
All original cases still run in the full reference and bridge lanes; the binary-only subset is
an additional check, not a replacement suite or a skip list.

**Go lane:** each Bats contract gets Go test coverage at its appropriate level. Pure parsing and
transition rules become unit tests; CLI/run/worktree/installer behavior becomes Go subprocess
contract tests against the compiled executable, using isolated homes and stub programs. Calling
`bats` from `go test` does not constitute a translated Go test. Reusing unchanged shell stubs as
process fixtures is acceptable. Do not try to convert every asynchronous scenario into a pure
unit test by mocking away the behavior it verifies.

R04 must prove the staging mechanism before migration relies on it: a sourced function works,
an extracted function works, a copied executable reaches the Go candidate, `gh` basename is
preserved, self callbacks reach the chosen executable, multi-home tests still work, no fallback
is hidden, and a deliberately failing candidate cannot pass its candidate assertions. No existing
helper or stub behavior may be changed to make these checks pass. Add harness tests separately.

### Parity ledger and fixtures

R01 creates `docs/agent-go-parity.md`, with one row per `(Bats relative path, exact test title)`.
Each row records assertions/branches, source/executable/mixed/build-tool classification,
implementing round, Go test/subtest names, fixture source, routing evidence, and completion.
The appendix below assigns file-level owners; R01 refines mixed files into case-level rows.
480 declarations is the starting denominator, not a quota of Go functions: one Bats loop can need
many Go subtests, and one end-to-end case can need both unit and subprocess tests.

For example, `node_cancel_exit.bats` iterates terminal outcomes. Preserve every outcome, existing
finish timestamp, notification, and log assertion in Go. `node_prelaunch_cancel.bats` injects a
specific metadata-write failure: inject the same failure, then assert no task, branch, or worktree
survives. `meta.bats` needs both goroutine and separate-process contention checks, including a
Bash writer and a Go writer sharing the lock. `authenticated.bats` needs exact selected model,
HTTP code/provider message, bad-key classification, and refusal of an unknown agent.

Write synthetic old-format fixtures, not copies of real host tasks or secrets. Differential tests
run the Bash and Go implementations in separate temporary homes/repos with the same scenario.
Compare exit status, stdout/stderr, persisted fields, external argv, typed bus effects, and logs.
Normalize only declared nondeterministic values (temporary root, PID, timestamp), retaining their
relationships. Do not normalize away ordering, run numbers, assignment identity, or failure causes.
Do not bless Go output as its own oracle or regenerate expected results from the candidate.

Tests own their processes and wait for conditions using deadlines. Use injected clocks/random
sources for decision logic and real subprocesses for locking, signals, and crash recovery.
Use `t.TempDir`, local bare Git remotes, isolated Git configuration, and fail-closed process fakes.
Go tests must fail if they attempt an unregistered real provider, systemd operation, or network
destination. Network fixtures are loopback only. No `make integration` or real `authenticated`
invocation is authorized by this plan.

## Round protocol and exact checks

For every round:

1. Start from the previous merged green round. Read the ledger rows and named Bash functions.
   Estimate implementation + tests + fixtures + docs + deletions before writing code. Split early.
2. **RED:** add only this round's Go tests and the signatures/types needed to compile. Use
   zero-value or typed-not-implemented bodies unreachable from production. Run the targeted
   tests and observe an assertion failure. Run `make gate` before the RED commit: its only
   failures may be these new tests. Record the command and failure excerpt in the commit body.
   Do not commit a compilation error as RED, and do not add future skipped tests.
3. **GREEN:** implement only those contracts. Run targeted Go tests and relevant unchanged Bats
   cases; run all existing checks before committing. Add the passing command/output to the body.
   Keep required orchestrator trailers last. Push/merge a round only at a fully green branch tip.
4. **REFACTOR:** simplify without editing established tests. Run affected checks again; repeat
   the full gate before any further commit. A discovered regression gets another RED/GREEN cycle.
5. Update the parity ledger and README where developer or user behavior changed. Attach routing
   evidence, compatibility notes, test results, size, and any permitted skips/suppressions to the
   round report. No GitHub workflow edits; propose any necessary CI changes for the human.

Current commands, valid before migration infrastructure exists:

```sh
make gate
make unit-bats BATS_JOBS=4 BATS_FILES='tests/unit/meta.bats tests/unit/node_prelaunch_cancel.bats'
bats -f 'quota reset' tests/unit/agent.bats
git diff --check
git diff --exit-code 940f6fdc067069f92a8023c84ed1b5eb011b95ae -- tests/
```

The last check confirms the original test tree is untouched during this plan. During implementation
new tests are allowed: use a manifest of original tracked test/helper/stub paths and their hashes
to distinguish allowed additions from forbidden changes. Keep provider stub scenarios unchanged.

Proposed targets to implement in R02–R04 (they **do not exist yet**):

```sh
make build-agent                    # (cd agent && go build -o ../build/agent-go .)
(cd agent && go test -count=1 -run 'TestMeta' ./...)
(cd agent && go vet ./... && go test -count=1 ./...)
(cd agent && go test -race -count=1 ./...)
make unit-bats-reference BATS_JOBS=4
make unit-bats-go BATS_JOBS=4        # staged bridge; emit exact routing/classification report
make unit-bats-go-binary BATS_JOBS=4 # extra command-only cases, actual binary in staged bin/agent
make check-agent-parity             # case ledger, original hashes, routing/completion checks
make gate
```

Targeted Bats file selection remains useful while developing. Full `make gate` must always run
all baseline cases; never narrow `BATS_FILES` on a final gate. Keep existing `BATS_SERIAL` scheduling.
R02 adds `agent` to `GO_MODULES`, `unit-go`, `build`, `lint-docs`, and generation checks. R04 wires
new lanes into `unit-bats` so the existing gate and CI entry point execute them; during migration
the candidate lane reports only contracts currently selected as Go. At R54 it becomes exhaustive
for executable paths. After cutover, use these Make targets for the full suite: raw `bats tests/unit`
from a tree with an ELF `bin/agent` cannot satisfy source-dependent cases.

Run race tests when introducing or changing concurrency, and over the whole Go module before
cutover. They supplement separate-process crash/lock tests, not replace them. Run all existing
agentbus/live tests unchanged on every gate. No new third-party packages or build tools are needed.

Size checks: run the configured `agent round-size <implementation-task>` in the implementing task
and inspect `git diff --numstat -M <round-base>...HEAD`. Count added plus deleted lines, not net
growth. If a round reaches 1,300 lines with unfinished work, divide it into reviewed A/B rounds;
keep the remainder unreachable and untested until its own round. Never relax the 1,500 cap.
Do not rely on changing the running installation's configuration to enforce this plan's budget.

## Ordered implementation rounds

Every row inherits the full RED/GREEN protocol and full gate above. The named Bats families are
focused verification, not permission to omit other tests. Each row budgets at most 1,500 lines:
roughly 550–750 implementation, 450–650 tests/fixtures, and 100 docs/build lines. Actual size is
measured, not inferred from the number of Bash lines. Reserve extra splits for R20–22, R30,
R38, R43–49 if their assertions do not fit. Earlier numbers are dependencies unless stated otherwise.

| Round | Implement after writing these tests | Focused verification / milestone |
|---|---|---|
| R01 | Inventory every public/hidden command, Bash function family, config variable, state file, log event, lock, and all 480 Bats cases. Record hashes and baseline results. This is documentation only. | `make gate`; case count and ownership ledger; no production changes. Explain the documentation-only test-first exception. |
| R02 | Add the module, thin CLI/help skeleton, local typed error core, and build/lint/generate/doccheck wiring. Tests: empty argv, help aliases, unknown command exit, error causes and typed nil. | Go skeleton tests; existing Bats remain on Bash. No production selection of unfinished commands. |
| R03 | Add injected command executor, clock/random boundaries, isolated Go subprocess test support. Test argv with spaces/newlines, env/cwd, stdin, stderr, exit codes, failed start, and cleanup. | Tests prove unexpected external execution fails closed. No real CLI/network calls. |
| R04 | Add staged reference/bridge/real-binary Bats lanes and parity checker. Test source/extract/copy/gh/self/multi-home behavior and rejection of a bad candidate. | Unchanged full Bats reference; skeleton-only Go selection. Test hashes identical; no edits to helpers. |
| R05 | `config.go`, `config_shell.go`: table-test all configuration families and defaults, scalar/list/map distinction, legacy investigators, exported env, and secrets timing. | Templates unchanged semantically; extra arguments and RUN_PATH characterized. Split export handling if necessary. |
| R06 | `meta.go`, `meta_shell.go` readers: round-trip old shell encodings, optional fields, unknown fields, task-ID resolution, ambiguous/missing IDs. | New synthetic legacy fixtures read by Go and Bash; current live readers still accept writes prepared by the codec. |
| R07 | Metadata updates and Linux locks: concurrent writers, stale run fence, write/rename/journal failures, lock timeout and descriptor inheritance. | `meta`, `node_checkout`, `node_prelaunch_cancel`; race plus separate-process tests. |
| R08 | `bus.go`, `logging.go`: exact bus argv/status mapping, structured log schema, typed notices, warning-only side effects, output write failure. | `agent` log/notice cases, `resilience`; agentbus compatibility tests unchanged. |
| R09 | Read/display CLI: status/ls, agents, show, ack, bus, inv, notes, helpers, dispatch parsing. Keep dispatch effects for R35. | Exact output/status and missing-ID tests; `run_notes`, `investigation_independent_labels`. Activate only completed read entry points. |
| R10 | Git identity/trailers and GitHub setup: per-run environment, signing, author format, wrapper location, no identity mutation outside run. | `identity`, `agent` identity cases, `github_trust`; verify argv[0]=gh path does not require ordinary config. |
| R11 | `gh` read filtering: trusted/unknown authors, marked comments, forks, API/list/view/GraphQL handling, `--jq`, failed gh. | `agent_gh`, `github_trust`; additive Go tests for every rejection/allow path. |
| R12 | GitHub write formatting: review byline, multiline body handling, PR links/attribution, trusted current-round PR selection. | `github_format`, `review_byline`, `job_attribution`, `job_attribution_paths`, `one_pr_per_round`. |
| R13 | `git.go`: local worktree/branch creation/removal, fetch/ref selection, failed checkout cleanup, no dirty-work loss. | `node_checkout` core cases; temp local remotes, spaces in paths, failing git boundary. |
| R14 | `task.go`, `task_ids.go`: allocate task/cohort IDs, collision retry, rollback partial writes, concurrent admission. | `resume_alloc`, `agent` rejected-start cases; task-ID lock compatibility and no leaked branches/tasks. |
| R15 | `systemd.go`: exact runner/timer/path/check units, env, cwd, self path, unit-active checks and launch failure rollback. | Stub systemd subprocess tests; callback paths and no lock descriptors retained by children. |
| R16 | `prompt.go`: template rendering, placeholders, task text/file/stdin, language, instructions and privileges. | `prompt_file`, `context_hygiene`, `agent` prompt cases; missing/large/multiline input tests. |
| R17 | `prompt_context.go`: checkpoint selection, context trimming, resume notes, fresh-session decision and long-run handoff text. | `prompt_checkpoints`, `fresh_session`, `fresh_relative`, `round_only`, `run_notes`. |
| R18 | Claude and Codex adapters: new/resume/fresh argv, models/extra args, MCP configuration, output/session/final extraction. | `prompt_file`, `investigation_mcp_off`; provider fakes only, including independent no-MCP runs. |
| R19 | Gemini/OpenCode adapters: configuration, presets, standalone env, MCP disabled for independent runs, terminal errors. | `investigation_team_opencode_schema`, `investigation_mcp_off`; same semantic adapter tests as R18. |
| R20 | `process_linux.go`, `watchdog.go`: streaming normalizer, activity, hung groups, descendants, signal propagation, drain/wait errors, large lines. | `agent` watchdog, `resilience`, `long_run_cut`; clock unit tests plus real fake-process group tests. |
| R21 | `runner_result.go`: result precedence for status/exit/quota/network/questions/helper/team roles; valid/invalid ROUND_DONE and bounce decisions. | `limits`, `network`, `agent`, `findings`; pure tables cover each branch before runner orchestration uses them. |
| R22 | `runner.go`: start → execute → persist outcome → deliver lifecycle, stop markers, terminal outcome preservation, delivery retries. | `node_cancel_exit`, `remote_control_kick_exit`, `resilience`; crash/signal injection around every durable phase. |
| R23 | Questions and notifications: parse/recommendations/answers, missing STATUS conversion, notification files/hooks, editor, delivery failures. | `agent` questions/notify cases, `pause_notify`; hook subprocess and formatting tests. |
| R24 | `answer.go`: `_answers_saved`, answer/resume validation, watches, answer race/round fencing and replay protection. | `agent`, `autopilot_questions` non-autopilot parsing, `remote_answer_round` shared rules. |
| R25 | `limits.go`, `wake.go`: reset parsing, pauses/timers, wake-loop guards, manual wake/kick, expected-run fencing. | `limits`, `network`, `kick`, `wake_cmd`, `pause_notify`, `long_run_cut`; no timing sleeps in unit tests. |
| R26 | Supervisor/reboot recovery: grace windows, dead unit detection, boot ID, resume limits, orphan/terminal handling. | `restart`, `resilience`, `member_out`; stale actions cannot affect a later run. |
| R27 | Solo resume and cleanup: dirty/untracked stash transfer, same/new round, interrupted rollback, questions/history. | `resume_stash`, `round_only`, `handover_failed`, `agent` solo resume/discard cases. |
| R28 | `team.go`: atomic team admission, worker-only rejection, registration rollback, stop/clean/close across local members. | `team_close`, `agent` team/legacy-state cases, `resume_alloc`; existing bus state stays readable. |
| R29 | `team_finish.go`: coordinator/member outcomes, member questions routing, exclusion, abandoned team and remote-member cancellation. | `member_out`, `coordination`, `team_close`, `pause_notify`; typed bus effects and finish ordering. |
| R30 | `resume_team.go`: checkpoints, same-round branches, new-round IDs, last-team, membership, remote handover and rollback. | `resume_members`, `resume_alloc`, `resume_stash`, `rounds`, `handover_failed`. |
| R31 | `rounds.go`: round limits, test/skip accounting, bounced completion and round reports, old PR supersession. | `rounds`, `skips`, `round_reports`, `round_handover`, `one_pr_per_round`; use existing agentbus size/check commands. |
| R32 | Precheck/verify: gate selection, progress, wait for settled team, review results/findings, fix-pass limits and retry. | `gate`, `verify_loop`, `findings`, `auto_stage`, `review_resilience`; failed checks never advance to merge. |
| R33 | Review pairs: investigator availability/rotation, deterministic injected draws, distinct slots, capacity waits and pair recovery. | `review_pair_random`, `rotation`, `autopilot_review`, `review_resilience`. |
| R34 | Autopilot: questions bounce, protected paths/test-change holds, trusted PR merge eligibility and next-round launch. | `autopilot_questions`, `autopilot_merge`, `round_handover`; all gh writes stubbed. |
| R35 | Queued investigation/helper dispatch: capacity/provider balancing, request/result linkage, queue exit semantics, worker completion wake. | `rotation`, `delegate_jobs`, `coordination`; keep independent investigation teams outside bus messaging. |
| R36 | Investigation team CLI/admission: topic sources, names/roles, immutable base, cohort manifests/leases and launch rollback. | `investigation_team_cli`, `validation`, `stdin`, `isolation`, `admission`; source files use the full prefix in the appendix. |
| R37 | Phase 1: independent run completion, quota holds, manifest recovery, published-ref verification and transport retries. | `investigation_team_runtime`, `supervision`, `regressions`, `publication_retry`, `reboot_regression`, `recovery_regressions`. |
| R38 | Pipeline phases 2/3: seeded selection, replacement limits, phase queue, PR base validation, precheck bounce and reviewer result. | `investigation_team_phase_selection`, `phase_queue`, `handoff`, `single_investigator`; never translate by a single large switch without transition tests. |
| R39 | Investigation completion/clarification: terminal handoff, pending delivery, stop cleanup, legacy phase-4 completion, single pipeline. | `investigation_terminal_completion`, `completed_legacy`, `single_pipeline`, `pipeline_clarification`, `clarification`. |
| R40 | Investigation advisory resume: manifests/pass counters, admission/recovery, prompts, independence, merged/closed PR refusal. | `investigation_team_advisory_resume`, `resume_recovery`, `resume_boundaries`, plus independent/prompt clarification resume files. |
| R41 | `jobs.go`, `remote.go`: remote run submission, proxies, lead/jobs/nodes commands, attribution and remote cancellation routing. | `lead`, `lead_cli`, `lead_cli_context`, `remote_proxy`, `remote_fixes`, `job_attribution_paths`. |
| R42 | Node inventory/heartbeat: agent presence/busy state, capabilities, activity, exact assignment IDs and state mapping. | `node_agents`, `node_activity`, `node_large_assignment`, `node_report_agent`; pure JSON/decision tests first. |
| R43 | Node task admission/launch: allow-list checkout, binding, start acknowledgement, pending launch and failure cleanup. | `node_checkout`, `node_prelaunch_cancel`, `node_recovery_fence`, `node_safety`; inject cancellation between ack and launch. |
| R44 | Node recovery/stop: daemon lock, stale boot/assignment, recovered tasks, terminal cancellation and interrupted finalization. | `node_recovery_fence`, `node_cancel_exit`, `node_prelaunch_cancel`, `restart`; process restart against old node state fixtures. |
| R45 | Node reporting/outbox: transition journal, report replay, retained outcomes, result text, assignment fencing. | `node_report_agent`, `node_two_homes`, `run_notes`, `node_large_assignment`; lost acknowledgement and duplicate reply tests. |
| R46 | Remote controls: validation, identity/heartbeat bounds, stop/wake/kick order, acknowledgment fences and local notification. | `remote_controls`, `remote_control_validation`, `heartbeat_limits`, `cancel_order`, `proxy`, `kick_exit`, `notifications`. |
| R47 | Remote questions: prepare/publish ready marker, atomic transfer, failed preparation retry, round fencing and delivered answers. | `remote_answer_round`, `remote_questions_atomic`, `remote_questions_preparation_retry`, `remote_control_web`. |
| R48 | Node sync core: mirrored metadata and task files, change detection, run identity, freshness, state replacement and two-home flow. | `node_sync`, `node_sync_state`, `node_sync_freshness`, `node_sync_two_homes`; retain exact numeric identities. |
| R49 | Sync limits/review: byte budgets, tails and partial lines, cursor advancement only on accepted data, review paths/visibility. | `node_sync_limits`, `node_sync_tails`, `node_sync_review`, `node_review_boundaries`, `remote_control_review`. |
| R50 | Node daemon/doorbell and remote helpers: poll/cancel/exit, no inherited admission lock, helper result delivery and retries. | `node_daemon`, `node_helper_hooks`, `delegate_jobs`, `node_two_homes`; owned process cleanup tests. |
| R51 | `authenticated.go`: per-agent/provider selection, probe argv/HTTP bodies, response classification, timeout and temp cleanup. | `authenticated`; Go httptest plus fake provider CLIs, never real provider requests. |
| R52 | `archive.go`: relationship/resume grouping, refuse active tasks, pack/verify before delete, worktree cleanup, archive failures. | `agent` archive/discard cases and new fault tests for pack/verify/removal; real 7z only in temporary data. |
| R53 | Transcript/tokens/log, live/web/lead forwarding: ordering/usage parsing, raw/rendered modes, argv and self executable. | `tokens`, `web`, `web_jobs`, `remote_control_web`, `agent` transcript/log; agentlive remains separate. |
| R54 | Close ledger gaps one bounded round at a time; wire all command paths to Go in candidate lanes. Test nested callbacks and all aliases; disallow executable Bash fallback. | All original Bats cases, binary-only Bats subset, all mapped Go tests, race suite. No unmatched test rows or unsupported commands. |
| R55 | Installer/build cutover and byte-identical source rename. New Go installer tests first: clean temp HOME, executable artifact, existing config preserved, failure, gh symlink, service paths. | Existing installer Bats unchanged; all three lanes; verify `-M` rename accounting. No live installation action. |
| R56 | Release qualification: add missing upgrade/rollback/crash regression tests first, test restart from synthetic old tasks and mixed legacy writers. Finish developer/release documentation. | Full gate, module race tests, full parity report, binary acceptance and no orchestration fallback. Split discoveries into further rounds; never waive them. |

R09 activates only read-only implementations whose dependencies exist. Completion of a pure helper
does not authorize routing its whole parent command to Go. R21/R22 can implement runner cores
without making incomplete notification/team flows externally reachable; activate the public run
flow only when its reachable dependencies are complete. For candidates, test lower layers directly
until then. Every intermediate main remains usable because the default installed command remains
Bash until R55. No skipped test stands in for a missing downstream implementation.

### Cutover details and size accounting

R55 uses a real rename of the **unchanged** original `bin/agent` to
`agent/testdata/legacy-agent.bash`. Keep it as the reference fixture; do not delete it in a later
cleanup disguised as a generated file. The rename avoids an 8,583-line deletion round and preserves
the historical oracle. Confirm Git detects 100% similarity and that the repository's round counter
reports the rename correctly. If the source was changed during migration, re-evaluate the actual
rename diff and split other cutover work instead of hiding it with exclusions.

Update `make build` to build the Go command to `bin/agent`, ignore that generated executable,
and have `make test` build what its staged lanes need. Before cutover, use only `build/agent-go`
so compilation cannot overwrite tracked Bash source. Update `lint-shell` to stop syntax-checking
the ELF artifact and to check the retained reference and any new Bash adapter scripts instead.
Preserve its one-shellcheck-process-per-file contract (`lint_shell_memory.bats`). Keep Go files
covered by gofmt, golangci-lint, staticcheck, vet, tests, and doccheck.

Change `install.sh` to build the Go module to the installed executable path, as it already builds
agentbus/agentlive. Avoid a required `cp` of a build artifact that existing successful no-op `go`
stubs never create. Do not add test-only bypasses to the installer. New Go installer tests use a
real build or a separate additive fake that produces a real artifact; they verify the binary in
a fresh temporary home, unlike the old service-layout tests. If a safe installer change actually
contradicts an unchanged test, stop that implementation step and request the required exception;
do not weaken either the production checks or the old test.

All source-dependent tests run in the staged layout after cutover; their original files stay in
`tests/unit`. The bridge/reference source is test-only and must never be reachable from installed
Go command error handling. Standalone release packaging must include the test fixture if it ships
the Bats suite. Update README architecture, build/install, configuration compatibility, tests,
rollback, and remaining external-command dependencies. No changes to `AGENTS.md` are authorized;
propose updating its Bash component description in the round report for human action.

Qualification must show that an old task resumes under Go and a Go-written compatible task is
readable after rollback. It must also cover previously persisted systemd/bus callback paths: the
installed path stays stable. Retain the old executable through the deployment mechanism for
rollback; deployment/restarts of the running host are a separate human action, not a migration test.

## Exit criteria

- Every existing test/helper/stub hash is unchanged; all 480 baseline cases remain runnable.
- Every baseline case has asserted Go equivalents, with all its scenarios/branches represented.
- Reference Bats, candidate bridge Bats, actual-binary Bats subset, Go tests, race checks, and the
  complete gate pass. Reports distinguish Bash internal coverage from actual Go execution.
- Every supported public/hidden command and `gh` basename works without Bash orchestration
  fallback. Configuration/metadata compatibility adapters and configured shell hooks are documented.
- Old state loads, new writes remain compatible, file locks interoperate, failed writes do not
  destroy prior state, stopped/stale runners cannot revive or overwrite newer work.
- Installation produces a real Go executable, with clean-home and upgrade tests. The existing
  provider and external-command stubs remain effective; no real paid execution was used.
- Each merged round fits the stricter 1,500-line budget; README and parity ledger reflect reality.
- Any unresolved parity difference blocks cutover, even if average coverage or test counts look good.

## Investigation validation

Only this report is changed by the investigation. No production code, tests, helpers, stubs,
configuration, workflows, or installed services are changed. No hardware devices, real provider
CLIs, or agentbus coordination tools were used. Repository source and local tests provide the
evidence; no external product behavior is assumed.

Validation on this worktree:

- `make gate`: all six steps passed (generated-code check, shell lint, Go lint, doccheck,
  Go vet/tests, and Bats). The Bats log contains 480 passing cases, zero failures, and zero skips.
- File inventory/appendix check: all 120 existing Bats files have round assignments, with no
  unknown or unassigned files. The round table contains 56 initial rounds.
- `git diff --exit-code 940f6fdc067069f92a8023c84ed1b5eb011b95ae -- tests/` passed;
  existing tests, helpers, and stubs were not changed. `git diff --check` passed.
- No missing tools or environment blockers were encountered. The proposed harness, Go code,
  and future acceptance targets are design work and have not been implemented or validated here.

Test-First: not possible — this investigation adds a Markdown implementation plan only; it does
not introduce executable behavior. Future code rounds must follow the RED/GREEN sequence above.


## Appendix: ownership of every existing Bats file

All paths below are relative to `tests/unit/`. Every row also participates in R01 inventory,
R04 harness validation, R54 parity closure, and R56 qualification. Multiple owners mean the
file spans subsystems; R01 assigns each exact case and assertion, without splitting or editing
the original file. Go tests live beside their domain implementation, for example `meta_test.go`,
`runner_result_test.go`, `node_controls_test.go`, and `archive_test.go`.

| Existing Bats file | Port/verify in round(s) |
|---|---|
| `agent.bats` | R08–R10, R14–R16, R20–R25, R27–R28, R52–R53 |
| `agent_gh.bats` | R11 |
| `authenticated.bats` | R51 |
| `auto_stage.bats` | R32 |
| `autopilot_merge.bats` | R34 |
| `autopilot_questions.bats` | R24, R34 |
| `autopilot_review.bats` | R33 |
| `context_hygiene.bats` | R16–R17 |
| `coordination.bats` | R29, R35 |
| `delegate_jobs.bats` | R35, R50 |
| `findings.bats` | R21, R32 |
| `fresh_relative.bats` | R17 |
| `fresh_session.bats` | R17 |
| `gate.bats` | R32 |
| `github_format.bats` | R12 |
| `github_trust.bats` | R10–R11 |
| `handover_failed.bats` | R27, R30 |
| `identity.bats` | R10 |
| `investigation_clarification.bats` | R39 |
| `investigation_completed_legacy.bats` | R39 |
| `investigation_independent_labels.bats` | R09, R36–R40 |
| `investigation_independent_resume_prompt.bats` | R40 |
| `investigation_mcp_off.bats` | R18–R19 |
| `investigation_pipeline_clarification.bats` | R39 |
| `investigation_prompt_clarification_resume.bats` | R40 |
| `investigation_resume_independence.bats` | R40 |
| `investigation_single_investigator.bats` | R38 |
| `investigation_single_pipeline.bats` | R39 |
| `investigation_team_admission.bats` | R36 |
| `investigation_team_advisory_resume.bats` | R40 |
| `investigation_team_cli.bats` | R36 |
| `investigation_team_handoff.bats` | R38–R39 |
| `investigation_team_isolation.bats` | R36 |
| `investigation_team_opencode_schema.bats` | R19 |
| `investigation_team_phase_queue.bats` | R38 |
| `investigation_team_phase_selection.bats` | R38 |
| `investigation_team_publication_retry.bats` | R37 |
| `investigation_team_reboot_regression.bats` | R37 |
| `investigation_team_recovery_regressions.bats` | R37 |
| `investigation_team_regressions.bats` | R37 |
| `investigation_team_resume_boundaries.bats` | R40 |
| `investigation_team_resume_recovery.bats` | R40 |
| `investigation_team_runtime.bats` | R37 |
| `investigation_team_stdin.bats` | R36 |
| `investigation_team_supervision.bats` | R37 |
| `investigation_team_validation.bats` | R36 |
| `investigation_terminal_completion.bats` | R39 |
| `job_attribution.bats` | R12 |
| `job_attribution_paths.bats` | R12, R41 |
| `kick.bats` | R25 |
| `lead.bats` | R41 |
| `lead_cli.bats` | R41 |
| `lead_cli_context.bats` | R41 |
| `lead_install.bats` | R55 |
| `limits.bats` | R21, R25 |
| `lint_shell_memory.bats` | R03–R04, R55 |
| `long_run_cut.bats` | R20, R25 |
| `member_out.bats` | R26, R29 |
| `meta.bats` | R07 |
| `network.bats` | R21, R25 |
| `node_activity.bats` | R42 |
| `node_agents.bats` | R42 |
| `node_cancel_exit.bats` | R22, R44 |
| `node_checkout.bats` | R07, R13, R43 |
| `node_daemon.bats` | R50 |
| `node_helper_hooks.bats` | R50 |
| `node_install.bats` | R55 |
| `node_large_assignment.bats` | R42, R45 |
| `node_prelaunch_cancel.bats` | R07, R43–R44 |
| `node_recovery_fence.bats` | R43–R44 |
| `node_report_agent.bats` | R42, R45 |
| `node_review_boundaries.bats` | R49 |
| `node_safety.bats` | R43–R44 |
| `node_sync.bats` | R48 |
| `node_sync_freshness.bats` | R48 |
| `node_sync_limits.bats` | R49 |
| `node_sync_review.bats` | R49 |
| `node_sync_state.bats` | R48 |
| `node_sync_tails.bats` | R49 |
| `node_sync_two_homes.bats` | R48 |
| `node_two_homes.bats` | R45, R50 |
| `one_pr_per_round.bats` | R12, R31 |
| `pause_notify.bats` | R23, R25, R29 |
| `prompt_checkpoints.bats` | R17 |
| `prompt_file.bats` | R16, R18 |
| `remote_answer_round.bats` | R24, R47 |
| `remote_control_cancel_order.bats` | R46 |
| `remote_control_heartbeat_limits.bats` | R46 |
| `remote_control_kick_exit.bats` | R22, R46 |
| `remote_control_notifications.bats` | R46 |
| `remote_control_proxy.bats` | R46 |
| `remote_control_review.bats` | R49 |
| `remote_control_validation.bats` | R46 |
| `remote_control_web.bats` | R47, R53 |
| `remote_controls.bats` | R46 |
| `remote_fixes.bats` | R41 |
| `remote_proxy.bats` | R41 |
| `remote_questions_atomic.bats` | R47 |
| `remote_questions_preparation_retry.bats` | R47 |
| `resilience.bats` | R08, R20, R22, R26 |
| `restart.bats` | R26, R44 |
| `resume_alloc.bats` | R14, R28, R30 |
| `resume_members.bats` | R30 |
| `resume_stash.bats` | R27, R30 |
| `review_byline.bats` | R12 |
| `review_pair_random.bats` | R33 |
| `review_resilience.bats` | R32–R33 |
| `rotation.bats` | R33, R35 |
| `round_handover.bats` | R31, R34 |
| `round_only.bats` | R17, R27 |
| `round_reports.bats` | R31 |
| `rounds.bats` | R30–R31 |
| `run_notes.bats` | R09, R17, R45 |
| `skips.bats` | R31 |
| `team_close.bats` | R28–R29 |
| `tokens.bats` | R53 |
| `verify_loop.bats` | R32 |
| `wake_cmd.bats` | R25 |
| `web.bats` | R53 |
| `web_jobs.bats` | R53 |
