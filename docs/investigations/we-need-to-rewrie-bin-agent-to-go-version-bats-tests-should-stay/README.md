# Unified plan: replace `bin/agent` with Go and keep Bats as the oracle

## 1. Decision

Implement a new, standard-library-only Go command under `agent/`, split by responsibility, and migrate in
small test-first rounds. During development, build it as `build/agent-go`; the installed Bash command remains
untouched. At the end, build the Go program as `bin/agent` and install that binary at the existing path.

Use three explicitly labeled verification lanes:

1. **Historical Bash baseline:** all original Bats files, helpers and stubs, byte-for-byte unchanged, run in a
   disposable export of a reviewed green baseline commit with its Bash `bin/agent`.
2. **Go candidate Bats contracts:** every applicable process-level Bats case, also byte-for-byte unchanged,
   runs against the Go binary in a disposable export of the current candidate tree.
3. **Native Go parity:** every original Bats case has equivalent Go coverage, including cases which source or
   extract Bash functions. These are real Go unit/subprocess tests, not Bats invoked from Go.

This resolves the central constraint without weakening either requirement: the final executable is pure Go,
while all original Bats files remain present, unchanged and executable in the historical lane. The old Bash
script is never a production fallback. A Bash wrapper, a sourceable compatibility implementation, a `_fn`
production dispatcher, or delegating unported commands to Bash would make the parity result misleading and is
not part of this plan.

The three independent reports are retained beside this document:
[inv-kimi](inv-kimi.md), [inv-astra](inv-astra.md), and
[inv-gemini-argon](inv-gemini-argon.md). Where they differ, this document is the merged recommendation.

## 2. Baseline and a prerequisite that blocks implementation

The inspected immutable base is `f6a602465a7d30e4b6a6737515e67e056f757a05`.

| Item | Verified inventory |
| --- | ---: |
| `bin/agent` | 8,523 lines |
| `bin/agent` SHA-256 | `461b7f65ecacd9496302fd351f8acc9d0aafdc8fde06b6ff2312f570c8ca7413` |
| Bats files | 110 |
| Bats `@test` declarations | 462 |
| Go language version in existing modules | Go 1.22 |

The suite mixes command-level scenarios with tests that source `bin/agent`, override shell functions and
globals, or extract Bash function bodies with `sed`. A Go ELF executable cannot preserve those source-level
interfaces. Do not claim that all original Bats cases can directly execute a Go binary. The coverage ledger,
not a filename heuristic or Go coverage percentage, determines candidate-Bats eligibility.

**The inspected base is not green.** All three investigations reproduced two failures, with 460 of 462 Bats
cases passing:

- `investigation_team_isolation.bats` requires independent Codex invocation to omit
  `mcp_servers.agentbus.command`, while `investigation_mcp_off.bats` requires `codex_mcp_args` to include that
  command together with `enabled=false`.
- `investigation_team_opencode_schema.bats` requires the disabled OpenCode MCP object to contain only
  `enabled`, while `investigation_mcp_off.bats` also requires `type=local` and a nonempty command.

These are contradictory existing contracts, not migration regressions. Before Round 0 accepts a migration
oracle, a separately reviewed baseline-repair change must establish which disabled-MCP schema is authoritative
for the supported CLIs. Under repository Rule 1, an implementer must not edit, skip or weaken any existing test
to resolve this. If a test correction is necessary, explicit human authorization is required. Record the
reviewed green successor commit and pin that commit for all later baseline comparisons; never silently replace
the base or describe this base's gate as green.

This prerequisite does not block reading, inventory or planning. It blocks implementation and release
acceptance.

## 3. Non-negotiable boundaries

Read [AGENTS.md](../../../AGENTS.md) and [docs/tdd.md](../../tdd.md) before every implementation round.

- Do not modify, delete, rename, skip or weaken any existing test, helper or stub. New tests are additive.
- Keep each PR at most 3,000 changed lines, aiming for 2,000, including tests. Split a domain into lettered
  sub-rounds when necessary; do not combine unrelated domains to fit.
- Every round ends with a green reviewed baseline and `make gate`, apart from the documented RED commit, where
  only its new assertions may fail. Unfinished candidate commands remain unreachable from the installed CLI.
- Add no third-party Go module or new system dependency without human approval. The `agent` module uses the
  standard library only.
