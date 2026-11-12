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
prompts/lang/<language>/...             human-facing texts in another language (English is the fallback)
prompts/agent/rust-developer.md
prompts/worker-agent/rust-developer.md
prompts/investigator/rust-developer.md
prompts/investigator/code-review.md
prompts/harness/<module>/<name>.md     texts the harness composes (run prompts, questions files, notifications)
config/agents.example.toml, config/systemd/agent-daemon.service
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

Both `Config` and `TaskList` take text, not paths: reading `agents.toml`, `secrets.toml` (and the file mode `Secrets::parse` needs) and a
`-f` task list is the runtime's job, so the core stays free of I/O.

### State, protocol and questions files (`agent-core`)

| Module | Types |
|---|---|
| `state` | `State` (with its schema version), `Team`, `Crew`, `Member`, `Participant`, `Run`, `RunEnd`, `Question`, `ItemProgress`, `ItemPhase`, `PullRequest`, `Review`, `FixPass`, `GateResult`, `Timestamp`, `SessionId`, `CommitId`, `PrNumber`. See [`state.md`](state.md) |
| `protocol` | `Request`, `Command`, `Response`, `Reply`, `Failure`, `Event`, `EventKind`, `Frame`, with the protocol version and `ProtocolError`. JSON lines over the daemon's socket. See [`state.md`](state.md) |
| `questions` | `QuestionsFile`, `Entry`, `Verdict`: parsing and rendering of the questions file, the `ok` verdict, the `**ANSWERED**` marker. See [`state.md`](state.md) |

They take and give text too (`State::from_json`, `Request::from_line`, `QuestionsFile::parse`): the state store, the
socket and the files are the runtime's. `serde_json` is a dependency of `agent-core` for that; it does no I/O.
`strum` (derives only) gives the keyword enums their text, as `McpServerKind` does.

### The prompt catalog and the CLI contract (`agent-core`)

| Module | Types |
|---|---|
| `prompt` | `PromptId` (every prompt, with its `PromptSpec`: path under `prompts/`, `Audience`, required variables), `Template` (the strict `{{name}}` renderer), `Variables`, `VariableName` and their errors (`TemplateError`, `RenderError`, `PromptError`) |
| `cli` | `CliAdapter` (build a run, make a parser), `OutputParser` (lines to `ActivityEvent`s, then `RunOutput`), `RunRequest`, `RunSpec`, `RunFile`, `ActivityEvent`, `RunOutput`, `SessionId`, `ErrorTail`, `McpServerKind` |

A template renders `{{name}}`; `\{{` is a literal `{{`. A variable without a value, a value without a variable, an
unclosed placeholder and an invalid name are errors, and the catalog adds two: a text that uses a variable the
catalog does not list, and one that leaves a listed variable out. The text of a prompt is read by the runtime;
`PromptSpec::candidates` gives the files to try. Only a text for the human (`Audience::Human`) has a version per
language, `prompts/lang/<language, lowercase>/<path>`, with English as the fallback; what a model reads is English
only. The integration test `tests/prompt_catalog.rs` checks every catalog entry against the files of `prompts/`, and
its failures name the file.

An adapter is pure: `CliAdapter::build_run` turns a `RunRequest` into a `RunSpec` (program, arguments, environment,
the files to write first, what to keep out of git), the runtime starts it, feeds the CLI's stdout line by line into
an `OutputParser` and its stderr at the end, and gets the final message, the session ID, how the stream says the run
ended and the error tail.

### The adapters (`agent-runtime`)

`agent_runtime::cli` has one `CliAdapter` per CLI — `claude::ClaudeAdapter`, `codex::CodexAdapter`,
`agy::AgyAdapter` (`CliKind::Gemini`), `opencode::OpencodeAdapter` — and `adapter_for(CliKind)`, which the run engine
uses to pick one from the configuration. The module's documentation lists the rules all four follow; the ones the run
engine relies on:

