# The Rust `agent` — architecture

`bin/agent` (Bash) is being rewritten in Rust, as a Cargo workspace in this repository, item by item of the plan
"Rewrite `bin/agent` in Rust". This file describes the workspace's layout, its process model and the rules every
item of the plan follows. Read it before you change anything under `crates/`; [`AGENTS.md`](../AGENTS.md) ("Rust")
points here.

Until the cutover (feature 10 of the plan) the old implementation runs the installation and the new one runs
nowhere but in tests and trials.

## Layout

```text
Cargo.toml                  the workspace: members, shared package fields, dependencies, lints
Cargo.lock                  committed; every cargo command of the gate runs --locked
rust-toolchain.toml         the pinned stable toolchain with rustfmt and clippy (rustup installs it)
rustfmt.toml, clippy.toml   formatting and the clippy settings of the lint baseline
crates/agent-core/          pure: identifiers, configuration, state, task list, decisions, protocol types
crates/agent-runtime/       effects: state store, processes, git, GitHub API, agentbus, prompts, notifications
crates/agent/               the `agent` binary: CLI commands and `agent daemon`
crates/agent-web/           the `agent-web` binary: dashboard (axum, htmx, server-sent events)
prompts/clarification.md
prompts/agent/rust-developer.md
prompts/worker-agent/rust-developer.md
prompts/investigator/rust-developer.md
prompts/investigator/code-review.md
prompts/harness/<module>/<name>.md     texts the harness composes (run prompts, questions files, notifications)
config/agents.example.toml, config/systemd/
docs/architecture.md, docs/configuration.md, docs/state.md, docs/task-list.md
```

The workspace exists with its four crates; the other files of this list arrive with the items that need them
(`docs/state.md` came with the state, the protocol and the questions files).

- **`agent-core`** decides and does no I/O: no `std::fs`, `std::process`, `std::net`, no `tokio`. What it needs from
  the world comes in as arguments — the clock ([`Clock`](../crates/agent-core/src/ids.rs)) and the random source
  (`rand::Rng`) included — so every decision is a pure function, tested without a file system.
- **`agent-runtime`** carries the decisions out: files, processes, git, the GitHub API, agentbus, prompts and
  notifications. It depends on `agent-core`, never the other way round.
- **`agent`** and **`agent-web`** are thin binaries on top: command lines, the daemon, the dashboard.

### The vocabulary (`agent-core`)

| Module | Types |
|---|---|
| `ids` | `AgentName`, `InvestigatorName` (normal and worker agents, investigators), `TeamId`, `TaskId`, `RunId` (`team-`/`task-`/`run-YYYYMMDD-HHMMSS-xxxx`, UTC, generated from an injected `Clock` and `Rng`), `ItemId` (`2.3`), `BranchName` (literal git branch names, with the bounds below) |
| `kinds` | `CliKind` (`claude`, `codex`, `gemini`, `opencode`), `Tier` (`normal`, `worker`, `investigator`), `Role` (`solo`, `coordinator`, `member`, `worker`, `reviewer`) |
| `status` | `RunOutcome` (`DONE`, `QUESTIONS`, `FAILED`, `SLEEPING`, `EXCLUDED`), `ReviewVerdict` (`APPROVE`, `REQUEST_CHANGES`), `TaskStatus` and `ItemStatus` with their transition tables |

Each is made only through its check (`FromStr`, `TryFrom<String>`, serde), its documentation lists what it refuses,
and its text form parses back to an equal value. Branch names follow Git's literal ref rules, with a
255-byte limit and an explicit refusal of `@` to avoid its revision shorthand; checkout-history expressions are
not expanded. A status changes only through `StatusMachine::transition_to`, which
refuses a change its table does not allow with a typed `InvalidTransition`:

```text
task:  queued -> running | paused | stopped
       running -> queued | waiting | questions | paused | stopped | failed | done
       waiting -> queued | paused | stopped | done
       questions, paused, failed -> queued | stopped
       stopped -> queued
       done (final)

item:  pending -> in_progress -> in_review -> approved -> done (final)
       in_progress -> pending (its member left)    in_review -> in_progress (a fix pass)
```