- Never run real provider CLIs, real GitHub/account probes, `make integration`, the installer against the real
  home, or stop/restart/reinstall the running installation. Tests use temporary homes, stubs, local Git
  remotes and loopback HTTP only.
- Do not change `.github/workflows/`; propose CI changes in the implementation PR. Do not change protected
  instruction files.
- Preserve public and internal CLI contracts, on-disk formats, agentbus/live process interfaces, log event
  names and exit statuses. Stricter parsing of an old value is also a breaking change.

## 4. Coverage ledger and immutable fixtures

Create the ledger and its checker before porting behavior. A machine-readable file with a generated Markdown
view is preferable. Start with exactly 462 baseline rows and add later tests as new rows; do not write future
Go tests merely to populate the ledger.

Each row records:

```text
stable ID and pinned baseline commit
Bats file:line and exact test title
classification: process / source-coupled / extraction / installer-build / mixed
setup helpers, stub dependencies and injected failures
observable assertions and external contracts
candidate-Bats eligibility and owning implementation round
Go test and named subtest
baseline result, RED evidence, Go GREEN result, candidate-Bats result
notes on volatile output normalization or difficult parity
```

One Bats case may become several Go tests. Table-driven coverage is valid only when every original case maps
to a named subtest with all original assertions. A check that merely asserts `err == nil` or process exit zero
is not an equivalent port.

The ledger checker must fail for missing or duplicate baseline IDs, vanished files, stale test titles,
unmapped cases, unimplemented release entries, and a filtered candidate run that does not print its selected
and remaining counts. It must also ensure later additive Bats cases cannot disappear from the verification
set.

Store SHA-256 values or Git blob IDs for every original test/helper/stub and verify both source and staged
copies. Keep historical `meta.env`, node state, transitions, sync and activity fixtures under
`agent/testdata/`; fixtures must include data written by the old implementation, not only Go-generated data.

## 5. Target Go design

Create `agent/go.mod` using Go 1.22 and one `package main`. Build during migration with:

```sh
cd agent && go build -o ../build/agent-go .
```

Keep `main` thin: call a testable `run(ctx, args, environment) int` and call `os.Exit` once, after finalization.
An `App` value owns resolved paths/configuration, standard streams, environment, self path, clock, random
source, HTTP client, logger and external-command runner. Pass it explicitly. Use request/result structs rather
than shell-style output globals. Avoid package-global mutable workflow state, process-wide `os.Chdir`, and
shared environment mutation in tests.

Suggested boundaries (split further by responsibility, not to pre-create empty files):

| Files | Responsibility |
| --- | --- |
| `main.go`, `cli.go`, `app.go` | invocation-name dispatch, exact parsing, dependencies, output and exit policy |
| `types.go`, `errors.go`, `errors.json`, `errors_gen.go` | typed enums and per-operation errors |
| `config.go`, `config_shell.go`, `environment.go` | legacy configuration, defaults, paths and environment |
| `logging.go`, `events.go` | stable structured events and file/journal/runner logging |
| `process.go`, `process_linux.go`, `signals.go`, `systemd.go` | argv/env, streaming, process groups, signals and units |
| `bus.go`, `git.go`, `identity.go` | typed agentbus subprocesses, Git/worktrees and commit identity |
| `storage.go`, `lock_linux.go`, `metadata.go` | atomic files, process locks and legacy `meta.env` |
| `task.go`, `allocation.go`, `worktree.go`, `run.go` | task admission, IDs, creation and rollback |
| `prompt.go`, `questions.go`, `notify.go` | prompts, human rounds and notifications |
| `provider_claude.go`, `provider_codex.go`, `provider_gemini.go`, `provider_opencode.go` | provider-specific argv, config, streams and results |
| `runner.go`, `runner_finish.go`, `watchdog.go`, `supervise.go` | runs, finalization, hang handling and recovery |
| `resume.go`, `lifecycle.go`, `wake.go`, `session.go`, `limits.go`, `tokens.go` | resume, stop/kick/wake, sessions and usage/limits |
| `team.go`, `helpers.go`, `investigation.go` | teams, helpers and ordinary investigations |
| `investigation_team.go`, `investigation_phase.go`, `investigation_resume.go` | independent investigation state machines |
| `round.go`, `review.go`, `autopilot.go` | rounds, reviews, precheck/fix/merge automation |
| `github.go`, `github_filter.go` | `gh` basename entry point, trust filtering and bylines |
| `lead.go`, `jobs.go`, `node.go`, `node_state.go`, `node_launch.go` | lead/node admission, heartbeat, launch and recovery |
| `node_sync.go`, `remote_controls.go`, `remote_questions.go` | bounded sync, durable controls and remote answers |
| `status.go`, `transcript.go`, `archive.go`, `authenticated.go`, `ui.go` | inspection, archives, auth probes and live/web forwarding |
| domain `*_test.go`, `testenv_test.go`, `testdata/` | one-to-one parity tests, focused unit tests and old fixtures |