| Per CLI | New run | Resumed run | MCP servers | Final message |
|---|---|---|---|---|
| Claude Code | `claude -p --output-format stream-json --verbose … --session-id ID` | `--resume ID` | `mcp.json` and `claude-settings.json` in the run's directory | the `result` event |
| Codex | `codex exec --json … -C DIR -c mcp_servers.…` | `… resume ID PROMPT` | `-c` settings (TOML) | the last message of a completed turn |
| Antigravity | `agy --output-format stream-json … --print-timeout 8h -p PROMPT` | `--conversation ID` | `.agents/mcp_config.json` in the worktree, kept out of git | the `result` event |
| opencode | `opencode run --standalone --format json --auto … -- PROMPT` | `--session ID` | `OPENCODE_CONFIG_CONTENT` (and a copy in the run's directory) | the last `text` |

- The request carries each MCP server's whole command line; the adapter adds only Serena's `--context <client>`
  (`claude-code`, `codex`, `antigravity`, `ide`). A server that is not in the request is not configured.
- The error tail is the CLI's stderr followed by the lines of the stream that report an error: the classifier of run
  outcomes reads only the tail, and opencode reports a provider's refusal on stdout.
- The quirk settings are constants of the adapters: Claude's turn limit (300) and Bash timeout (20 minutes), `agy`'s
  print timeout (8 hours), opencode's Bash timeout (10 minutes). Other flags, such as `--effort high`, are the
  agent's `args`.
- Not in the adapters, because they are not pure: laying down a `RunFile` (creating its directories), keeping
  `worktree_excludes` out of git, a prompt too long for one command-line argument, and opencode's `session export`
  for a resumed session whose result it did not print. They come with the run engine (6.1).
- Recorded output of each CLI is in `crates/agent-runtime/tests/fixtures/cli/`; its README says which files were
  recorded and which were written by hand.

### The decisions (`agent-core`)

| Module | What it decides |
|---|---|
| `decide::team` | `plan_team`: from the agents named, the configuration, the coordinator asked for (`--coordinator`, then the task list's `Coordinator:` line) and a random source, a `TeamPlan` — solo, or a team with its coordinator, members and workers — or a `TeamPlanError`: unknown agent, a worker as coordinator, a coordinator that was not named, only workers, no member besides the coordinator, and too many (or too few) investigators for two outside reviewers |
| `decide::assign` | `next_assignments`: which item goes to which member next — one item per member, the items of a feature in order (the next starts when the previous is merged, with `--no-merge` approved), features in parallel once the features they depend on are merged; `first_free_worker` for `delegate` |
| `decide::round` | `RoundState`, `RoundEvent` and `RoundState::transition`: member work, integration, gate green at the head, pull request published for that head, two reviews, merge or hand-over to the human, item done; "changes requested" starts a fix pass until `[review] fix_passes`, then the human is asked, and the answer grants one more pass. Each transition says what the daemon does next (`RoundAction`) |
| `decide::outcome` | `classify_outcome`: from the final message, the exit status, the error tail and who ended the run, the `RunEnd` to record — reported status, no status, quota exhausted (with the reset time when the CLI gives one), rate limited, hung, stopped — and the `Pause` it implies; `after_hang` for the watchdog's strikes. The CLIs' outputs are fixtures in `crates/agent-core/tests/fixtures/cli-outputs/` |

The daemon feeds these functions what it observed and carries out what they return; none of them touches a file, a process or
the network. The order of the checks and the rules each follows are in the documentation of its module.

### The run engine and the daemon (`agent-runtime`, `agent`)

`agent daemon` is the process model below, working: one daemon per home, the Unix socket, the run engine, recovery.

| Where | What |
|---|---|
| `agent_runtime::run` | `RunEngine::run`: one run of a model CLI, from its files to its classified end. Touches no state |
| `agent_runtime::run::compose` | the prompt of a run: context, role, rules of the tier, brief, item, checkpoint — or, for a resumed session, what interrupted it |
| `agent_runtime::{settings, harness_prompts, clock}` | `agents.toml` and `secrets.toml` read from the home; the second prompt catalog (`HarnessPrompt`, `prompts/harness/run/`); `SystemClock` and `SharedClock` |
| `agent::daemon` | `owner` (the state), `server` (the socket), `notifier`, `settle` (what a run's end does), `recovery` (on start) |
| `agent::{client, commands}` | the socket client, `agent status`, `agent daemon` |

**Tasks of the daemon** (CON-3, CON-4): the *owner* is the only task that reads or changes the state; the *server* passes
requests to it through a bounded channel; the *notifier* delivers notifications off its path. Cancelling the shutdown
token (SIGTERM, SIGINT) stops them in order: the server stops accepting, the owner has its runs stopped and records them,
the notifier delivers what is queued, the socket is removed. Every spawned task is joined. A state that cannot be saved
ends the daemon with an error — models must not go on working with a state that is not kept — and `Restart=on-failure`
of the unit starts it again.

**Start.** `agent daemon` needs `AGENTS_HOME` (no default before the cutover: a trial cannot touch the live home by
accident), `agents.toml` there and the prompt files (`prompts/` in the home, or `--prompts-dir DIR`); every file of both
catalogs is checked at start and every problem reported at once. It takes the lock of the home (a second daemon exits with
"already running"), reads the state, recovers it, makes the socket (in a private directory, mode `0600`, renamed into
place) and starts serving. A trial:

```sh
export AGENTS_HOME=$(mktemp -d)
cp config/agents.example.toml "$AGENTS_HOME/agents.toml"
agent daemon --prompts-dir "$PWD/prompts" &    # logs on stderr
agent status                                   # no teams
agent daemon --prompts-dir "$PWD/prompts"      # agent: the agent daemon is already running: ...
kill -TERM %1                                  # ends the daemon and its runs
```

**What the daemon does with a task.** A task whose status is `queued` is started: the owner makes the run ready
(`start.rs`: the agent's CLI and model from the configuration, the worktree `worktrees/<repo>/<task>` of the home, the
prompt, the MCP servers — `agentbus serve` for the task, Serena when configured), records the run, sets the task `running`
and gives the job to the engine. Creating the worktree and queueing tasks is the next item's (`agent run`); until then a
test or a trial writes them. When the engine returns, the end of the run decides (`settle.rs`):

| The run ended | The task goes to | And |
|---|---|---|
| `STATUS: DONE` | `done` for a solo task and a review, `waiting` for a coordinator, member or worker | |
| `STATUS: QUESTIONS` | `questions` when `tasks/<task>/questions-<round>.md` is there and parses | the round is recorded and the human notified; a missing or unreadable file fails the task and says why |
| `STATUS: FAILED`, no `STATUS:` line | `failed` | the human is notified |
| `STATUS: SLEEPING` / `EXCLUDED` | `waiting` / `done` | |
| quota exhausted, rate limited | `paused` | woken at the time the CLI gave (plus a minute), else after `[limits]` minutes; the human is notified |
| hung (the watchdog killed it) | `queued` again, resumed from its session | after `[watchdog] hang_strikes` hangs in a row without progress: `failed`, the human notified. A run that did something before it went silent starts the count again |
| the daemon shut down | `queued` | the run is recorded as ended without a status and no exit code |

**The watchdog** is the engine's: a run whose standard output and error files show no change for `[watchdog] hang_minutes`
is stopped (its whole process group: `SIGTERM`, then `SIGKILL`), and its end is classified as a hang unless the provider
refused it (then it is a limit). **Pauses** are kept in no field of the state: the wake-up time is worked out again from
the last run's end, so a pause survives a restart. The daemon looks at the clock every 5 seconds (not a long sleep: a
suspended machine does not stretch a pause).

**Recovery on start.** A run that was going (the daemon died, or was killed) is ended as "no status, no exit code" and its
task queued; it resumes from its session with the text for an interrupted harness. A paused task gets its wake-up again.
A clock that was set back never ends a run before it began.

**The socket** serves `status`, `show` and `subscribe` of the protocol of [`state.md`](state.md); the other commands answer
`refused` naming themselves until their items. A line that is not a request is answered with `invalid` and the connection
goes on; a line over 2 MiB ends its connection. At most 64 connections are served at once, 16 runs go at once, and a
subscriber that falls 1024 events behind is let go (it subscribes again and starts from the state).

**The home** (`agent_runtime::store::Home`):

| Path | What |
|---|---|
| `agents.toml`, `secrets.toml`, `prompts/` | the settings and the prompt files, put there by the human |
| `state.json`, `daemon.lock`, `agent.sock` | the state, the lock that keeps one daemon per home, the socket |
| `tasks/<task>/activity.jsonl` | what each run did, one JSON line an event |
| `tasks/<task>/runs/<run>/` | the output of a run (`out.jsonl`, `err.log`), the files its CLI is configured with, `prompt.md` for a prompt too long for a command line |
| `tasks/<task>/questions-<round>.md` | the questions of a round |
| `worktrees/<repo>/<task>/` | the git worktree of a task |
| `notifications/` | the notification files |

Not done here (items of the plan): `agent run` and the team's life (7.1), questions answered and the controls (7.2), the
gate (7.3), pull requests and reviews (8.1), the dashboard (9.1). The checkpoint of a resumed agent is read from the
agentbus, which a team's life creates; until then a resumed run has none. opencode's `session export` for a resumed
session whose result it did not print is not done: such a run is classified from what it printed.

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

These hold for every item of the plan, on top of the rules each role is given — the developer rules to write Rust, the
review rules to review it — whose rule IDs (ERR-2, PR-1, ...) they cite. Both are in the prompt catalog (`PromptId`);
the developer rules also come in a variant for workers (a delegated task instead of a pull request).

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
  `tracing-subscriber`, `tempfile`, `rustix`, `reqwest` (rustls), `axum`, `notify`, `rand`, `time`, `strum` (`derive`). Any other needs
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