### The configuration and the task list (`agent-core`)

| Module | Types |
|---|---|
| `config` | `Config` (read from `agents.toml`) with its `AgentConfig`, `InvestigatorConfig`, `RepoConfig` and sections; `Secrets` (from `secrets.toml`) with `Secret`; the typed values (`Language`, `Program`, `AbsolutePath`, `CommandLine`, `Minutes`, `LoopbackAddr`, ...); `ConfigError`, `SecretsError`, `InvalidValue`. See [`configuration.md`](configuration.md) |
| `task_list` | `TaskList`, `Feature`, `Item` (title, description, expected result, checks) and `TaskListError`, whose every variant but two carries its line. See [`task-list.md`](task-list.md) |

Both take text, not paths: reading `agents.toml`, `secrets.toml` (and the file mode `Secrets::parse` needs) and a
`-f` task list is the runtime's job, so the core stays free of I/O.

### State, protocol and questions files (`agent-core`)

| Module | Types |
|---|---|
| `state` | `State` (with its schema version), `Team`, `Crew`, `Member`, `Participant`, `Run`, `RunEnd`, `Question`, `ItemProgress`, `ItemPhase`, `PullRequest`, `Review`, `FixPass`, `GateResult`, `Timestamp`, `SessionId`, `CommitId`, `PrNumber`. See [`state.md`](state.md) |
| `protocol` | `Request`, `Command`, `Response`, `Reply`, `Failure`, `Event`, `EventKind`, `Frame`, with the protocol version and `ProtocolError`. JSON lines over the daemon's socket. See [`state.md`](state.md) |
| `questions` | `QuestionsFile`, `Entry`, `Verdict`: parsing and rendering of the questions file, the `ok` verdict, the `**ANSWERED**` marker. See [`state.md`](state.md) |

They take and give text too (`State::from_json`, `Request::from_line`, `QuestionsFile::parse`): the state store, the
socket and the files are the runtime's. `serde_json` is a dependency of `agent-core` for that; it does no I/O.

## Process model

`agent daemon`, a systemd user service, is the only owner of the state and of every model run. Every other command
— `agent run`, `answer`, `status`, `stop`, `gate`, ... — and the dashboard `agent-web` are clients of its Unix
socket. One owner, no file races (CON-3).

- `agent run <agents...> "tasks" | -f tasks.md [--coordinator NAME] [--no-merge]`: one agent is a solo task, more
  than one is a team. `--autopilot` is the default: PR → two reviews → merge → next item; `--no-merge` stops before
  the merge and hands the PR to the human.
- The coordinator: an explicit `Coordinator: NAME` line at the top of the task list or `--coordinator` wins;
  otherwise an investigator in the team (randomly among two or more); otherwise a random normal agent. Workers never
  coordinate. An investigator coordinating weaker models leads them: explains, shows the way, and writes or fixes
  code itself when needed.
- The daemon assigns items, one to each normal member at a time. Workers do not take items: they serve `delegate`
  requests, the first free worker in the configured order.
- The coordinator integrates a member's branch, gets the gate green and opens the PR. The next item of a feature
  starts when the previous one is merged (with `--no-merge`: approved by both reviews).
- Every PR gets two reviews from investigators outside the team. A team that leaves fewer than two configured
  investigators outside it is refused at start: "too many investigators in the team". "Changes requested" goes back
  to the team at most 5 times; after that the human is asked.

## Rules of the plan

These hold for every item of the plan, on top of the Rust developer rules (`prompts/agent/rust-developer.md`) and
the review rules (`prompts/investigator/code-review.md`), whose rule IDs (ERR-2, PR-1, ...) they cite. Neither rule file is tracked in this round. The investigation used the human-provided developer rules and
review rules from the main checkout, where the review file is named `rust-review.md`; the requested
`code-review.md` name is absent. Publishing the prompt catalog belongs to item 2.3.

- **Size, relaxed for this plan:** PR-1's limit is 5000 changed lines per PR, tests not counted, and PR-2's threshold
  for planning a split is about 4000. Everything else in PR-1 and PR-2 applies as written: how lines are counted, a
  ceiling not a target, never fewer tests because of it. An item that would exceed the limit is split before coding
  and the split is recorded in the PR.