Reuse `tools/errgen` with an agent-local `errors.json`; never edit generated files or add unrelated operations
to `agentbus/errors.json`. Map operation causes to the **existing agent CLI exit codes**, not blindly to
agentbus's exit-code policy. Expected answers such as `TAKEN` remain output, not errors. No error is ignored.
All declarations need the comments required by `doccheck`; statuses, providers, tiers, modes and log events are
typed constants.

### 5.1 Configuration and persisted shell data

Do not write a partial Bash configuration parser and call it compatible. `agents.conf` and `secrets.env` are
executable Bash with arrays, expansion, export/unset behavior and subtle empty-versus-unset semantics. Preserve
them through a narrow, fixed `bash --noprofile --norc` configuration helper:

- pass file paths as argv, never interpolated shell code;
- source files in the existing order and only in contexts which load them now;
- serialize an allowlisted typed schema over a dedicated pipe with length-safe framing, retaining scalar/array
  type and unset versus empty;
- keep configuration stdout separate and never log the serialized secret channel;
- test computed values, arrays, legacy investigator keys, `AGENT_EXTRA_ARGS` splitting, PATH/locale and malformed
  input.

This is compatibility for an established configuration language, not Bash orchestration. Replacing that
language is a separate versioned migration.

Treat `meta.env` as data, not executable configuration. Prove a non-executing reader/writer against Bash
`printf %q` golden and differential fixtures: quotes, backslashes, dollar signs, whitespace, newlines, Unicode,
control bytes except NUL, empty values and unknown keys. Test both ordinary and `C.UTF-8` locales and old-to-new
plus new-to-old reads. A narrowly scoped codec helper may be used temporarily if necessary, but no task
operation falls back to the old Bash command.

### 5.2 Processes, locks and state

- Use `exec.Cmd` argument slices, exact working directories and explicit environment allowlists. Preserve stdin
  for large prompts/JSON; do not move payloads into argv or impose scanner-size limits absent from Bash.
- Preserve child exit statuses and stream/normalizer failures independently. Drain streams and wait for every
  child. `exec.CommandContext` alone does not kill a process group; model sessions, descendants, TERM/KILL and
  exited-PID races explicitly.
- Inventory every lock path, mode, timeout, ordering and descriptor-inheritance rule. Use real cross-process
  `flock`-compatible locks, not Go mutexes. Close locks before reentrant bus/runner calls and never hold them
  across unrelated waits.
- Use sibling temporary files, current modes and atomic rename. Make temporary names unique across goroutines
  and processes. Preserve unknown fields and original bytes after read/write/rename failures.
- Keep agentbus, live/web, Git, systemd, `jq` where required, `7z`/`7za`/`7zr` and provider CLIs behind their
  existing process boundaries. Do not write bus state directly or invent a new archive format.
- Return through one finalization owner before `os.Exit`; deferred functions do not run after `os.Exit`. Model
  run/assignment fences and reboot/SIGKILL recovery explicitly. Do not promise exactly-once external side
  effects that the current design cannot guarantee.

## 6. Build the three verification lanes

### 6.1 Historical baseline lane

Use `git archive` from the pinned **green successor** commit into a temporary directory, then run its original
Makefile/Bats environment with its Bash `bin/agent`. The commit must already exist locally: check it with
`git cat-file -e` and fail with setup instructions if absent. Do not fetch from another agent's checkout or
silently use the network. An operator-supplied checksummed archive is an acceptable alternative.

This lane includes original installer/Makefile tests and therefore runs more than just `bats` against a copied
script. Record the baseline commit, binary/script hash, test count and tool versions. Production installations
do not need this historical source.

### 6.2 Go candidate Bats lane

Add a staging tool under `tools/`, with its own tests, which creates a disposable export of the **current
candidate tree**, including staged/unstaged source and newly added tests. An export of an older `HEAD` is not
sufficient during RED/GREEN development. It must:

