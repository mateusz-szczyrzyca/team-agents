# AGENTS.md — team-agents

Instructions for AI agents (and humans) working on this repository. They are binding. Read the whole file
before you start; the [README](README.md) explains how the system is used.

## What this repository is

team-agents runs headless AI coding agents — Claude Code, Codex, Antigravity (Gemini) and OpenCode — on a Linux
host, alone or as teams, under `systemd --user`. It does not implement an agent loop: it starts the agent CLIs,
gives them a protocol (the prompts), a shared bus for coordination (an MCP server) and a human in the loop
through question files and notifications.

**This repository is developed mostly by its own agents.** You are probably running inside a team-agents
installation right now, started by the same `agent` binary you are about to change. Keep that in mind (see
"Never" below).

### Components

| Component | Where | Language | Role |
|---|---|---|---|
| `agent` | `Cargo.toml`, `crates/` | Rust | the CLI and the daemon ([`docs/architecture.md`](docs/architecture.md)): `agent-core` (pure decisions, the state, the prompts' catalog), `agent-runtime` (files, processes, git, GitHub, agentbus), `agent` (the commands and the daemon), `agent-web` |
| `agentbus` | `agentbus/` | Go, standard library only | the MCP server every agent run talks to (messages, reservations, investigations, teams, board, notebook, checkpoints) and the CLI subcommands the daemon calls; its own state in the bus directory under `flock` |
| prompts | `prompts/` | Markdown | the texts given to the agents and to the human; every file is an entry of the catalog in `crates/agent-core/src/prompt.rs`, which names its variables |
| `errgen` | `tools/errgen/` | Go | generates the per-operation error types `agentbus/errors_gen.go` from `agentbus/errors.json` |
| `doccheck` | `tools/doccheck/` | Go | fails when a constant, variable, type or struct field of `agentbus` has no comment |
| the gate | `Makefile`, `tools/gate.sh`, `tools/test-lock.sh` | Make, Bash | what `agent gate` runs, with short output, one test run at a time on the machine |
| tests | `crates/**/tests.rs`, `crates/*/tests/` (fake CLIs and a fake GitHub, no network), `agentbus/*_test.go` | | |

The Bash implementation (`bin/agent`), its rewrite in Go (`agent/`), the terminal panel and the old web interface
(`live/`) and their Bats suite were removed in 0.25.0. Do not bring them back and do not write new Bash or Go for
what the Rust workspace does.

### How the parts talk

The human starts one daemon per `AGENTS_HOME` (`agent daemon`; `config/systemd/agent-daemon.service`). It owns
`state.json`, the history (`history.db`) and every run; the other commands (`agent run`, `status`, `answer`, `stop`,
...) talk to it through `agent.sock`. `agent run` creates a task or a team with a git worktree and a branch for each
task. The daemon builds the prompt of a run from `prompts/`, starts the agent CLI with an MCP configuration that
spawns `agentbus serve` for that run, records what the CLI does in `tasks/<task>/` and acts on the `STATUS:` line of
the agent's final message. The daemon calls `agentbus` subcommands and relies on their output format and exit codes.

## Never

These rules have no exceptions unless the human explicitly asks for it in the task.

The precheck before a round's review holds rules 1, 2 and 6 on the round's own diff: an existing test or check
changed, removed or renamed, production code that looks at whether it runs under test, a change to `AGENTS.md`,
`CLAUDE.md` or `.github/workflows/` sends the round back — only the human allows it (his answer to the check's
question, or an `Allow:` line of the task list where one is possible), and
an `--autopilot` decision is not the human's. A lint suppression (`//nolint`, `#nosec`, `# shellcheck disable=`,
`# noqa`, `# type: ignore`, `@ts-ignore`, `eslint-disable`, `#[allow(...)]`) passes when the PR description names its
file and why; a skipped test belongs in the PR's `## Skipped tests` section, with its reason.

1. **Never modify existing tests.** Do not change, delete, rename, skip or weaken any existing test, test helper
   or fake behaviour (`tests.rs`, `crates/*/tests/**`, `*_test.go`), and do not change expected values to make a test pass. When a
   test fails, the code is wrong until proven otherwise. If the task requires behaviour that an existing test
   forbids, stop and ask the human in your questions file; wait for the answer. **Adding new tests is required**
   (see "Tests"); adding a new action to a fake CLI is allowed, changing an existing one is not.
2. **Never change `AGENTS.md` or `CLAUDE.md`.** Record what you learned in the repository notebook
   (`notebook_append`) and suggest changes to this file in your report.