- **One item, one PR.** The PR description quotes the item in full under `## Task`; the reviewers check the PR
  against it and run its checks. A file outside the item's expected list, an expected part left out, or a check that
  cannot pass yet needs its reason in the PR description. Each round starts from its own file,
  `rounds/round-N.M.md` ([`rounds/round-1.1.md`](../rounds/round-1.1.md), [`rounds/round-2.1.md`](../rounds/round-2.1.md)).
- **Every item stands alone:** it compiles and passes the whole gate on `main` without the other items of its
  feature, and uses only code from the features its feature's "Depends on" line names, all of them merged. Items of
  one feature are taken in order; features whose dependencies are merged run in parallel.
- **The old implementation is a reference only.** `bin/agent` (Bash), `agent/` (the unfinished Go version) and
  `live/` (the old dashboard) show how it worked. Nothing is ported line by line, linked, called or kept compatible:
  no shared files, no shared state format, no shared commands. `agentbus/` (Go) stays and is used as it is.
- **No prompt text in `.rs` files, ever.** Everything a model reads, and everything written into a file a human
  answers (questions-file instructions, notification texts), lives in `prompts/` as a template with `{{name}}`
  variables. Error messages of the CLI and log events stay in code.
- **Async with `tokio`.** No external process except the model CLIs (`claude`, `codex`, `agy`, `opencode`), `git`,
  `agentbus`, the repository's own gate command, the configured notification command and the configured editor.
  Everything the Bash version did with `sed`, `awk`, `jq`, `grep`, `flock`, `gh` or `tailscale` is done natively.
- **Approved dependencies:** `tokio`, `tokio-util`, `clap`, `serde`, `serde_json`, `toml`, `thiserror`, `tracing`,
  `tracing-subscriber`, `tempfile`, `rustix`, `reqwest` (rustls), `axum`, `notify`, `rand`, `time`. Any other needs
  a DEP-1 justification. Each is declared once, in `[workspace.dependencies]`, with its minimal features.
- **A separate `AGENTS_HOME`.** New code is developed and tested against its own `AGENTS_HOME`; nothing touches the
  live `~/agents` of the running Bash harness until feature 10.
- English everywhere: code, comments, docs, commit messages, PR descriptions.

Out of scope for the plan: remote nodes and the lead, investigation teams with phases and standalone investigation
requests, `agent live`, archiving, token usage and cost accounting, the precheck rules (B1–B4), automatic PR size
limits (reviewers check PR-1), an `agentbus` socket API (it is called as a process).

## The gate

```sh
make rust     # the workspace alone
make gate     # the whole repository; its rust step is make rust
```

`make rust` runs, all `--locked`:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked          # doctests included, under the machine's test lock
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked
```

It is a step of `make gate`, `make test` and `make lint`. The existing CI workflow
([`.github/workflows/test.yml`](../.github/workflows/test.yml)) runs `make lint`, so it also runs the complete Rust
gate. The Ubuntu runner supplies rustup; the first cargo command installs the pinned toolchain and its components.
No workflow edit is needed. This also avoids requiring the investigation token to have the `workflow` scope.

The lints are the baseline of the developer rules' appendix, in `[workspace.lints]` of `Cargo.toml`; every crate has
`[lints] workspace = true`. They are `warn` there and fatal through `-D warnings` in the gate. A lint is suppressed
only with `#[expect(lint, reason = "...")]` on the smallest item (LINT-3), and every suppression is listed in the PR.

## Tests first, in Rust

The cycle of [`docs/tdd.md`](tdd.md) holds: the RED commit has the tests and fails only on them, the GREEN commit
the simplest code that passes. In Rust the RED commit's bodies are compiling stubs that return a wrong value (accept
everything, refuse everything, an empty table) — never `todo!()`, which aborts the test instead of failing it and is
never committed (TDD-1). A guarantee a type claims — a private constructor, two types that do not mix — gets a
`compile_fail` doctest; stable rustdoc does not check the error code named in it, so the snippet is checked by hand
to fail for that reason.