- reject missing candidate/baseline inputs and preserve relative paths and executable modes;
- build one candidate binary, record its hash, and place it at the export's `bin/agent` before Bats setup copies
  it into each temporary home;
- route secondary/node homes to the same artifact and prove this in the tool's tests;
- copy original tests/helpers/stubs without text rewriting and verify their hashes;
- select only ledger-approved process contracts, print selected/remaining counts, propagate failures, and fail
  if the candidate binary is removed;
- clean up exports and only those process groups it created.

Do not override `source`, `sed`, `bash` or `install`, add a Bats-only production dispatch, or silently fall back
to Bash. Source-coupled cases are covered by the historical and native-Go lanes, not hidden skips in a claimed
full candidate run.

### 6.3 Native Go parity lane

Port `tests/unit/helpers.bash` mechanics into `agent/testenv_test.go`: isolated `HOME`, `AGENTS_HOME` and stub
home; generated config/prompts/secrets; local bare Git remote and repository; freshly built agentbus; unchanged
stub CLIs on a per-child PATH; and the existing fake-agent scenario JSON protocol.

Use `t.TempDir`, per-instance dependencies and `t.Cleanup`. Substitute clocks/random sources and external
process boundaries per test. Reuse real temporary filesystems and local Git repositories where they are the
contract. Use loopback HTTP for lead/node tests. Poll observable conditions with deadlines like
`wait_status`/`wait_grep`; never synchronize with a bare sleep. On timeout, include bounded logs and terminate
and reap only the test's child groups.

Translate each source/extraction test by its behavior, not its shell seam. For example, replace an overridden
Bash `grep` fault with an injected reader/writer failure; replace a mocked `start_runner` with an injected launch
boundary. Preserve every assertion, including no partial task/branch/worktree after failure. Independent
processes, not just goroutines, must verify metadata and ID-allocation locks. `go test -race` supplements but
does not replace those cross-process tests.

## 7. Test-first procedure for every domain

1. **Characterize.** Read the selected Bats body, setup and relevant stubs. Run the original case in the green
   historical lane. Record exact argv/env, streams, exit status, files, Git state, logs and failure boundaries
   in the ledger.
2. **RED.** Add only this round's Go test and compiling zero-value signatures/typed causes. Run it and confirm
   an assertion fails for missing behavior; a compile failure or `panic("TODO")` is not RED. Run the gate and
   commit the test with its failing output. Only new assertions may fail.
3. **GREEN.** Implement the smallest behavior. Run the focused Go test, the corresponding original Bats case in
   the baseline lane and, if eligible, the unchanged case against Go in the candidate lane.
4. **Cross-check.** Remove or invalidate the candidate in a staging-tool test and prove the candidate lane then
   fails. Compare the ledger assertions, not just status. Update RED/GREEN evidence.
5. **REFACTOR.** Improve structure without changing tests or behavior; run focused tests again.
6. **Gate.** Before every commit, run the required domain checks and `make gate`. Run `go test -race` for
   concurrency work and final acceptance. Check the round size and original-file hashes. Commit GREEN only when
   the complete round head is green.

Commands available at the repository root now:

```sh
make gate
make unit-bats
make unit-go
bats -f 'watchdog' tests/unit
bats tests/unit/meta.bats tests/unit/resume_alloc.bats
(cd agentbus && go test -run TestLegacyStateLoads ./...)
git diff --exit-code <reviewed-green-base> -- bin/agent tests/unit
```

Once Round 1 adds the module and plumbing, use:

```sh
make build-agent-go
(cd agent && go vet ./... && go test -count=1 ./...)
(cd agent && go test -race -count=1 ./...)
make check-agent-parity
make unit-bats-agent-go
make gate
```

These latter target names are proposed deliverables, not commands that exist today. Define
`check-agent-parity` to audit hashes and mappings, with a strict release mode. Define `unit-bats-agent-go` to
run only eligible candidate contracts during migration and require the full compatible subset before release.
There is no root Go module; run module tests from `agent/`.

## 8. Ordered implementation rounds

Every item is a work package, not permission for an oversized PR. Split by provider, transition family or
ledger size. Run `agent round-size <task>` for each actual round. Add `agent` to all relevant Makefile build,
vet, test, lint, doccheck, generation and freshness lists in Round 1; extending `GO_MODULES` alone does not
update every explicit target.

### Round 0 — establish the green oracle and inventory