3. **Never stop, restart or reinstall the running installation.** No `agent stop`, `agent pause`, no stopping or
   restarting of the daemon (`systemctl --user stop|restart agent-*`, signals), no replacing of its binary and no
   edits in its `AGENTS_HOME`. You and the other agents run inside it. Test only against a temporary `AGENTS_HOME`
   of your own — the tests do that for you.
4. **Never run real agent CLIs** unless the task asks for it: they cost the human money and quota.
5. **Never edit `agentbus/errors_gen.go` by hand.** Edit `agentbus/errors.json` and run `make generate`.
6. **Never change `.github/workflows/`.** You do not have permission, and CI runs with secrets. Propose changes
   in the PR description.
7. **Never commit secrets** — tokens, keys, `secrets.env`, real host names or e-mail addresses of the human. Test
   data uses `example.com` and invented names.
8. **Never add a dependency** (a crate, a Go module, a system package) without the human's approval.
   `agentbus` uses the standard library only.
9. **Never break compatibility silently.** These are interfaces with existing installations: the files in
   `AGENTS_HOME` (`state.json` and its schema — [`docs/state.md`](docs/state.md) —, `history.db`, the files of a
   task, the bus's state and messages, checkpoints, notebooks), the output and exit codes of `agentbus` subcommands
   that the daemon parses, the `agent` CLI and the protocol of its socket, the keys of `agents.toml` and
   `secrets.toml`, the MCP tool names and arguments, the log event names. A change
   to any of them needs backward compatibility (old data still readable) or a migration, tests for both, and a
   README update. Say it in the PR description. `agentbus/testdata/state-legacy.json` is a bus state written
   by an earlier version: it must keep loading (`TestLegacyStateLoads`). When you change a persisted type, add a
   fixture of the format before your change and a test that loads it. Remember that stricter validation of an
   old value is a breaking change too: one rejected value makes the whole bus state unreadable.

## Setup

- Rust: `rustup` (no sudo, `~/.cargo/bin` in `PATH`). The first cargo command in the repository installs the toolchain
  of `rust-toolchain.toml` with `rustfmt` and `clippy`; cargo downloads the crates of `Cargo.lock` on first build.
- Go (the version in `agentbus/go.mod` or newer): the tests of the Rust workspace build `agentbus` from its source.
  `bash` ≥ 5, `git`, `make`.
- Linters of `agentbus` and the scripts, pinned and checksum-verified: **`make tools`** installs `golangci-lint`,
  `staticcheck` and `shellcheck` into `~/.local/bin` without sudo. If `make lint` says a linter is missing, run
  `make tools` — do not skip the check and do not install other versions. Missing system packages: install them
  only if the prompt says you have sudo; otherwise report what is missing.

## Workflow and checks

The gate, run **once**, on the commit you hand over (and again on the last commit of every fix):

```sh
make gate
```

**In the worktree of a task, run it as `agent gate`**, not as `make gate`: `agent gate` runs the same gate and records
its result, and the harness opens the pull request only for a branch whose recorded gate passed at its tip on a clean
tree. A gate run in any other way is not recorded, and the round waits.

`make gate` runs the Rust workspace's checks (`make rust`: fmt, clippy, the tests, the docs) with short output: one
line per step, for a failed step only the lines that tell what failed, the full logs in `build/gate/`. When you
need more, read the log with `grep -n` or `tail` — never print a whole log: everything a command prints stays in
your context and is read again at every later step. While you work, run single tests and clippy on the crate you
change, not the gate: a RED commit fails it by design.

**The checks of `agentbus` are not part of the gate.** A round of the Rust workspace does not change `agentbus/` or
`tools/`, so do not run `make gate-full`, `make test`, `make lint` or `make unit-go` for it: they take minutes and
check code you do not touch. A change of `agentbus/` or of the tools — only when the task asks for it — must pass
`make gate-full` as well (the Go lints, `doccheck`, the generated errors, `go test`).

One test run at a time on the machine: every target that runs tests (`gate`, `gate-full`, `test`, `unit-go`,
`rust`) holds a lock (`tools/test-lock.sh`, `/tmp/.make.command.lck`). A busy lock refuses with
`another tests are running now - please wait before you try again` (exit 75) and names who holds it; wait and try
again, or run `GATE_WAIT=1 make gate` to wait in line. Never run the tests around it. A run that died frees the lock at
once; one alive past `TEST_LOCK_STALE_MIN` (10) minutes is taken for hung and stopped.

- A single Rust test: `cargo test -p agent-core <name>`; one crate's lints:
  `cargo clippy -p agent-core --all-targets --all-features --locked -- -D warnings`.
- On a change of `agentbus/` or the tools (`make gate-full` runs all of it): `make lint` — `shellcheck -S warning`
  on the scripts, `gofmt`, `golangci-lint` (configuration: `.golangci.yml`) and `staticcheck` on every Go module,
  `doccheck`, a check that `errors_gen.go` matches `errors.json`, and the Rust gate; `make unit-go` — `go vet` and
  `go test` for `agentbus`; after changing `agentbus/errors.json`: `make generate`.
- All checks must pass with **zero** findings. Fix the cause; a suppression (`#[expect(..., reason = "...")]`,
  `//nolint`, `# shellcheck disable`) carries on the same line why the finding does not apply, and every one is
  named in the PR description.

Do not push code that fails `make gate`. If the gate fails for a reason outside your change (a broken tool, a
flaky test), do not work around it: describe it in your report and ask.

## Go (`agentbus/`, `tools/errgen`, `tools/doccheck`)

`agentbus` is the only Go program left. These rules hold for it; nothing new is written in Go.

### Errors — the model

Every operation returns its **own error type**, so the signature shows what can fail:

```go
func (s *Server) quizStart(question string) (string, *QuizStartError)
```

- `QuizStartError` is `OpError[QuizStartCause]`; `QuizStartCause` is an enum of exactly the failures of
  `quiz_start`, generated from `agentbus/errors.json`. Each cause has a class (`InvalidArgument`, `NotFound`,
  `NotInTeam`, `Permission`, `Conflict`, `InvalidState`, `Storage`, `UnknownTool`, `Process`) that gives the
  sentinel for `errors.Is` and the exit status of CLI subcommands. A `Storage` cause is added automatically.
- Lower layers return concrete types: `*StorageError` (files, stdout), `*ValueError` (invalid values),
  `*ProcessError` (helper processes). An operation converts them explicitly: `fromStorage(QuizStartStorage, serr)`,
  `fromValue(BoardAddInvalidLevel, verr)`.
- State changes go through `transact(bus, <Op>Storage, func(st *State) *<Op>Error {...})`, which turns lock,
  load and save failures into the operation's storage cause.
- Internal helpers that check something return a result enum, not an error (see `requireLead` → `leadCheck`);
  each operation maps the result to its own cause.
- The plain `error` type is allowed **only** where an interface requires it (`json.Unmarshaler`,
  `slog.Handler`, `flag.Value`) and in the MCP dispatcher. Convert with `asError()` (never assign a typed nil
  pointer to an `error`) and say in the comment which concrete type is inside.
- **Never ignore an error.** No `_ = f()`, no unchecked `Close`. Either the error fails the operation, or it is
  a side effect that is logged (`warnSideEffect(effect, err, attrs...)`; `warnOnce` for pollers). Decide which
  and say why in a comment when it is not obvious.
- Messages are for agents and humans: what went wrong and, where possible, what to do instead.
- "Not an error" results stay results: `TAKEN`, `QUEUE FULL` and similar answers are returned as output, not
  as errors.

### Types

- Use the type system to make invalid states impossible. Statuses, states, tiers, levels, kinds, roles and log
  event names are **typed constants** (`agentbus/types.go`, `events.go`, `logging.go`), never string literals
  in logic. A new enum gets `Valid()`, is parsed with `parseEnum`, and rejects unknown values in
  `UnmarshalJSON` (with a `*ValueError`).
- Times are `Timestamp`; its zero value means "has not happened yet".
- Every bus message is an event type implementing `busEvent` (`Kind()`, `Text()`) in `agentbus/events.go`;
  post it with `bus.post(st, from, to, event)`. Do not build message texts at the call site.
- Prefer small structs with named fields to multiple return values of the same type.

### Comments and naming

- **Every constant, variable, type and struct field has a comment** that tells a reader what it is for
  (`doccheck` enforces it; generated code gets its comments from `errors.json`). Exported identifiers: the
  comment starts with the name.
- Comments explain *why* and the contract (units, ownership, what nil/zero means, what can fail), not what the
  next line obviously does.
- English everywhere: identifiers, comments, messages, logs, tests, commit messages.

### Logging

- `log/slog` through `logInfo` / `logWarn` / `logError` with a `LogEvent` constant (snake_case, stable: dashboards
  and alerts use the names). New events are added to the `LogEvent` list with a comment.
- Records share one schema with the daemon ([`docs/log.md`](docs/log.md)): `ts`, `level`, `component`, `event` plus fields. No
  `fmt.Print*` for diagnostics; stdout of subcommands is a parsed interface (use `cliWriter` / `printJSON`).

### Style

- `gofmt`; small functions with one job; early returns; no global mutable state beyond what exists (and when
  you must, document who owns it).
- Concurrency in `agentbus` is between processes: all shared state is read and written inside `transact` /
  `locked`. Never wait or sleep while holding the lock; poll outside it (see `reserve`, `waitForMessage`).
- Keep `agentbus` free of third-party dependencies.

### Adding an MCP tool (checklist)

1. Add the operation and its causes to `agentbus/errors.json`; `make generate`.
2. Declare the tool and its argument types in the tool list (`toolSchemas` / `*ToolDefs`) and add the handler
   returning `(string, *<Op>Error)`; dispatch it in `callTool` with `toolResult(...)`.
3. Bus events for it in `events.go`; log events in `logging.go` if needed.
4. Tests: every cause (`failsWith(t, <Op><Cause>)(...)`), the success path, and the dispatcher.
5. Document it in the README ("MCP tools available to agents") and, if agents should use it, in the prompts.

## Shell scripts (`tools/`, `examples/`)

Only the gate's own scripts and the examples are Bash; the harness is not. No new script for what the Rust
workspace can do.

- `#!/usr/bin/env bash` with `set -euo pipefail`; `shellcheck -S warning` clean with the pinned version.
- Quote every expansion; use arrays for argument lists; `local` for every function variable; a one-line comment
  above every function saying what it does and its arguments.
- **No silent errors.** No bare `|| true`, `2>/dev/null` or `|| :`; when a non-zero exit is an expected answer
  (e.g. `grep` finding nothing), handle that code explicitly and comment it.

## Rust (`Cargo.toml`, `crates/`)

The harness is a Cargo workspace. Read [`docs/architecture.md`](docs/architecture.md) before you change anything
under `crates/`: its layout, its process model and the rules every item of the plan (`rust-agent-plan.md`,
`rounds/`) follows.

- **Rules:** each role is given its own rules by the harness — the developer rules to write Rust, the review rules to
  review it — cited by rule ID (ERR-2, PR-1, ...). For code under `crates/` they take the place of the Go section of this
  file; the rules of "Never", "Tests", "Commits and pull requests" and "Definition of done" still hold.
- **Size of a PR of the Rust plan:** PR-1 and PR-2 as written, with no relaxation: a PR is planned for at most
  **1500** changed lines, tests not counted, and never passes **2000** — the 500 between them are a reserve for what
  the estimate missed, not room to plan into. Lines are counted with the PR-1 command, not `agent round-size`. For
  these PRs this replaces the 3000 lines of "Rounds". A task estimated above 1500 is split before coding, into
  tasks of their own.
- **Check:** `make rust` — `cargo fmt --check`, clippy with `-D warnings`, the tests and `cargo doc` with warnings
  denied, all `--locked`. `make gate` runs it, and nothing else; the existing CI workflow runs
  `make lint`, including the Rust gate. The lint baseline is `[workspace.lints]` in `Cargo.toml`; a lint is suppressed only with `#[expect(..., reason = "...")]` on the smallest
  item, listed in the PR.
- **Tests first** as in [`docs/tdd.md`](docs/tdd.md) and the developer rules (TDD-1, TDD-14): the RED commit's bodies
  are compiling stubs that return a wrong value, never `todo!()`. A test is frozen once its RED commit exists: later
  commits add tests, and a wrong test is corrected only by a `TEST-FIX:` commit of its own, listed in the PR under
  `## Tests changed after RED`. The unit tests of a new module go into its `tests.rs` (TDD-10).
- **Never the live installation:** code runs and is tested against its own temporary `AGENTS_HOME`, never the one of
  the installation you run in.

## Prompts (`prompts/`)

- English; a translation of a text the human reads goes under `prompts/lang/<language>/`. Every file is an entry of
  the catalog (`PromptId` in `crates/agent-core/src/prompt.rs`), which names who reads it and the `{{variables}}` it
  may use; a test checks that every entry exists and renders with them. A new text is a new entry — never a text
  built in the code.
- Keep the protocol consistent across the texts of one audience (`harness/team/`, `harness/review/`,
  `harness/run/`, ...): when you change a rule in one, check the others, and the golden files of the tests.
- Prompts are instructions for other models: short, unambiguous, imperative; say what to do when something
  fails.

## Tests

- **Tests come first** (`docs/tdd.md`): RED (the test, failing, committed with its failing output) → GREEN (the
  simplest code) → REFACTOR (tests unchanged). Where that is not possible, the commit says
  `Test-First: not possible — <reason>`. A bug fix starts with a test that fails without the fix. Remember rule 1:
  existing tests are not yours to change. The one exception: the round that implements the
  functionality of a skipped placeholder test replaces that placeholder with a real test.
- Go: check errors by their exact cause (`failsWith(t, BoardClaimAlreadyTaken)(...)`, `failure(...)` when you need
  the error, `expectToolError` for dispatcher errors) or with `errors.Is` / `errors.As` — never by comparing
  message texts (the helper `missing(...)` is the one message that is part of a contract). Use `newBus(t)`
  (temporary state directory), `server(...)`, `createTeam(...)`, `captureLogs(t)`.
- Rust: as the developer rules say (TDD-*): unit tests in the module's `tests.rs`, tests of a whole process in
  `crates/*/tests/` against a temporary `AGENTS_HOME` with the fake CLIs and the fake GitHub of the fixtures; wait
  for a state, never with a bare sleep; assert on the state, the history, the files and git, not on timing.
- Tests are deterministic, need no network and no real accounts, and clean up after themselves.

## Documentation

- Every user-visible change updates the README in the same PR: commands, configuration, files, MCP tools, log
  events, exit codes, security implications.
- `config/agents.example.toml` is the template of the configuration and [`docs/configuration.md`](docs/configuration.md)
  its reference: keep them in sync with the code (a test reads the template).
- Write for someone who installs team-agents for the first time.

## Commits and pull requests

- The orchestrator sets the commit author and gives you the trailers to add; do not change git identity.
- Small commits, each building and passing the checks — except the RED commit of a test-first cycle, which fails
  only on its new tests. Subject in the imperative mood, at most 72 characters
  (`Add board_reassign tool`, `Fix lost wake-up after a crash`); the body says why, and what the reader cannot
  see in the diff.
- The PR description: what and why, how it was tested (commands and results), compatibility notes (rule 9),
  every lint suppression, and anything the reviewer should look at first.
- **Say what the new version brings.** A PR that raises the version adds its entry at the top of
  [`WhatsNew.md`](WhatsNew.md), in the commit that raises the version: a heading with the version, the date and the
  PR, then its lines under `### New features`, `### Bug fixes` and `### Enhancements` — only the headings that have
  something under them. Every line starts with the tags of the crates or modules whose code the change is in, so
  that the reader knows where to look: `[agent]`, `[agent-core]`, `[agent-runtime]`, `[agent-web]`, `[agentbus]` —
  one tag per crate, in this order, no other tags (a change of the prompts or of the workspace's `Cargo.toml` alone
  carries the tag of the crate whose behaviour it changes). Write simply and briefly, for the human who restarts
  the daemon and wants to know what is different now: what the daemon or the CLI does that it did not do before, in
  plain words — no rule IDs, no names of functions or files, no account of how it was built; one line per change,
  and where it helps one example of a command, a line of output or a configuration key. A new feature is something
  the human can now do or see; a bug fix says what went wrong before; an enhancement makes something that existed
  better. A change the human must act on (a new key, a changed command, a migration) starts with **Action:** after
  its tags. The example to follow:

  ```markdown
  ## 0.17.0 — 19.11.2026 — PR #131

  ### Bug fixes
  - [agent] A message sent to a lead during a fix pass no longer loses the lead's fix response.
  - [agent] [agent-core] A lead is woken at most twice for the same message, also after a restart of the daemon.
  ```

## Rounds

A PR is planned for at most **1500** changed lines, tests not counted, and never passes **2000** (PR-1, PR-2; section
"Rust"). A bigger task is split before coding and planned as rounds — see the "Rounds" part of your prompt. After
every round `main` builds and passes `make gate`; unfinished work is dead code or a stub returning a typed "not
implemented" error; no test is written ahead of its round, and a skip is an exception with a reason outside the
code, listed in the PR's `## Skipped tests` section (`docs/tdd.md`, "Skipped tests").

## Definition of done

- [ ] `make gate` passes with zero findings, on the commit handed over.
- [ ] New behaviour has tests, written first (or a `Test-First: not possible` reason); no existing test was changed.
- [ ] The PR is within the limit of its size (PR-1); every skip it adds is listed with its reason in the PR's
      `## Skipped tests` section.
- [ ] Errors are typed per operation; nothing is ignored; enums instead of string literals; every declaration
      has a comment.
- [ ] README, `config/agents.example.toml`, `docs/` and prompts updated where the change is visible to users or agents.
- [ ] `WhatsNew.md` says, simply and briefly, what the new version brings ("Commits and pull requests").
- [ ] Compatibility of on-disk formats, subcommand output and exit codes kept (or migrated, tested and
      documented).
- [ ] The report and the PR description explain what was done, why, and what is left.