1. Resolve the disabled-MCP contradiction in a separate reviewed change, with human approval for any existing
   test correction. Pin the resulting green commit.
2. Recount every dispatcher arm, alias, config key, persisted file and Bats case. Record hashes, source coupling,
   assertions and old-format fixtures in the 462-row ledger.
3. Document the three lanes and acceptance criteria. No production behavior changes.
4. **Checks:** full green baseline gate, ledger totals/unique IDs, original hashes and review of every internal
   callback as well as public commands.

### Round 1 — Go skeleton and staging plumbing

1. Add `agent/go.mod`, minimal dispatch, typed errors, `App` dependencies and safe test environment. Start with
   help/unknown-command behavior; unimplemented commands return typed not-implemented errors only in the
   development binary.
2. Build `build/agent-go`; wire all Go generation/lint/test/doccheck targets.
3. Add the baseline/candidate staging tools and their failure/cleanup/hash/binary-selection tests.
4. **Checks:** Go tests, generation freshness, staging tests, original full Bats baseline and `make gate`.

### Round 2 — configuration and logging

1. Port the bounded Bash configuration adapter, defaults, legacy keys and secret separation.
2. Port JSONL/journal/runner logging, stable event names and side-effect warnings while keeping human stdout
   separate.
3. **Checks:** arrays, expansion, unset/empty, malformed files, paths with spaces, secret non-disclosure, and
   relevant `agent.bats`, `identity.bats`, `run_notes.bats`, `pause_notify.bats` mappings; full gate.

### Round 3 — storage, metadata and locks

1. Port atomic replacement, historical metadata/node-state fixtures, unknown fields and typed storage failures.
2. Implement real process locks, ownership/deadlines and descriptor behavior.
3. **Checks:** `meta.bats`, storage/fencing cases in `node_checkout.bats`, `node_safety.bats` and
   `remote_control_kick_exit.bats`; independent Bash/Go writers, reader/writer/rename faults, no lost updates,
   `go test -race`; full gate.

### Round 4 — process, systemd, bus and Git boundaries

1. Add streaming process execution, process groups, signals/exit forwarding and unit lifecycle.
2. Add typed agentbus calls and local-Git identity/worktree operations with partial-failure rollback.
3. **Checks:** long streams, normalizer/tee failures, orphan cleanup, `identity.bats`, `lead_cli.bats`,
   `lead_cli_context.bats`, `web.bats`, and existing agentbus/live tests; full gate.

### Round 5 — `gh` trust boundary

1. Port executable-basename dispatch before workflows depend on it. Preserve pass-through argv/status, `--jq`,
   pagination, review bylines, list/view filtering and protected GraphQL refusal.
2. **Checks:** `agent_gh.bats`, `github_trust.bats`, `github_format.bats`, `review_byline.bats`; trusted,
   untrusted, forked, marked and malformed responses, missing `AGENT_REAL_GH`, fail-closed stdout; stubs only;
   full gate.

### Round 6 — read-only inspection and authentication

1. Port status/show/agents/ack, bus/inv/helpers/notes, nodes/jobs inspection, `_agents-json`, transcript/log
   rendering and token calculations in small sub-rounds.
2. Port `authenticated` and `_authcheck` with controlled processes and loopback APIs.
3. **Checks:** `authenticated.bats`, `tokens.bats`, `run_notes.bats` and relevant inspection cases; provider
   models/modes, unknown agents and sanitized errors; no real probes; full gate.

### Round 7 — task admission and resume preparation

1. Port validation before side effects, collision-safe ID/cohort allocation, task/worktree creation and rollback.
2. Port solo/team resume chains, dirty work/stashes and same-round/new-round decisions; keep launch injected
   until the runner is complete.
3. **Checks:** `resume_alloc.bats`, `resume_stash.bats`, `resume_members.bats`, `resilience.bats`,
   `node_checkout.bats`, `round_only.bats`; concurrent CLI processes and rejected requests leave no artifacts;
   full gate.

### Round 8 — prompts, questions and notifications

1. Port every prompt kind/placeholder, environment/repository instructions, trailers and checkpoints.
2. Port question/answer validation, editor behavior, notifications and legacy `NOTIFY_SCRIPT`/`NOTIFY_CMD`.
3. **Checks:** prompt/identity/question/notification files named by the ledger, no unresolved `{{...}}`, large
   input via stdin, answer races and notification failure records; full gate.

### Round 9 — provider adapters, one at a time

1. Port Claude, Codex, Gemini/Antigravity and OpenCode in separate sub-rounds. Preserve model/extra args,
   session create/resume, prompt delivery, MCP schemas, stdin closure, streams and normalization.
2. **Checks:** fake CLI scenarios for every provider, malformed/missing `STATUS`, large prompts, no hidden
   interactive wait, `investigation_mcp_off.bats`, `investigation_team_isolation.bats`,
   `investigation_team_opencode_schema.bats`, `investigation_team_stdin.bats`; full gate after each provider.

### Round 10 — solo runner, watchdog and recovery

1. Connect admission, prompt, provider and finalization. Port pause/quota/network classification, fresh
   sessions, step/hang limits, missing-status questions and notifications.
2. Port stop/kick/wake, answer resume, supervision/reboot recovery and failed starts with generation fences.
3. **Checks:** solo `agent.bats` cases, `limits.bats`, `network.bats`, `restart.bats`, `kick.bats`,
   `wake_cmd.bats`, `fresh_session.bats`, `fresh_relative.bats`, `long_run_cut.bats`, `handover_failed.bats`;
   process-group/race tests and candidate contracts; full gate.

### Round 11 — teams, helpers and ordinary investigations

1. Port team creation/member lifecycle/checkpoints/close, helper dispatch and investigator queue/rotation.
2. Keep bus-owned state in agentbus and preserve delivery/resume behavior.
3. **Checks:** `coordination.bats`, `team_close.bats`, `member_out.bats`, `rotation.bats`, `verify_loop.bats`,
   `delegate_jobs.bats` and related helper/team scenarios; concurrent dispatch and full gate.

### Round 12 — independent investigation teams

1. Split validation/isolation, phase-1 admission/supervision, publication/retry, merger/reviewer selection,
   phase handoff, resume and reboot recovery into sub-rounds.
2. Preserve immutable bases, no-bus/MCP isolation, availability fences, seeded selection and durable boundaries.
3. **Checks:** every `investigation_team_*.bats` case and `investigation_mcp_off.bats`, including native Go
   mappings for all source-coupled regressions; synthetic temporary homes only; full gate.

### Round 13 — rounds, reviews and autopilot

1. Port round reports/size/skip checks, PR reuse/supersession, review pairing, precheck/fix passes, merge
   blockers, human questions and next-round handoff.
2. Preserve retry/backoff, one-PR-per-round and test-change holds; all GitHub traffic remains stubbed.
3. **Checks:** `findings.bats`, `skips.bats`, `gate.bats`, `round*.bats`, `one_pr_per_round.bats`,
   `review_*.bats`, `autopilot_*.bats`, `auto_stage.bats`; full gate.

### Round 14 — lead commands and node lifecycle

1. Port lead/jobs/nodes commands, repository allowlists, node state/capabilities and daemon lifetime locking.
2. Split prepare, start ACK, final launch fence, cancellation, reports/outbox, heartbeat and boot recovery.
3. **Checks:** `lead*.bats`, `job_attribution*.bats`, `delegate_jobs.bats`, `node_agents.bats`,
   `node_report_agent.bats`, `node_daemon.bats`, `node_checkout.bats`, `node_safety.bats`,
   `node_prelaunch_cancel.bats`, `node_cancel_exit.bats`, `node_recovery_fence.bats`, `node_two_homes.bats`,
   `node_large_assignment.bats`, `node_helper_hooks.bats`, `node_review_boundaries.bats`; full gate.

### Round 15 — remote synchronization and controls

1. Port activity/state sync, clipping, byte limits/hashes, cursors/freshness, metadata allowlists and checkpoints.
2. Port remote stop/kick/pause/answers/reviews with durable acknowledgements, retries and stale-round fencing.
3. **Checks:** all `node_sync*.bats`, `node_activity.bats`, `remote_*.bats`, `web_jobs.bats`; two disposable
   homes and loopback servers, failed-transfer retry, stale controls cannot mutate newer work, race tests and
   full gate.

### Round 16 — archive and destructive cleanup

1. Port complete team/resume/helper/investigation group discovery, active-state refusal, archive collisions,
   pack/validate and only then task/worktree/checkpoint cleanup. Preserve branches and 7z compatibility.
2. Port clean/discard/stop-all interactions only against synthetic tasks.
3. **Checks:** archive/discard/stop-all cases, failed-pack/failed-validation recovery, local archive extraction,
   old metadata/activity rendering and full gate. Never inspect or delete this installation's real tasks.

### Round 17 — complete parity and mixed-version recovery

1. Audit every dispatcher arm, config key, persisted format and ledger row. Add missing Go tests before missing
   implementation; no unclassified case or release placeholder remains.
2. Read old-produced config/task/node data with Go and Go-produced data with old readers. Exercise an
   interrupted old run resumed/supervised by Go and old scheduled callbacks resolving internal commands.
3. Compare stable output/state, normalizing only declared volatile fields such as temporary paths, IDs and time;
   never normalize away status, error, trust, missing-record or lock differences.
4. **Checks:** strict ledger, all Go parity, full eligible candidate-Bats subset, full unchanged historical
   baseline, old/new fixtures, race/process tests and `make gate`.

### Round 18 — pure-Go cutover, installation and rollback

1. Rename the old script unchanged to `tools/legacy/agent.bash` as a readable oracle, using a pure rename so the
   round-size checker does not turn 8,523 moved lines into a giant rewrite. Verify with
   `git diff -M --numstat`; do not add a size-check exclusion.
2. Generate an ignored compiled `bin/agent`; never commit a platform-specific binary. Remove that path from Bash
   syntax/ShellCheck inputs while retaining checks for every remaining shell script. Final `make build` builds
   all binaries and final `make gate` invokes all three lanes, including later additive tests.
3. Update `install.sh` to build to a temporary sibling and atomically rename the Go executable into the existing
   installed path. Preserve config/secrets/prompts policy and callback/unit paths. Add native Go installer tests
   for real artifacts, build/replacement failure and node/lead units; historical no-output Go stubs alone cannot
   verify the new artifact.
4. Update README, configuration templates and development documentation. Propose CI and protected-instruction
   updates without editing those files.
5. Test installation with scratch sources and temporary `HOME`, `AGENTS_HOME`, `XDG_CONFIG_HOME` and runtime
   directory, plus stub systemctl/systemd-run/provider CLIs. Verify installed self-invocation, `gh` symlink,
   live/web, old metadata and archive reading. Never run the installer or restart services in the real home.
6. Roll back by an operator-controlled previous-release binary/package switch, with no runtime fallback. Prove
   before release that old readers accept Go-written data and rehearse interrupted-run recovery in temporary
   homes.

## 9. Per-round completion checklist

- [ ] The pinned baseline is green and original test/helper/stub hashes are unchanged.
- [ ] Current-round tests were written first; RED output and GREEN evidence are recorded, with no future tests
      or undocumented skips.
- [ ] Every mapped assertion and failure injection is preserved; ledger states are accurate.
- [ ] Focused Go, historical Bats and eligible candidate Bats checks pass.
- [ ] All repository Go tests and the full historical Bats lane pass; concurrency rounds also pass `-race`.
- [ ] `make gate` has zero findings, generated files are fresh, and the PR is within the line limit.
- [ ] No real provider/account/GitHub/systemd resource or live installation was touched.
- [ ] No external dependency, runtime Bash fallback, production test hook or hidden candidate filtering exists.
- [ ] README/config/prompts and compatibility documentation are updated for visible changes.
- [ ] The PR identifies unported commands, remaining ledger rows and any parity that was difficult to prove.

## 10. Release acceptance

The rewrite is complete only when:

- `bin/agent` and the installed `~/.local/bin/agent` are the Go-built executable; all public/internal commands,
  scheduled callbacks and the `gh` basename path run in Go with no Bash orchestration fallback.
- All 462 original cases have passing native Go equivalents, every applicable unchanged Bats contract passes
  against Go, and all original Bats cases still run unchanged in the labeled historical lane.
- Existing test files, helpers and stubs remain unchanged. No original assertion was replaced by a generic
  success check.
- Legacy files load, Go-written files remain readable by supported old components, and external argv/env,
  stdout/stderr, exit codes, JSON fields and log events remain compatible.
- The complete gate, race/process tests, clean-checkout build, isolated installation and rollback rehearsal
  pass with zero findings and within the round limit.
- The two baseline MCP expectations were resolved independently and no migration change hides or reintroduces
  that contradiction.

If a future requirement instead insists that every original source/`sed` Bats case directly target the Go ELF,
that requirement is technically incompatible with leaving those tests unchanged. Stop and request an explicit
test-conversion exception; do not silently introduce a Bash shim or weaken coverage.
