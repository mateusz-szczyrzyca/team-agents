Rewrite `bin/agent` in Rust — task list

This file is the plan for a new `agent`, written from scratch in Rust in this repository. The text before the first
feature is the brief every member reads; each item below is one task, one branch and one pull request.

Ground rules

- Each role is given its own rules (the developer rules to write code, the review rules to review it); they are
  binding, and their rule IDs (ERR-2, PR-1, ...) are the ones this plan cites.
- Size, relaxed for this plan: the new `agent` is needed fast, so PR-1's limit is 5000 changed lines per PR, tests
  not counted, and PR-2's threshold for planning a split is about 4000. Everything else in PR-1 and PR-2 applies as
  written: how lines are counted, a ceiling not a target, never fewer tests because of it. Task 1.1 records this in
  `AGENTS.md`, which the reviewers read first. An item that would exceed the limit is split before coding and the
  split is recorded in the PR.
- Every item ends with its expected result (the files it creates or changes, what it adds, its tests, and why)
  and its checks (what anyone can run or look at to see that it works, and what they should see). The PR
  description quotes the item in full under `## Task`; the reviewers check the PR against it and run its checks.
  A file outside the expected list, an expected part left out, or a check that cannot pass yet needs its reason in
  the PR description.
- Each round is started with its own file, `rounds/round-N.M.md`: its `Round:` line, this round's item in full, these
  rules, and an overview of the other items, which are context only.
- Every item compiles and passes the gate on `main` without the other items of its feature. The gate is the
  repository's gate command and nothing more: here the Rust workspace's checks. The suites of the old
  implementation (its Bats and Go tests) are never run by a round, and a failure in them is not the round's
  business. An item uses only code from the features named in its feature's "Depends on" line, all of them merged.
  Items of one feature are taken in order; features whose dependencies are merged run in parallel.
- The old implementation — `bin/agent` (Bash), `agent/` (unfinished Go), `live/` (old dashboard) — is a reference
  for "how it worked" only. Nothing is ported line by line, linked, called or kept compatible: no shared files, no
  shared state format, no shared commands. `agentbus/` (Go) stays and is used as it is.
- English everywhere: code, comments, docs, commit messages, PR descriptions.
- No prompt text in `.rs` files, ever. Everything a model reads, and everything written into a file a human answers
  (questions-file instructions, notification texts), lives in `prompts/` as a template with `{{name}}` variables.
  Error messages of the CLI and log events stay in code.
- Async with `tokio`. No external process except: the model CLIs (`claude`, `codex`, `agy`, `opencode`), `git`,
  `agentbus`, the repository's own gate command, the configured notification command and the configured editor.
  Everything the Bash version did with `sed`, `awk`, `jq`, `grep`, `flock`, `gh` or `tailscale` is done natively.
- Approved dependencies (others need a DEP-1 justification): `tokio`, `tokio-util`, `clap`, `serde`, `serde_json`,
  `toml`, `thiserror`, `tracing`, `tracing-subscriber`, `tempfile`, `rustix`, `reqwest` (rustls), `axum`,
  `notify`, `rand`, `time`, `strum` (`derive`), `rusqlite` (`bundled`; the history, from item 8.4).
- An enum with a text form (a keyword enum: a CLI kind, a status, a verdict) takes it from `strum` derives: each text
  declared once, in a `#[strum(...)]` attribute. No hand-written text tables, no macro of our own for it.
- New code is developed and tested against a separate `AGENTS_HOME`; nothing here may touch the live `~/agents` of
  the running Bash harness until feature 10.
- The items of rounds, of the history and of checkpoints (8.3 to 8.10, 11.1, 12.1 and 12.2) are tested harder than
  the developer rules ask: a log that is wrong is worse than no log, and a takeover that is wrong loses work.
  - End to end: every record and every query is proved by a run of the daemon with fake CLIs and a fake GitHub
    server, read back through the reading API or `agent log`. Rows a test inserts by hand prove a query, never the
    recording.
  - Both sides of every rule: the case that satisfies it and the nearest one that does not. An invariant is refused
    by the API and again when read.
  - Every failure the item names is provoked — a killed daemon, a file that cannot be written, a full queue, a newer
    schema, a restart in the middle — and the test shows the work going on and the failure reported.
  - What holds for every input (a round trip, a sum, an order) is tested with generated inputs from a seeded source;
    the failure message gives the seed.
  - What a human reads is pinned in golden files, IDs and times normalised.
  - No secret reaches the history or the log: a test runs a round with marker values for the secrets and searches
    the bytes of the database for them.
  - Time comes from the injected clock, never from sleeping (TDD-8).
  - The PR description maps every sentence of the item that states a behaviour to the test that proves it (`## Tests`,
    a table: behaviour, test). A behaviour without a test is a finding.
  - The reviewers break three behaviours of the PR, not one (the review rules, step 6), at least one of them in what
    is recorded.

Target layout

    Cargo.toml                  workspace
    crates/agent-core/          pure: identifiers, configuration, state, task list, decisions, protocol types
    crates/agent-runtime/       effects: state store, processes, git, GitHub API, agentbus, prompts, notifications
    crates/agent/               the `agent` binary: CLI commands and `agent daemon`
    crates/agent-web/           the `agent-web` binary: dashboard (axum, htmx, server-sent events)
    prompts/clarification.md
    prompts/agent/, prompts/worker-agent/, prompts/investigator/   each role's rules (in the prompt catalog)
    prompts/harness/<module>/<name>.md     texts the harness composes (run prompts, questions files, notifications)
    config/agents.example.toml, config/systemd/
    docs/architecture.md, docs/configuration.md, docs/state.md, docs/task-list.md, docs/history.md, docs/log.md

Process model: `agent daemon` (a systemd user service) is the only owner of the state and of every model run. Every
other command — `agent run`, `answer`, `status`, `stop`, `gate`, ... — and the dashboard are clients of its Unix
socket. One owner, no file races (CON-3). The history database has one writer too, the daemon; `agent log` reads it
directly, so it works when the daemon does not.

Behaviour in short

- `agent run <agents...> "tasks" | -f tasks.md [--coordinator NAME] [--no-merge]`: one agent is a solo task, more
  than one is a team. `--autopilot` is the default: PR → two reviews → merge → next item; `--no-merge` stops before
  the merge and hands the PR to the human.
- Coordinator: an explicit `Coordinator: NAME` line at the top of the task list or `--coordinator` wins; otherwise an
  investigator in the team (randomly among two or more); otherwise a random normal agent. Workers never coordinate.
- An investigator coordinating weaker models leads them: explains, shows the way, and writes or fixes code itself
  when needed.
- The daemon assigns items, one to each normal member at a time. Workers do not take items: they serve `delegate`
  requests, the first free worker in the configured order.
- The coordinator integrates a member's branch, gets the gate green and opens the PR. The next item of a feature
  starts when the previous one is merged (with `--no-merge`: approved by both reviews).
- Every PR gets two reviews from investigators outside the team. A team that leaves fewer than two configured
  investigators outside it is refused at start: "too many investigators in the team". "Changes requested" goes back
  to the team at most 5 times; after that the human is asked.
- A team and a round are things with an ID (`team-…`, `round-…`): a round is one item of a team's list from its
  start to its merge. What happens — who was chosen and how, every prompt, every step and change of every run, the
  pull request and its reviews, the daemon's own log — is recorded under those two IDs in the history, an SQLite
  database, and read with `agent log`. The state file stays the one place for what is; the history is what happened.
- Work that is interrupted is taken up where it stands. A run that used up its turns is resumed, not failed. A
  round outlives its team: `agent run --continue <round>` gives it to a new team, in the same worktrees. Every task
  keeps a plan in its worktree — to do, in hand, done — that the harness reads and enforces; a task whose agent is
  gone is taken over by a free agent of the right tier on another account, who gets a briefing made from the
  history and the plan. `agent status <round>` shows each task's checkpoint as `OPEN`, `BUSY` or `CLOSED`.

Checkpoints for the human, between features (the agents do not run these):

- After feature 2: read `docs/configuration.md` and `docs/state.md`. The configuration and the state decided there
  are expensive to change later.
- After feature 7: a trial on a scratch repository with fake CLIs and a separate `AGENTS_HOME`: a solo task and a
  team from a two-item list, one question answered, one stop and resume.
- After item 8.3: read `docs/state.md` again: version 2 of the state and of the protocol, and the round.
- After item 8.4: read `docs/history.md` and ask the database of one round with `sqlite3`. The schema of the history
  is expensive to change later.
- After item 8.9: stop a real solo task in the middle of its work and continue its round with another agent: it
  works in the same worktree and its first message shows that it knows what was done.
- After feature 8, before feature 10: a real run with cheap models on a toy repository and a separate
  `AGENTS_HOME`, in the default `--autopilot`: from a two-item list to two merged PRs. Then `agent log --round` of
  each: the story is whole — the task, who was chosen and how, every prompt, every step, every change, both
  reviews — and `agent log --level warn` shows nothing unexplained. The cutover starts only after it passes.
- After feature 9: the dashboard opened through `tailscale serve` from another device: the teams listed, a question
  answered, a team paused and resumed.
- After item 12.1: read the `provider` and `account` of every agent in the live `agents.toml`; they decide who may
  stand in for whom.

Out of scope for this plan: remote nodes and the lead, investigation teams with phases and standalone investigation
requests, `agent live`, archiving, the cost of tokens in money, moving the state file into the database, choosing
agents by the quota their accounts have left (load balancing), automatic PR size limits (reviewers check PR-1), a
proof by the harness that a RED commit fails (a reviewer runs one, TDD-1), an `agentbus` socket API (it is called
as a process).

1. Workspace and core vocabulary
Depends on: none
[ ] 1.1 Create the Cargo workspace, its gate and the core vocabulary
    Workspace with the four crates of the target layout; `agent --version` and `agent-web --version` work.
    `rust-toolchain.toml` (current stable), edition 2024, the lint baseline from the appendix of the developer rules
    in `[workspace.lints]`, `rustfmt.toml`, `clippy.toml`. A `make` target that runs fmt check, clippy `-D warnings`,
    tests and `cargo doc` with warnings denied, added to the repository's existing gate and CI.
    `docs/architecture.md` with the layout, the process model and the rules above; a Rust section in `AGENTS.md`
    pointing to it and stating the relaxed size limit of this plan.
    In `agent-core`: validated newtypes (parsing, `Display`, serde) for agent names, investigator names, team IDs,
    task IDs, run IDs, task-list item IDs (`2.3`) and branch names, invalid values unconstructable (TYP-1), ID
    generation with an injected clock and random source; `CliKind` (claude, codex, gemini, opencode), `Tier`
    (normal, worker, investigator), `Role` (solo, coordinator, member, worker, reviewer), run outcomes, review
    verdicts, and task and item statuses as state machines whose invalid transitions do not compile or return a
    typed error. Reference: the STATUS values of `bin/agent` `final_status`.
    Expected result:
    - Files: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`; `Cargo.toml` and
      `src/` of `crates/agent-core`, `crates/agent-runtime`, `crates/agent`, `crates/agent-web`;
      `crates/agent-core/src/{ids,kinds,status}.rs`; `docs/architecture.md`. Changed: `Makefile`, `AGENTS.md`, the CI
      workflow.
    - Adds: `AgentName`, `InvestigatorName`, `TeamId`, `TaskId`, `RunId`, `ItemId`, `BranchName` (`FromStr`,
      `Display`, serde); `CliKind`, `Tier`, `Role`, `RunOutcome`, `ReviewVerdict`; `TaskStatus` and `ItemStatus` with
      their transitions; the Rust gate target.
    - Tests: round trip and rejected values for each newtype; every allowed and every refused status transition.
    - Why: every later item builds on these types, and the gate makes every later PR checkable.
    Checks:
    - On a clean clone the Rust gate target and the repository's whole gate pass.
    - `cargo run -q -p agent -- --version` and `cargo run -q -p agent-web -- --version` print a version and exit 0.
    - `cargo tree -p agent-core -e normal` lists only approved dependencies.
    - Each newtype refuses, in a named test, every invalid value its documentation names.

2. Core models and contracts
Depends on: 1
[ ] 2.1 Configuration and the task list file
    `agent-core`: the typed model of `agents.toml` and `secrets.toml` (mode 0600, never logged — SEC-2), parsed with
    `deny_unknown_fields`: agents and investigators (CLI kind, model, extra arguments as an array), the normal and
    worker lists (worker order = delegation order), binary paths and the agents' PATH, language of human-facing texts
    (default English), watchdog and limit settings, review fix passes (default 5), git identity and signing key,
    trusted GitHub accounts, notification command and directory, web listen address and allowed logins,
    per-repository gate command and base branch. Validation: a name in two lists, an unknown name, a duplicate.
    `config/agents.example.toml`, `docs/configuration.md`. Reference: the old `agents.conf`.
    The task-list parser for the format of this file: an optional `Coordinator: NAME` line, a free-text brief,
    features (`N. Title`), an optional `Depends on:` line, items (`[ ] N.M Title`, `[x]` accepted as done) and
    indented description lines, with an indented `Expected result:` part kept apart (the reviewers get it); errors
    carry the line number; inline text and `-f file` give the same result. This
    file itself is a test fixture that must parse. `docs/task-list.md`.
    Expected result:
    - Files: `crates/agent-core/src/{config,task_list}.rs` (or modules of those names),
      `config/agents.example.toml`, `docs/configuration.md`, `docs/task-list.md`.
    - Adds: `Config`, `Secrets`, `AgentConfig`, `InvestigatorConfig`, `RepoConfig` and their typed load errors;
      `TaskList`, `Feature`, `Item` (title, description, expected result) and the parser with its typed errors.
    - Tests: the example configuration loads; each validation error; task-list fixtures, this plan among them;
      line numbers in errors.
    - Why: every start reads both, and an item's expected result is what its reviewers check.
    Checks:
    - `config/agents.example.toml` loads in a test; a copy with one name in both tier lists fails with the typed error
      naming that name.
    - This plan file parses in a test into 10 features and 16 items, each item with its expected result.
    - A task list with a malformed item line fails with the number of that line.
[ ] 2.2 State, control protocol and questions files
    `agent-core`: the persistent state model — teams, members and their roles, the task list with per-item progress,
    runs, questions and answers, PRs, reviews and fix passes, gate results — with a schema version and serde
    round-trip tests; `docs/state.md`. The versioned request and response types of the daemon socket (JSON lines):
    run, status, show, answer, stop, pause, unpause, resume, say, gate, and a subscription to state events for the
    dashboard; unknown versions refused with a typed error. Parsing and rendering of a questions file: questions, the
    recommendation, a `**Verdict:**` per question (`ok` accepts the recommendation), the `**ANSWERED**` marker on the
    last line; the instruction text comes from a template. Reference: `bin/agent` `questions_note`,
    `live/web_questions.go`.
    Expected result:
    - Files: `crates/agent-core/src/{state,protocol,questions}.rs`, `docs/state.md`.
    - Adds: `State`, `Team`, `Member`, `ItemProgress`, `Run`, `Question`, `Review`, `GateResult` with a schema
      version; `Request`, `Response`, `Event` with a protocol version; `QuestionsFile` parsing and rendering.
    - Tests: serde round trips; a wrong schema or protocol version refused; questions files with `ok`, an own
      answer, a missing verdict and the ANSWERED marker.
    - Why: one persisted model and one protocol shared by the daemon, the CLI and the dashboard.
    Checks:
    - A state saved and loaded back is equal; a state with another schema version is refused with a typed error.
    - A request with an unknown protocol version is refused with a typed error.
    - A questions file rendered, answered with `ok` and with own text, and marked ANSWERED parses back to those
      answers.
[ ] 2.3 Prompt catalog and the model CLI contract
    `agent-core`: `PromptId` — every prompt and human-facing text, with its path under `prompts/` and its required
    variables; a strict `{{name}}` renderer (a missing or unknown variable is an error, literal braces can be
    escaped); per-language overrides of human-facing texts with English as the fallback. Adds the role prompts of the
    layout (the developer and review rules provided by the human, the worker and investigator variants; the
    existing `prompts/clarification.md` is used as it is) and a test that every catalog entry exists and renders
    with its variables.
    The trait every CLI adapter implements: build a run (program, arguments, environment, files such as an MCP
    configuration) from a typed request; turn the CLI's output into typed activity events; extract the final
    message, the session ID and the error tail. Pure: an adapter never spawns anything.
    Expected result:
    - Files: `crates/agent-core/src/{prompt,cli}.rs`; `prompts/agent/rust-developer.md`,
      `prompts/worker-agent/rust-developer.md`, `prompts/investigator/rust-developer.md`,
      `prompts/investigator/code-review.md`.
    - Adds: `PromptId` with paths and required variables, the `{{name}}` renderer and its errors, the language
      fallback; the `CliAdapter` trait with `RunRequest`, `RunSpec`, `ActivityEvent`, `RunOutput`.
    - Tests: rendering (missing, unknown and escaped variables); every catalog entry exists and renders; the trait
      with a test adapter.
    - Why: keeps every prompt out of the code and fixes the contract the four adapters implement.
    Checks:
    - `git grep -nE '"[^"]{100,}"' -- 'crates/*.rs'` finds no long text literal outside tests: the texts are in
      `prompts/`.
    - Removing a prompt file, or a variable a prompt requires, makes the catalog test fail with that file's name.
    - A test adapter implements the CLI contract without spawning anything.
[ ] 2.4 Replace the `keyword_enum!` macro with strum
    The keyword enums (`CliKind`, `Tier`, `Role`, `RunOutcome`, `ReviewVerdict`, `TaskStatus`, `ItemStatus`, and any
    added since) take their texts from `strum` derives instead of the hand-written `keyword_enum!` macro: `strum` is
    maintained, widely used and tested, and the macro reimplements it. Add `strum` to `[workspace.dependencies]` with
    the `derive` feature only (DEP-2), at its current release. Each text is declared once, in a `#[strum(...)]`
    attribute (`serialize_all` where an enum's texts follow one case rule, `serialize` on a variant that does not),
    and `Display`, parsing, `as_str()`, serde and the list of variants all read it.
    Behaviour does not change. The same texts; parsing stays case-sensitive (no `ascii_case_insensitive`); the same
    `UnknownKeyword` error with what was parsed, the refused text and the accepted texts in declaration order —
    strum's own parse error carries none of these, so keep `UnknownKeyword`, e.g. through `EnumString`'s
    `parse_err_ty` and `parse_err_fn` or one generic helper over `VariantNames`; serde through the same texts and
    through validation (TYP-2); `ALL` and `as_str()` kept for their callers. The macro and its documentation are
    removed. A refactoring with no behaviour change (PR-4), with nothing else in the PR (PR-7).
    An exception to the order of items: this item depends only on feature 1 and may run before 2.2 and 2.3. A keyword
    enum one of them added with the macro before this item is merged is converted here too.
    Expected result:
    - Files: `Cargo.toml` (the workspace dependency), `Cargo.lock`, `crates/agent-core/Cargo.toml`,
      `crates/agent-core/src/kinds.rs`, `crates/agent-core/src/status.rs`; any other file that declares a keyword enum
      with the macro.
    - Removes: `keyword_enum!`. Adds: the strum derives and attributes on each keyword enum; `UnknownKeyword` built
      from strum's variant names.
    - Tests: the existing tests of the keyword enums (`assert_keywords`, `assert_refuses`, the status-transition tests)
      pass unchanged: none edited or removed. A new test only for a path strum adds that they do not cover.
    - Why: a tested, maintained crate instead of a home-made macro (DEP-1: `strum` is the standard crate for this; std
      and the existing dependencies have no derive for it).
    Checks:
    - `git grep -n 'keyword_enum' -- crates` finds nothing.
    - `git diff <base> -- crates/agent-core/tests` is empty, and the PR edits or removes no `#[test]` function in
      `kinds.rs` or `status.rs`.
    - `cargo tree -p agent-core -e features -i strum` shows `strum` with its `derive` feature only.
    - The Rust gate passes.

3. Core decisions
Depends on: 2
[ ] 3.1 Team formation, assignment, the autopilot round and run outcomes
    Pure functions, each fully tested. Team formation: from the requested agents, the configuration, an optional
    coordinator and a random source to a team plan — solo or team, the coordinator by the rule of the brief, typed
    refusals (unknown agent, worker as coordinator or member, too many investigators in the team for two outside
    reviewers). Work assignment: one item per normal member; items of a feature in order (the next starts when the
    previous is merged, or approved with `--no-merge`); features in parallel once their "Depends on" features are
    merged. The round of one item as typed states and events: member work → coordinator integration → gate green at
    the head → PR open → two reviews → merge (or handed to the human with `--no-merge`) → item done; "changes
    requested" → fix pass, at most the configured number, then a question to the human. Run outcome
    classification from the final message, exit status and error tail: done, questions, failed, quota exhausted
    (with the reset time when the CLI gives one), rate limited, hung — and the pause each implies; fixtures of real
    CLI outputs. Reference: `bin/agent` limit and quota handling.
    Expected result:
    - Files: `crates/agent-core/src/decide/{team,assign,round,outcome}.rs`; fixtures of CLI outputs under
      `crates/agent-core/tests/`.
    - Adds: `plan_team`, `next_assignments`, `RoundState` and `RoundEvent` with their transition function,
      `classify_outcome`.
    - Tests: table tests per rule — coordinator precedence, each refusal, investigators in the team, assignment
      order across features and items, every round transition, each outcome kind per CLI.
    - Why: every decision of the harness in one pure, fully tested place; the daemon only carries them out.
    Checks:
    - Each rule of "Behaviour in short" has a named test; the PR lists them.
    - `git grep -nE 'std::(fs|process|net)|tokio' -- crates/agent-core/src` finds nothing: the decisions do no I/O.

4. Runtime services
Depends on: 2
[ ] 4.1 State store, prompts, processes, git and notifications
    `agent-runtime`, local effects. The `AGENTS_HOME` layout, atomic writes (temporary file and rename — SEC-5), a
    lock that keeps one daemon per home (`rustix` flock), schema-version check, an append-only activity log per task.
    The prompt loader: reads the catalog's files from the configured directory, validates all at startup, applies the
    language override, renders. The process supervisor: argument vector, environment and working directory
    (`tokio::process`, never a shell string — SEC-3), its own process group, stdout and stderr to files,
    last-activity time, stop of the whole group with a timeout, exit status, everything bounded (CON-5). Typed git
    operations over the `git` command — worktree add and remove, branch, merge without fast-forward, merge in
    progress, clean-tree check, rev-parse, push — with commit identity and signing per run, tested on real temporary
    repositories. Notifications: the file (title line, body already rendered by the caller) and the configured
    command run with its path; secrets only in its environment.
    Expected result:
    - Files: `crates/agent-runtime/src/{store,prompts,process,git,notify}.rs`.
    - Adds: `StateStore` (load, save, home lock, activity append), `PromptLoader`, `Supervisor` and
      `RunningProcess`, the git operations, `Notifier`.
    - Tests: atomic save and a second lock refused; prompt files missing or with wrong variables; a process group
      stopped whole and a timeout; git operations on temporary repositories; the notification command called with
      its file and environment.
    - Why: the local effects the daemon needs, each behind a small typed boundary.
    Checks:
    - A second lock of the same home fails with the typed error while the first is held.
    - A supervised process that started children leaves none of its group behind after a stop (a test checks with the
      process table).
    - A save interrupted before its rename leaves the old state whole and no temporary file in the home.
[ ] 4.2 GitHub API and agentbus
    `agent-runtime`, remote effects. A `reqwest` client for the GitHub REST API — create, update, mark ready, merge
    and read a PR, post a review or comment, list comments filtered to the trusted accounts; token from
    `secrets.toml`; timeouts and capped retries for idempotent calls only; tested against a local fake server.
    A typed wrapper over the `agentbus` commands the daemon uses (team create, get, set and done; board add, dump,
    claim and release; delegation queue; checkpoints; send; status), exit codes mapped to typed errors, tested
    against the real `agentbus` built from `agentbus/`.
    Expected result:
    - Files: `crates/agent-runtime/src/{github,agentbus}.rs`.
    - Adds: `GitHub` (the PR, review and comment operations, the trusted-account filter); `AgentBus` (the commands
      the daemon uses, typed results and errors).
    - Tests: each GitHub call against a local fake server, retries and timeouts included; each agentbus command
      against the real binary in a temporary bus directory.
    - Why: the two remote effects, testable without GitHub and without a running team.
    Checks:
    - The GitHub tests reach only the local fake server: its base URL is injected, none names api.github.com.
    - The agentbus tests run the binary built from `agentbus/` in a temporary bus directory and never touch
      `~/agents`.

5. Model CLI adapters
Depends on: 2
[ ] 5.1 Claude Code, Codex, Antigravity and opencode adapters
    The 2.3 contract for each CLI: model, extra arguments, session resume, its MCP configuration with `agentbus serve`
    (and Serena when configured), its output parsed into activity events, the final message. Per CLI: `claude` turn
    limit and stream JSON; `codex` configuration flags and event stream; `agy` print timeout, MCP file and effort
    flags; `opencode` provider/model strings and bash timeout. Fixtures of recorded output for each. Reference: the
    CLI dispatch of `bin/agent`.
    Expected result:
    - Files: `crates/agent-runtime/src/cli/{claude,codex,agy,opencode}.rs`; recorded outputs as test fixtures.
    - Adds: one `CliAdapter` per CLI.
    - Tests: per CLI, the program, arguments, environment and MCP configuration for a new run and a resumed one;
      recorded outputs parsed into activity events, the final message and the session ID.
    - Why: every model runs through the same contract; a CLI's quirks stay in its adapter.
    Checks:
    - For each CLI a test asserts the exact arguments of a new run and of a resumed one, and an MCP configuration
      naming `agentbus serve`.
    - Each CLI's recorded output gives its final message and session ID; a cut output gives a typed error, not a
      panic.

6. Daemon and runs
Depends on: 3, 4, 5
[ ] 6.1 The daemon, its socket and the run engine
    `agent daemon`: one instance per home, the Unix socket (mode 0600) serving the 2.2 protocol, graceful shutdown
    (`CancellationToken`, every spawned task joined — CON-4), recovery on start (runs interrupted by a restart are
    marked and resumed from their session), `agent status` as its first client, a systemd user unit in
    `config/systemd/`. The run engine: compose the prompt (role prompt, rules, brief, item, checkpoint) from
    templates, start it through its adapter and the supervisor, write activity, classify the outcome, apply it.
    Watchdog: no activity for the configured minutes kills the run; resumed with its session up to the configured
    strikes, then the task is marked hung. Quota and rate limits pause the task and wake it on time.
    Expected result:
    - Files: `crates/agent/src/main.rs`, `crates/agent/src/daemon/`, `crates/agent/src/client.rs`,
      `crates/agent/src/commands/status.rs`, the run engine, `config/systemd/agent-daemon.service`, the run prompts
      under `prompts/harness/run/`.
    - Adds: `agent daemon`, the socket client, `agent status`, the run engine with the watchdog and the pause and
      wake of limits.
    - Tests: a daemon in a temporary home started and stopped, a second one refused; `agent status` over the socket;
      runs of a fake CLI that ends done, with questions, hung and out of quota; recovery after a restart.
    - Why: the single owner of state and runs that every command and the dashboard talk to.
    Checks:
    - With `AGENTS_HOME=$(mktemp -d)`: `agent daemon &` starts, `agent status` answers, a second `agent daemon` exits
      with "already running".
    - SIGTERM to the daemon ends it and its runs, with no process of theirs left.
    - A fake CLI silent past the (shortened) watchdog limit is killed and resumed; after the configured strikes the
      task is hung.
    - A fake CLI reporting an exhausted quota pauses its task until the reset time it gave.

7. Teams, questions and the gate
Depends on: 6
[ ] 7.1 Starting work and the team's life
    `agent run <agents...> "tasks" | -f file [--coordinator NAME] [--no-merge]`: the team plan, the agentbus team,
    branches and worktrees, the first assignments, start; the client prints what started and where. Then the team's
    life: a member done → the coordinator integrates; the next assignment; wake-ups on bus messages; `delegate`
    requests served by the first free worker in the configured order, the result returned to the requester; a team
    with all items done is closed and the human notified.
    Expected result:
    - Files: `crates/agent/src/commands/run.rs`, `crates/agent/src/daemon/team.rs`, the team prompts under
      `prompts/harness/team/`.
    - Adds: `agent run` and the team's life: integration, assignments, wake-ups, delegation, closing.
    - Tests: with fake CLIs — a solo task and a team started, each refusal of `agent run`, a member done followed by
      integration and the next assignment, a delegation served by the first free worker, a finished team closed.
    - Why: the core flow from a task list to working agents.
    Checks:
    - With fake CLIs on a scratch repository: `agent run a "text"` starts a solo task; `agent run a b -f list.md`
      starts a team whose coordinator follows the rules.
    - `agent run` with a worker, or with too many investigators for two outside reviewers, is refused before anything
      starts.
    - A member's DONE leads to the coordinator's integration and the next assignment, both shown by `agent status`.
[ ] 7.2 Questions, answers and controls
    A run ending with questions writes the next questions file and notifies the human; it opens in VS Code when
    configured. Answers arrive by `agent answer <id>` (the editor, or `--file`), by saving the file with the ANSWERED
    marker (watched with `notify`), or from the dashboard; the task resumes with them. `agent stop`, `pause`,
    `unpause`, `resume` and `say` for a task or a team; state and activity show what was done and by whom.
    Expected result:
    - Files: `crates/agent/src/commands/{answer,stop,pause,unpause,resume,say}.rs`,
      `crates/agent/src/daemon/questions.rs`, the questions-file and notification texts under
      `prompts/harness/questions/`.
    - Adds: the questions lifecycle and the controls.
    - Tests: a questions file written and the task resumed after answers from `agent answer --file`, from a saved
      file and from the protocol; each control's effect on state and activity.
    - Why: the human's two ways into a running team: answering and steering.
    Checks:
    - A fake CLI ending with questions leaves a questions file and a notification file; `agent answer <id> --file
      answers.md` resumes the task with the answers in its next prompt.
    - `agent pause`, `unpause` and `stop` change the status `agent status` shows, and the activity names who did it.
[ ] 7.3 Gate service
    `agent gate`, run by agents in their worktree: a FIFO queue in the daemon, one gate at a time on the machine,
    the repository's gate command with a timeout, the result recorded with the commit and whether the tree was
    clean (a merge in progress may be gated; it is recorded as not clean). A run waiting on its own gate is never
    cut by the watchdog.
    Expected result:
    - Files: `crates/agent/src/commands/gate.rs`, `crates/agent/src/daemon/gate.rs`.
    - Adds: `agent gate` and the daemon's gate queue with its records.
    - Tests: two gates waiting run in order; a timeout; the record of a clean pass, a failure and a merge in
      progress; a run waiting on its gate not cut by the watchdog.
    - Why: one gate at a time on the machine, and a recorded result the autopilot can trust.
    Checks:
    - Two `agent gate` calls at once: the second waits, then runs; both results are recorded with their commit.
    - A gate over its timeout is stopped and recorded as failed; a run waiting on it is not cut by the watchdog.

8. Autopilot, rounds, the history and checkpoints
Depends on: 7
[ ] 8.1 Pull requests, reviews and the merge loop
    When an item's integration has a green gate at its head: push, open the PR with a body from a template, mark it
    ready; later pushes update it. Two free investigators from outside the team (the PR waits until two are free) review
    it with the review rules of the prompt catalog and the item's full text, its expected result included; the verdict
    is parsed from the review rules' report format and posted on the PR. Both ready → merge through the API, then the
    next item; with `--no-merge` the human is notified instead. Changes requested → a fix pass for the team with the
    findings, at most the configured number; then a question to the human. All items done → the team ends and the human
    is notified.
    Expected result:
    - Files: `crates/agent/src/daemon/{pr,review,merge}.rs`, the PR and review texts under `prompts/harness/pr/`
      and `prompts/harness/review/`.
    - Adds: PR opening and updating, reviewer selection and verdict parsing, merge, fix passes, `--no-merge`.
    - Tests: with a fake GitHub server and fake CLIs — two ready reviews merge and start the next item; changes
      requested start a fix pass; the last fix pass asks the human; `--no-merge` notifies instead of merging; the
      reviewer's prompt holds the item's full text and its expected result.
    - Why: the default `--autopilot` loop, end to end.
    Checks:
    - Against the fake GitHub server: two ready reviews merge the PR and start the next item; changes requested start
      a fix pass; after the configured passes the human gets a question.
    - With `--no-merge` nothing is merged and the human is notified.
    - The reviewer's prompt, as the fake CLI recorded it, holds the item's title, description and expected result.
[ ] 8.2 Round checks before review
    Before a PR's head goes to the reviewers — the first time and after every fix pass — the daemon checks the
    round's own diff (from the merge base with the base branch to the head) and its commits, natively through git,
    with no model. What is a test and what judges a round comes from the repository's `checks` table in
    `agents.toml`, as path patterns: `test_paths` (the test files), `code_paths` (the production code),
    `protected_paths` (the files that judge a round: the gate's scripts, lint and CI configuration, the
    repository's rules). Nothing in the code names a repository or its layout; a repository without the table goes
    straight to review.
    Only the human may allow — the round waits on a question that names each file; allowed: the review starts and
    the paths are recorded for the item; refused: back to the team:
    - an existing test file changed other than by added lines, removed or renamed;
    - a protected file changed.
    Back to the team with the reasons, at most `check_passes` times (`[review]`), then the human is asked:
    - a test file that a `RED:` commit of the round touched has lines removed or replaced by a later commit, unless
      that commit touches test files only, its subject starts with `TEST-FIX:` and the PR description names it
      under `## Tests changed after RED` (TDD-14);
    - the round changes production code and adds no line to a test file, and none of its commits says
      `Test-First: not possible — <reason>` (TDD-1);
    - an added line that skips a test, suppresses a lint or asks whether the code runs under test, in a file the PR
      description does not name (TDD-11, LINT-3, TDD-12); the markers are a built-in table per file extension.
    A test module inside a source file is not a test file: it stays with the reviewers (the review rules, step 4).
    Expected result:
    - Files: `crates/agent-core/src/decide/checks.rs`, the `checks` table in `crates/agent-core/src/config.rs`, the
      round's checking phase in `crates/agent-core/src/decide/round.rs`, the commits and changed lines of a range in
      `crates/agent-runtime/src/git.rs`, `crates/agent/src/daemon/checks.rs`, the texts under
      `prompts/harness/checks/`, `config/agents.example.toml`, `docs/configuration.md`.
    - Adds: the `checks` table and `check_passes`; the checks as a pure decision (the facts of a diff in, the issues
      out); the round's checking phase with its two outcomes; the question to the human and the message to the team.
    - Tests: each rule as a decision — the case that raises it and the nearest one that does not (added lines only,
      a named `TEST-FIX:` commit, a `Test-First: not possible` line, a named file); the facts of a range read from a
      real repository in a temporary directory; with a fake GitHub server and fake CLIs — an issue for the team
      starts no review and wakes the team with the reasons; after `check_passes` times the human is asked; an issue
      only the human may allow asks the human, and the answer starts the review or sends the round back; the checks
      run again on the head of a fix pass; a repository without the table goes straight to review.
    - Why: the Bash harness held such rounds without a model (its precheck); without these checks the default
      `--autopilot` merges on the word of two models.
    Checks:
    - Against the fake GitHub server: a round that edits an existing test file gets no review; the human's question
      names the file; after the human allows it, the two reviews start.
    - A round whose `GREEN:` commit edits a test file its `RED:` commit added goes back to the team with the commit
      and the file; the same change as a `TEST-FIX:` commit named in the PR description goes to review.
    - A round that changes a protected file waits for the human; a round that changes code and adds its tests goes
      straight to review.
[ ] 8.3 Rounds as an entity
    A round is the work of one team on one item of its task list, from the item's start to its merge (with
    `--no-merge`: to its approval) — or the whole work of a team whose list has no items. One round, one branch of
    the lead, one pull request. Until now a round is worked out at every look from the items' phases; it becomes a
    thing with an ID, so that whatever happens can say which round it belongs to. A team is already such a thing
    (`TeamId`): after this item a team's ID and a round's ID are the two keys everything else is found by.
    The word: from now on "round" means this and nothing else in the code, the state, the protocol, the CLI's
    output and the documents of the Rust `agent`. What they call the round of a questions file becomes its
    `number`; the reviews asked for one head of a pull request are a review pass. This rename is mechanical: it
    comes first, in commits of its own that change no behaviour, in this same PR (the one exception to PR-4 this
    item makes).
    `RoundId` is `round-YYYYMMDD-HHMMSS-xxxx`, generated like the other IDs. State version 2: a team holds its
    rounds, oldest first — `{id, item?, opened_at, state}`, the state `open` or `closed` with a time and how:
    `merged`, `approved` (`--no-merge`) or `abandoned` (the team was stopped or ended without it) — and every run
    names the rounds it works for. The daemon opens a round when its item is first given to a member (a list
    without items: when the team starts) and closes it when the item is done. It decides the rounds of a run when
    it starts the run, by a pure function of the role and of why the run starts:
    - a member or a solo agent: the round of its item; none while it has no item;
    - the coordinator: every round of its team that is open;
    - a reviewer: the round of the pull request it reviews;
    - a worker: the rounds of the run that asked for it;
    - a fix pass, answers and a resume stay in the rounds the task works for.
    Invariants, checked by the API and again when the state is read: an item has at most one round and has one as
    soon as it leaves `pending`; a closed round is never opened again and its item is done (or its team is over); a
    run names only rounds of its own team, none twice; a round that is `merged` belongs to an item `done` as
    merged. A daemon that restarts goes on with the same rounds.
    Version 1 is migrated when read, as `docs/state.md` requires: each item past `pending` gets a round whose ID is
    derived from its team's ID and its place in the list (reading the same file twice gives the same state); the
    runs of version 1 name no round. Protocol version 2: `status` and `show` take `{"round": "round-…"}` beside a
    team and a task; a team's summary lists its rounds; `show` of a round gives its item, its state, each task
    that worked for it with its role, agent, worktree and branch, its runs, its pull request and its review
    passes; the events `round_opened` and `round_closed`, and `round` on every event that is about one. `agent
    status <team>` lists the rounds, `agent status <round>` shows one, `agent run` prints the round of each item.
    Expected result:
    - Files: `crates/agent-core/src/ids.rs`, `crates/agent-core/src/state/round.rs` and the state's other modules,
      version 2 and the migration in `crates/agent-core/src/state.rs`, the rounds of a run in
      `crates/agent-core/src/decide/round.rs`, `crates/agent-core/src/protocol/`, `crates/agent/src/daemon/owner/`,
      `crates/agent/src/commands/status.rs`, the fixtures `state-v2.json`, `state-v1-as-v2.json` and
      `protocol-v2-*.jsonl`, `docs/state.md`, `docs/architecture.md`.
    - Adds: `RoundId`; the round in the state with its invariants; state version 2 with its migration; the rounds
      of a run; protocol version 2 with the round target, the round in summaries and events; rounds in `agent
      status`.
    - Tests: the ground rule on testing these items applies. IDs: the round trip, generated IDs, every refused
      form. State: each invariant refused by the API and refused when read; any sequence of the API's operations
      (generated, seeded) gives a state that saves and loads to the same text; `state-v2.json` is written back as
      read; `state-v1.json` loads as exactly `state-v1-as-v2.json`, twice the same; a file of version 3 is refused
      as an unsupported version. Protocol: the lines of version 2 pinned; a line of version 1 refused as an
      unsupported version. The rounds of a run: a table of cases — every role, no item, two open rounds, a worker
      asked by a run with rounds and by one without, a fix pass, answers, a resume. The daemon, with fake CLIs and
      a fake GitHub server: a coordinator, a member and a worker on a two-item list open and close two rounds in
      order and every run names the rounds the table says; a fix pass stays in its round; `--no-merge` closes a
      round as `approved`; a stopped team's open rounds close as `abandoned`; a list without items is one round; a
      daemon stopped in the middle of a round goes on with the same round ID; two teams at once never share a
      round. `agent status` of a team and of a round, as text.
    - Why: without an ID a round cannot be asked for — its logs, tasks, agents, worktrees, prompts and tokens.
    Checks:
    - With fake CLIs and the fake GitHub server: `agent run` of a two-item list prints two round IDs; `agent status
      <round>` shows the item, the tasks with role, agent, worktree and branch, and the pull request.
    - `git grep -nwi round -- crates docs/state.md` finds no line that means a questions file or a review pass.
    - A state file of version 1 copied into a fresh `AGENTS_HOME` is read; the saved file has version 2.
[ ] 8.4 The history: a database of what happened, and what every run did
    The state file says what is; the history says what happened. It is one SQLite database, `history.db` in the
    home (`rusqlite` with the `bundled` feature: WAL, foreign keys on, `STRICT` tables), append only, every record
    keyed by its team and, where it has one, its round. The state file, the questions files, the worktrees and the
    files of a gate's output stay where they are.
    One writer: the daemon, on a thread of its own behind a bounded queue, a record or a batch per transaction.
    The history never stops or delays the work: when it cannot be written — the file, the disk, a full queue — the
    work goes on, the failure goes to stderr once per cause, the records lost are counted, and `agent status`
    shows the count. Readers open the file read-only and need no daemon. The schema has a version (`PRAGMA
    user_version`) and forward-only migrations in code; a database of a newer version is not written: the daemon
    says so at its start and runs without a history. `[history]` in `agents.toml`: `enabled` and
    `max_text_bytes`, the longest text of one field (a longer one is cut at a character boundary and marked cut).
    Recorded in this item, each with its time:
    - a team starting: the task list as given, the repository and its base branch, the merge mode, the agents named
      on the command line, and the plan — every participant with its role and how it got it: named, named by
      `--coordinator`, named by the `Coordinator:` line, drawn at random (among whom), the configured order;
    - a round opening: its item in full (title, description, expected result, checks), or the brief;
    - a run: its task, agent (name, tier, CLI, model), role and rounds; why it starts (a first run, an item, a
      wake-up, a worker's request with the run that asked, a fix pass with its number, a review with its pull
      request and review pass, answers, a resume by the human, a continuation); the whole prompt as the CLI got
      it; the command line and the working directory (the worktree), its branch and its head; the names — never
      the values — of the environment variables the daemon sets; the session; at its end: how it ended, the final
      message whole, the head;
    - every step of a run, in order: what `ActivityEvent` holds today — a message, a command, a file edit, a bus
      call, another tool call, an error, a control — with its number in the run;
    - the changes of a run, from git: the commits made between its two heads (hash, subject, author time) and, for
      each file, added, deleted, modified or renamed (from where), the lines added and removed, binary or not;
      and what it left uncommitted in the worktree.
    A typed reading API in `agent-runtime` gives these back: the runs of a round or a team, the steps and the
    changes of a run, the prompts of a round. `docs/history.md` documents every table, column and index, the
    pragmas and how to ask with `sqlite3`; the schema is pinned as a file a test compares with a fresh database.
    Not in this item: gates, checks, pull requests, reviews, questions (8.5); the daemon's own log (8.6); `agent
    log` (8.5, 8.7); what is deleted and when (8.7); tokens (11.1).
    Expected result:
    - Files: the records in `crates/agent-core/src/history.rs`, `crates/agent-runtime/src/history/` (schema,
      migrations, writer, reader), `crates/agent/src/daemon/history.rs` and the places of the owner and the run
      engine that record, a run's commits and files in `crates/agent-runtime/src/git.rs`, `[history]` in
      `crates/agent-core/src/config.rs`, `config/agents.example.toml`, `docs/history.md`, `docs/configuration.md`,
      `docs/architecture.md`, `Cargo.toml`.
    - Adds: the dependency; the database with its schema, version and migrations; the writer with its queue and its
      count of lost records; the records of teams, rounds, runs, steps and changes; the reading API.
    - Tests: the ground rule on testing these items applies. The schema: a fresh database equals the pinned file;
      the migration mechanism with a test-only older version; a newer version is refused and the daemon runs
      without a history. Records: each kind written and read back equal, with generated values (seeded) —
      non-ASCII text, four-byte characters, a text of exactly, one under and one over `max_text_bytes`, cut at a
      character boundary and marked. Writing: the steps of several runs written at once keep each run's order; a
      second connection reading while the writer writes sees whole records only; the same record written twice is
      one row; a writer killed in the middle of a batch (a child process, `SIGKILL`) leaves a database that opens,
      passes `PRAGMA integrity_check` and holds no half record. Failures: a `history.db` that is a directory, a
      home that is read-only, a full queue, a write that fails — the run ends as it would, the failure is on stderr
      once, the count is in `agent status`. End to end, a coordinator, a member and a worker with fake CLIs on one
      item: the plan with every role and how it was got (the draw of a seeded source included); a row for every run
      and none twice, also across a daemon restart in the middle of a run; each prompt equal, byte for byte, to
      what the fake CLI recorded it got, workers and the coordinator included; the command line equal to what the
      fake CLI saw; the steps equal to the fake CLI's script, in order; the changes equal to what the fake CLI did
      — a file added, deleted, modified, renamed, a binary one, one left uncommitted; the worker's run names the
      run that asked. Secrets: after that round, run with marker values in `secrets.toml` and in the environment
      of the CLIs, the bytes of `history.db` and of its WAL do not hold the markers.
    - Why: every question about a round — who, with what prompt, in which worktree, doing what, changing what — is
      answered from one place that can be searched, instead of files spread over the task directories.
    Checks:
    - After a round with fake CLIs: `sqlite3 history.db` with the queries of `docs/history.md` lists the round's
      runs with agent and role, prints one run's prompt whole, its steps in order and the files it changed.
    - With `history.db` replaced by a directory the same round still ends merged, and `agent status` shows lost
      records.
    - `git grep -n 'CREATE TABLE' -- crates` finds the schema in one module only.
[ ] 8.5 The history of a pull request, and the story of a round
    The rest of what happens in a round is recorded, each with its time, team and round:
    - a gate: its task, head, whether the tree was clean, the outcome, how long it took, the file of its output;
    - the checks before a review (8.2): each issue found, and what followed — sent back to the team (which pass),
      asked of the human, allowed by the human;
    - the pull request: opened (number, branch, head, title, the body whole), every push (the new head), marked
      ready, each review asked (the reviewer, its task, the review pass, the head), each review submitted (the
      verdict, the report whole), a fix pass started (its number of how many, the findings sent to the team),
      approved, merged (the merge commit) or handed to the human (`--no-merge`), closed;
    - questions asked (the file whole) and answered (the verdicts whole, the channel); controls (which, on what, by
      whom); notifications sent to the human (the text); a round closing, with how; a team ending.
    What counts as lost, as item 8.4 built it and the human confirmed: a record that cannot be written, and a
    record whose content cannot be captured — a head that cannot be read, a change snapshot that fails. Both are
    counted, each under its cause, and `agent status` shows the count. The records of this item follow the same
    rule, and a story says, in the place of what is missing, that it was lost and why.
    `agent log --round <id>` prints the story of a round and `agent log --team <id>` the stories of a team's
    rounds in order, read from the history directly — they work without the daemon. A story is in the order of
    time:
    1. the round and its team, its item in full;
    2. the agents by role and how each was chosen;
    3. every run: who, in which role, why it started, its worktree and branch, how long it took, its prompt (the
       first lines and the size; `--prompts` prints every prompt whole) and its command line, its steps numbered
       with their kind and time (`--no-steps` leaves them out), how it ended;
    4. after each run its changes: the commits, the files added, deleted, modified and renamed with their lines,
       what was left uncommitted;
    5. the gates and the checks; the pull request from opening to merge, every review pass with each reviewer's
       prompt, command line, verdict and report, every fix pass; the questions and the answers.
    `--json` prints the same records as JSON lines, each exactly once. The story is a pure function in
    `agent-core`: the records in, the lines out. An unknown ID is an error with its own exit code.
    Expected result:
    - Files: the new records in `crates/agent-core/src/history.rs` and `crates/agent-runtime/src/history/`, the
      story in `crates/agent-core/src/story.rs`, `crates/agent/src/commands/log.rs`, the places of
      `crates/agent/src/daemon/` that record (gate, checks, pull requests, reviews, merges, questions, controls,
      notifications), `docs/history.md`, `docs/architecture.md`, the golden files of the stories.
    - Adds: the records of gates, checks, pull requests, review passes, fix passes, questions, controls and
      notifications; the story; `agent log --round` and `--team` with `--prompts`, `--no-steps` and `--json`.
    - Tests: the ground rule on testing these items applies. Each new record written and read back equal. Stories
      as golden files, each made by a daemon run with fake CLIs and a fake GitHub server, IDs and times
      normalised: a solo round merged at the first review pass; a team round with a worker, one fix pass and a
      second review pass that approves; a round the checks send back once and then ask the human about; a
      `--no-merge` round; an abandoned round; a round with a question answered; texts with non-ASCII and very long
      lines. In each, the reviewer's prompt of every review pass equals what the fake CLI got. The story as a
      function: every record of the round is in `--json` exactly once (generated histories, seeded); records with
      the same time keep the order they were written in; two teams running at once give stories that share no
      record and together hold every record. `agent log --round` with no daemon running; an unknown round; a round
      of a database written by 8.4 only (no pull request yet).
    - Why: the log the human needs first — one round from its task to its merge, readable from top to bottom.
    Checks:
    - After a team round with one fix pass, with fake CLIs and the fake GitHub server and the daemon stopped:
      `agent log --round <id>` shows the item, the agents by role, each run with its prompt's first lines and its
      steps, the files each changed, two review passes with their verdicts, the merge.
    - `agent log --round <id> --prompts` prints the reviewers' second prompt whole; `--json | wc -l` equals the
      number of the round's records in the database.
[ ] 8.6 The daemon's own log
    What the `agent` programs themselves do and what goes wrong in them is logged, so that a failure of the system
    can be found afterwards and its logic followed. Every `tracing` event of the daemon goes to stderr as now and
    into the history: its time, level, the module, a stable event name, the message, its fields, and its team,
    round, task and run — taken from spans that every look at a team, every run, every request served and every
    call of git, the agentbus, the GitHub API and a model CLI opens. `[log] level` (`error`, `warn`, `info`,
    `debug`; default `info`) says what is kept.
    - `error` and `warn` — every deviation from the normal course, each logged once, where it is handled, with the
      whole chain of its causes: a line of a CLI's output that cannot be read, a run that ends without a status, a
      hung run, a quota or a rate limit, the bus down and what waits for it, a git or GitHub call that fails or is
      tried again, a gate that cannot start, times out or leaves a dirty tree, a state that cannot be saved, a
      questions file that is not valid, a name the configuration no longer has, what a restart found and
      corrected, a request refused as invalid, a panic (a panic hook logs it before the process ends).
    - `info` — every change of the state: a team, a round, an item's phase, a task's status, a run starting and
      ending, with what changed.
    - `debug` — the logic: every decision of `agent-core::decide` the daemon takes, with what it was given (in
      short) and what it decided, unless it decided that nothing is to be done; every outside call with how long
      it took and how it ended.
    The event names are an enum (`strum`), and `docs/log.md` lists each with its level and when it happens. A look
    that changes nothing logs nothing; the same warning again and again is one row with a count. A failure to
    write the log is never itself logged into the history: it goes to stderr.
    Expected result:
    - Files: `crates/agent-core/src/log.rs` (the event names and levels), the layer in
      `crates/agent-runtime/src/history/log.rs`, the subscriber in `crates/agent/src/main.rs`, the spans and events
      in `crates/agent/src/daemon/` and `crates/agent-runtime/src/`, `[log]` in `crates/agent-core/src/config.rs`,
      `config/agents.example.toml`, `docs/log.md`, `docs/configuration.md`.
    - Adds: the log in the history; the spans with the four IDs; the event names; the levels; the panic hook.
    - Tests: the ground rule on testing these items applies. For every event name a test provokes its cause with
      the fakes and finds the event with its level and its IDs; one more test walks the enum and fails for a name
      no test produces. A daemon that looks at an idle team a hundred times writes no row at `info`. The same
      warning five times is one row with the count 5. Every level keeps what it should and drops the rest. Events
      inside a run carry the team, the round, the task and the run; events of a look carry the team. A decision
      that does something is one `debug` row with its input and result; one that does nothing is none. A history
      that cannot be written: the events are on stderr, nothing loops, the daemon goes on. A panic in a task of the
      daemon is logged with its message and location. Marker secrets, as in 8.4, are not in the log.
    - Why: until now a failure of the harness itself left nothing a human could read afterwards.
    Checks:
    - With fake CLIs: a run whose CLI prints a line that is not JSON leaves a `warn` row with the event's name, the
      task and the run; with `[log] level = "debug"` the assignment of an item is a row with its input and result.
    - `docs/log.md` lists every name of the enum (a test compares them).
[ ] 8.7 `agent log`: questions to the history, search, following and what is kept
    `agent log` with no words prints the newest rows of the daemon's log. It reads the history directly and narrows
    by `--team`, `--round`, `--task`, `--run`, `--agent`, `--level`, `--event`, `--since` and `--until`, alone or
    together; `-n` limits, `--json` prints JSON lines, `-f` follows: rows written later are printed as they come,
    each once, in order, across a restart of the daemon. `--grep TEXT` searches prompts, steps, final messages,
    review reports, questions, answers and log messages (FTS5); the text is taken as text, whatever it holds.
    For a round or a team, `--show` gives one thing only: `tasks` (with role, agent and status), `agents` (with
    role and how chosen), `worktrees` (with branch and head), `runs`, `prompts`, `changes`, `reviews`, `log`, and
    `usage`, which says that tokens are not recorded yet (11.1).
    What is kept: `[history] keep_days` — the daemon deletes, at its start and once a day, the closed rounds older
    than that with everything of theirs (runs, steps, changes, prompts, reviews, their log rows and the files of
    their gates' output), the teams all of whose rounds are gone, and the log rows of no round older than `[log]
    keep_days`; `0` keeps everything; an open round is never deleted. The file shrinks afterwards
    (`auto_vacuum`). `agent log --stats` shows the size of the database, the rows of each table and the oldest
    record. The task's `activity.jsonl` is no longer written: what read it — `show` of a task — reads the steps
    from the history.
    Expected result:
    - Files: `crates/agent/src/commands/log.rs`, the queries and the search index in
      `crates/agent-runtime/src/history/`, the deleting in `crates/agent/src/daemon/history.rs`, the removal of the
      activity file from `crates/agent-runtime/src/store.rs` and its readers, `[history] keep_days` and `[log]
      keep_days`, `docs/history.md`, `docs/log.md`, `docs/configuration.md`, `docs/state.md`.
    - Adds: the filters, `--grep`, `-f`, `--show`, `--stats`; the search index; the deleting; the end of
      `activity.jsonl`.
    - Tests: the ground rule on testing these items applies. On a history written by daemon runs of two teams and
      three rounds with fake CLIs: every filter alone and every pair of filters gives exactly the rows a test
      works out from `--json` of the whole history; every `--show` for a round and for a team. Search: a word found
      in a prompt, in a step, in a report and in a log message; Polish letters; a text with quotes, `%`, `*`, `"`
      and the words `AND`, `OR`, `NEAR` is searched as text and breaks nothing. Following: rows written after the
      start come in order and once, also when the daemon restarts and when a deletion runs meanwhile. Deleting:
      exactly the closed rounds older than the limit are gone, from every table (the rows of each table counted
      before and after), with their gate files; an open round of any age stays; a team with one old and one new
      round stays with the new one; `keep_days = 0` deletes nothing; reading while the deletion runs sees whole
      rounds only; the file is smaller afterwards. Every query of `agent log` uses an index: `EXPLAIN QUERY PLAN`
      names one, for a history of a hundred thousand steps. The existing tests that read `activity.jsonl` read the
      history instead; they are listed in the PR.
    - Why: the questions asked of a round and of a team — its logs, tasks, agents, worktrees, prompts — each have
      one command, and the home no longer grows without end.
    Checks:
    - `agent log --round <id> --show prompts` prints the prompts of that round and of no other; `--show agents`
      lists who took part, with roles.
    - `agent log --grep '<a word of one prompt>'` finds the run that got it; `agent log -f` shows a run starting
      while it is watched.
    - With `keep_days = 1` and the clock moved two days on, a closed round is gone from `agent log --stats` and an
      open one is still there.
[ ] 8.8 A run that reaches its turn limit goes on
    A model CLI may end a run because a budget of its own ran out, not because the work failed: Claude Code ends
    with `error_max_turns` when a run has used its `--max-turns`. Until now such a run has no `STATUS:` line, so its
    task is `failed` and the work stays in the worktree with nobody to go on with it — also when the agent had
    finished and only its last message was missing. It becomes an end of its own, `turn_limit` (a `RunEnd`, in the
    state, the protocol and the history): the task is queued and resumed in its session with a short text that says
    what happened and asks it to go on from where it was. That happens at most `[limits] turn_limit_resumes` times
    in a row (default 3; a run that ends in any other way starts the count again); then the task is `failed` and
    the human is told, with the count. The limit is no longer a constant of the adapter: `[limits] max_turns`
    (default 300) is given to every CLI that has such a limit; an adapter whose CLI has none never reports this
    end. The versions of the state and of the protocol follow the rules of `docs/state.md`.
    Expected result:
    - Files: `crates/agent-runtime/src/cli/claude.rs`, the stream's outcome in `crates/agent-core/src/cli.rs`,
      `crates/agent-core/src/decide/outcome.rs`, `crates/agent-core/src/state/run.rs`, the settling of a run in
      `crates/agent/src/daemon/`, `[limits]` in `crates/agent-core/src/config.rs`, `config/agents.example.toml`,
      the text in `prompts/harness/run/`, `docs/architecture.md`, `docs/configuration.md`, `docs/state.md`.
    - Adds: the end `turn_limit`; its place in the table of what a run's end does; the count and its limit;
      `max_turns` and `turn_limit_resumes`; the text of the resumed run.
    - Tests: the ground rule on testing these items applies. The adapter: recorded output of Claude Code that ends
      with `error_max_turns` is this end; one that ends with `error_during_execution` is not; a `success` whose
      message has a `STATUS:` line is unchanged. The decision: a table — the first, the last allowed and the first
      refused resume, the count starting again after another end, a quota that runs out in the same run (the quota
      wins). The daemon with a fake CLI that ends at its turn limit: the task is queued and its next run resumes the
      same session with the text; a fake CLI that finishes on the resumed run ends the task as it would have; with
      `turn_limit_resumes = 1` the second such end fails the task and notifies the human once; the command line of
      the fake CLI carries `max_turns` of the configuration; a daemon restarted between the end and the resume
      resumes once. The history holds the end and the resumed run with why it started.
    - Why: a run that used up its turns is not a failure; treated as one it loses finished work, as happened with the
      old harness on item 8.3.
    Checks:
    - With a fake CLI that prints a `result` of `error_max_turns` on its first run and `STATUS: DONE` on its second:
      the task ends `done` after two runs and `agent status` never shows it `failed`.
    - With `[limits] max_turns = 50` the fake CLI is started with `--max-turns 50`.
[ ] 8.9 A round outlives its team: continuing a round
    Work that was interrupted is taken up where it stands, by the same agents or by others. A team may break — its
    coordinator gone, its agents out of quota, a stop — while the round's work is in its worktrees. Until now such a
    round is closed as `abandoned` and a new `agent run` starts from nothing and cannot be pointed at the worktree.
    State version 3: a round belongs to the state, beside the teams, and names the teams that worked for it, oldest
    first; a round whose team is over stays open, without a team, until it is continued or the human ends it
    (`agent abandon <round>`). A task is a place of work: its ID, its worktree and its branch never change; who
    holds it may. A task records its holders, oldest first, and may have none (vacant). Invariants, checked by the
    API and when read: a round has at most one team that is not over; a task belongs to one team at a time; an
    agent holds at most one task of a team.
    `agent run --continue <round> <agents...> ["addition"]` forms a team as `agent run` does and gives it the
    round: every task that worked for the round moves to the new team and gets its holder there by role — the
    lead's task to the new coordinator (or the solo agent), the item's task to the member that gets the item, a
    task nobody fits stays vacant and is shown. The round goes on from the phase its item is in: its branch, its
    pull request, its review passes and its fix passes are kept; a review asked of an investigator that is now in
    the team is asked again of another. A new holder starts a new session in the task's worktree. Before its first
    run the daemon takes a snapshot: a commit of everything in the worktree, uncommitted and untracked files
    included, made without touching the index or the files, kept under `refs/agents/snapshots/<round>/<task>/<n>`
    and recorded in the history. The first prompt of a new holder is a briefing made from the history: the item;
    who worked here before and how each run ended; the commits made; what is uncommitted (from the snapshot); the
    last final message; the questions answered so far; and the rule: find out the real state first — run the tests
    — and trust commits and tests, not what was said.
    `agent run` of a list with an item that has an open round in the same repository (the same item ID and title)
    is refused: it names the round and the two ways on, `--continue <round>` or `--new` (the old round is closed as
    `abandoned`, a new one opens). `agent status <round>` shows a checkpoint for every task of the round — the key
    is the round and the task: its state, worked out and not stored (`OPEN`: it has work and no holder that can do
    it; `BUSY`: its holder is at work or waits for something; `CLOSED`: the round is closed), its holder and the
    holders before, its worktree, branch and head, its last snapshot.
    Expected result:
    - Files: the round and the task's holders in `crates/agent-core/src/state/`, version 3 and its migration in
      `crates/agent-core/src/state.rs`, the continuation in `crates/agent-core/src/decide/`, `--continue`, `--new`
      and `abandon` in `crates/agent-core/src/cli.rs`, `crates/agent-core/src/protocol/` and
      `crates/agent/src/commands/`, the snapshot in `crates/agent-runtime/src/git.rs`, the daemon in
      `crates/agent/src/daemon/owner/`, the briefing in `prompts/harness/continue/`, the fixtures of version 3,
      `docs/state.md`, `docs/architecture.md`, `docs/history.md`.
    - Adds: rounds beside the teams; holders and vacant tasks; state version 3 with its migration; `agent run
      --continue` and `--new`; `agent abandon`; the snapshot; the briefing; the checkpoints in `agent status`.
    - Tests: the ground rule on testing these items applies. State: each invariant refused by the API and when
      read; a state of version 2 loads as the pinned state of version 3, twice the same; generated sequences
      (seeded) of opening, moving, continuing and abandoning save and load to the same text. The continuation as a
      decision: a table — a solo round, a team round in each phase of its item (`in_progress`, `in_review`,
      `fixing`, `approved`), a new team smaller and larger than the old, no agent for a task, a reviewer now in
      the team. The snapshot, in a real repository: modified, staged, deleted, untracked and ignored files; the
      index and the files are byte for byte what they were; two snapshots of an unchanged worktree are one tree.
      End to end with fake CLIs and the fake GitHub server: a solo run killed in the middle of its work with
      uncommitted files is continued by another agent, whose fake CLI gets the briefing (golden text, IDs and times
      normalised) in the same worktree, finishes, and the round merges; a team stopped while its item is `fixing`
      is continued by a new team on the same pull request and the fix pass count goes on; a continued round keeps
      its ID and names both teams; `agent run` of the same item is refused with both ways named; `--new` abandons
      and opens; a daemon restarted in the middle of a continuation continues once.
    - Why: the incident behind this feature — the worktree was there and nobody could be pointed at it.
    Checks:
    - With fake CLIs: a solo task is stopped after its fake CLI changed two files without committing; `agent run
      --continue <round> <another agent>` starts in the same worktree, and its prompt names both files.
    - `git show refs/agents/snapshots/<round>/<task>/1 --stat` lists the two files; `git status` in the worktree is
      what it was.
    - `agent status <round>` shows the task `OPEN` after the stop and `BUSY` after the continuation, with both
      holders.
[ ] 8.10 The plan of every task
    A task that works for a round keeps a plan the harness can read: what it means to do, what is done, what it is
    doing. Only the agent knows this, and after an interruption it is what the next holder needs most. The plan is
    a file in the worktree, `.agent/plan.md`, kept out of git by the harness, in the task-list format
    (`docs/task-list.md`) read in a plan mode: the features are stages, the items are steps, and a step is `[ ]` to
    do, `[>]` in hand (at most one in the file), `[x]` done or `[-]` given up — then its text says why. A step is
    known by its ID: an ID never disappears and is never used again.
    The agent's side, in the texts of `prompts/harness/plan/` (the same for every repository): write the plan
    before the first change; mark a step in hand before starting it and done when its commits are made; add steps
    whenever the work shows them; give a step up with its reason instead of deleting it; end the run when a stage
    is complete, with `STATUS: NEXT`. `[plan] roles` lists the roles that must keep a plan; this item knows `solo`,
    `member` and `coordinator`, and all three are the default.
    The harness's side: the daemon reads the plan whenever the file changes and at the end of every run, and puts
    every new version into the history with its run. A run of such a task is sent back, with the reason, when it
    ends with `DONE` while steps are to do or in hand, ends without a plan, ends with a plan that does not parse
    (the parser's error and line), or lost a step that the version before had. That happens at most `[plan]
    bounces` times in a row (default 3); then the human is asked. A run that has made `[plan] warn_steps` steps
    (default 30) with no plan is a warning in the log. `STATUS: NEXT` queues the task again; with `[plan]
    new_session_per_stage` (default on) the next run starts a new session whose prompt is the briefing of 8.9 with
    the plan in it, so a long piece of work is not one conversation that every turn reads again. A new holder's
    briefing carries the plan too.
    A conversation also ends when the agent does not end it. From this item on, with `[plan]
    new_session_on_turn_limit` (default on), a run that reached its turn limit (8.8) is not resumed in its session:
    the next run starts a new session with the same briefing, so `[limits] max_turns` is also the longest a
    conversation gets. And a run that ends because its context window is full — until now a failure — is handled
    the same way, counted with the turn-limit resumes against `turn_limit_resumes`. Both keep the worktree, the
    branch and the plan; the briefing says why the session is new.
    Expected result:
    - Files: the plan in `crates/agent-core/src/plan.rs` (on the task-list parser), the outcome `NEXT` in
      `crates/agent-core/src/status.rs` and its transitions, the decision in `crates/agent-core/src/decide/`, the
      watching and the sending back in `crates/agent/src/daemon/`, the plan's versions in
      `crates/agent-runtime/src/history/`, `[plan]` in `crates/agent-core/src/config.rs`, `prompts/harness/plan/`,
      `docs/task-list.md`, `docs/architecture.md`, `docs/configuration.md`, `docs/history.md`.
    - Adds: the plan format and its parser; the versions in the history; the four reasons to send a run back and
      their limit; `STATUS: NEXT` and the new session per stage; the new session after a turn limit and after a
      full context window; the plan in the briefing; `agent log --show plan`.
    - Tests: the ground rule on testing these items applies. The parser: every mark; two steps in hand; a step
      given up without a reason; an unknown mark; an ID used twice; a file of stages only; each with its line in
      the error. Versions: a step removed, an ID used again, a done step turned back are found; an added step, a
      new stage and a reworded title are not. The decision: a table of every reason, the count, its start again
      after a clean run, and the human asked after the limit. End to end with fake CLIs that write the file: a run
      that ends `DONE` with a step to do is sent back with that step named and finishes on its second run; three
      runs without a plan ask the human; `STATUS: NEXT` after a complete stage starts a new session (another
      session ID) whose prompt holds the plan and the commits, and with the option off resumes the same session; a
      `NEXT` with the stage not complete is sent back; a fake CLI that ends at its turn limit is followed by a run
      with another session ID whose prompt holds the plan, the commits and the reason, and with the option off by a
      run that resumes the same session (8.8); a fake CLI that reports a full context window is followed by a new
      session and the task does not fail, and after `turn_limit_resumes` such ends in a row it fails and the human
      is told once; every version is in the history once, identical ones not
      twice; a role left out of `[plan] roles` is not sent back; a daemon restarted between the end of a run and
      its sending back sends it back once.
    - Why: a plan outside the conversation is what makes work resumable by anybody — and lets a long task run as
      several short conversations instead of one that costs more with every turn.
    Checks:
    - With a fake CLI that ends `DONE` leaving `[ ] 1.2` in its plan: `agent status` shows the run sent back, and
      `agent log --round <id> --show plan` shows the versions of the plan with their times.
    - With a fake CLI that completes stage 1 and ends `NEXT`: the next run has another session ID and its prompt
      shows stage 1 done.
    - With a fake CLI that ends at its turn limit: the next run has another session ID, works in the same worktree,
      and its prompt says the session is new because the turns ran out.

9. Dashboard
Depends on: 8
[ ] 9.1 The dashboard
    `agent-web` (axum): listens on loopback only; requests from `tailscale serve` are accepted only for the
    configured logins, checked through the Tailscale LocalAPI (no `tailscale` process); a client of the daemon
    socket. Server-rendered HTML with htmx, refreshed live by server-sent events relayed from the daemon's
    subscription: teams with their rounds, tasks with their status and task-list progress; a round's view with its
    story as `agent log --round` prints it (the history, read directly and read-only); a task's view with its recent
    steps (from the history), its result and its PR; the answer form for a pending questions file; stop, pause,
    unpause and resume. systemd user unit.
    Expected result:
    - Files: `crates/agent-web/src/`, its templates and static files (htmx vendored with its version and licence),
      `config/systemd/agent-web.service`.
    - Adds: the dashboard's routes, the login check, the server-sent events relay, the answer form and the controls.
    - Tests: pages rendered against a fake daemon socket; a login allowed and one refused through a fake LocalAPI;
      events relayed; an answer and each control sent to the daemon; a round's page against a history written by a
      daemon run with fake CLIs, equal to the story of that round.
    - Why: the human's view of the teams, and a second way to answer and steer.
    Checks:
    - With a fake daemon socket: the start page lists the teams; a login not allowed gets 403; a request not from
      loopback is refused.
    - An answer posted from the form reaches the daemon; a change of state shows on the page without a reload.
    - A round's page shows that round's story: its item, its agents by role, its runs, its pull request.

10. Cutover
Depends on: 8, 9
[ ] 10.1 Install and documentation
    `install.sh` builds and installs `agent`, `agent-web` and `agentbus` with their systemd units; `README.md`
    rewritten for the new commands, configuration and behaviour.
    Expected result:
    - Files: `install.sh`, `README.md`, the user-facing pages under `docs/`.
    - Adds: installation of the three binaries and their units; documentation of the new commands and settings.
    - Tests: the installation into a temporary prefix, described with its result in the PR.
    - Why: the new `agent` can replace the old one on the machine.
    Checks:
    - `./install.sh` into a temporary prefix installs the three binaries and their units; `agent --version` from there
      works.
    - The README describes every command `agent --help` lists.
[ ] 10.2 Remove the old implementation
    Delete `bin/agent`, `agent/`, `live/`, their Bats tests and their Makefile and CI targets; keep `agentbus/`.
    Mechanical removal only (PR-6), with the exact list in the PR description.
    Expected result:
    - Removed: `bin/agent`, `agent/`, `live/`, their Bats tests, their Makefile and CI targets.
    - Tests: the gate green without them.
    - Why: one implementation left, the new one.
    Checks:
    - `git grep -nE 'bin/agent|live/'` finds no reference outside history in `docs/`.
    - The repository's gate passes without the removed parts.

11. Usage
Depends on: 10
[ ] 11.1 The tokens of every run
    Every adapter reads the token usage its CLI reports — input, output, cache read, cache written, reasoning where
    the CLI gives it, by model — and the history keeps it for the run: one row for each model a run used. What a
    CLI reports as a running total and what as an addition is decided per CLI from recorded output, documented in
    `docs/history.md` and never counted twice, also for a session that a later run resumes. A CLI that reports
    nothing leaves the run "not reported", never zero. `agent log --round <id> --show usage` and `--team <id>
    --show usage` print the tokens by agent and role, by model, and in total; a coordinator's run that worked for
    two rounds is shown in both, marked, and counted once in the team's total. No money: prices are out of scope.
    With the tokens known, the size of a conversation can end it: `[limits] fresh_session_tokens` (default 0: off)
    — when the last run of a task read more input than that a step on average, its next run, whatever starts it,
    is a new session with the briefing of 8.9 and the plan, as after a turn limit (8.10).
    Expected result:
    - Files: `crates/agent-runtime/src/cli/{claude,codex,agy,opencode}.rs`, the usage in
      `crates/agent-core/src/cli.rs` and `crates/agent-core/src/history.rs`, the table and its migration in
      `crates/agent-runtime/src/history/`, `crates/agent/src/commands/log.rs`, recorded output of each CLI under
      the tests' fixtures with the CLI's version, `docs/history.md`, `docs/state.md`.
    - Adds: the usage of a run in each adapter's output; the usage table (a schema migration); `--show usage`;
      `fresh_session_tokens`.
    - Tests: the ground rule on testing these items applies. Per CLI, from its recorded output: one usage report,
      several (a running total, additions), two models in one run, none, a run killed after its first report, a
      resumed session. The threshold: a run just under `fresh_session_tokens` a step is resumed in its session,
      one just over starts a new one with the briefing, `0` never does, and a run that reported no usage never
      does. The sums: for generated runs (seeded) a round's total is the sum of its runs and a team's
      total counts every run once. The migration: a database of the schema before this item gains the table and
      loses nothing. End to end with fake CLIs that report usage: `--show usage` of a round and of a team as golden
      text.
    - Why: what a round cost in tokens, and which agent and role spent them.
    Checks:
    - After a round with fake CLIs that report usage: `agent log --round <id> --show usage` lists every agent with
      its role and tokens; the total equals the sum of the rows.
    - A run of a fake CLI that reports no usage is shown as "not reported".

12. Taking work over
Depends on: 10
[ ] 12.1 Accounts, providers and taking work over
    The daemon gives the work of an agent that is gone to one that is free, without the human. The configuration
    says what the names cannot: every agent, investigator and worker has a `provider` and an `account` (required;
    two agents with the same pair share their limits). A quota that runs out marks the whole account exhausted
    until its reset: no run starts on any agent of it, and every task those agents hold is paused with it.
    An agent is gone from its task when the task is `failed`, when its agent left (`EXCLUDED`) or is no longer in
    the configuration, or when its account is exhausted and the reset is further away than `[takeover]
    wait_minutes` (default 15; with no reset time given, the pause of `[limits]` counts). A stop or a pause by the
    human is never such a reason. An agent is free when it holds no task, or its task waits with no item under way
    and nothing queued, and its account is not exhausted; with `[takeover] waiting_for_review_is_free` (default
    off) a member whose item is in review is free too. Who may take the task, in this item for the crew:
    - a normal agent's: another normal agent of the team, or an investigator of the team; never a worker;
    - an investigator's: another investigator — a free one of the team first, else one of the configuration from
      outside it, who joins the team, and only while two investigators stay outside it for the reviews;
    - the coordinator's: by the rule that chooses a coordinator, among the free.
    Among several the choice is random. The decision is a pure function and the daemon's alone; the coordinator is
    told by a message and may ask for another holder within the same rules. The new holder takes the task as in
    8.9 — the same worktree and branch, a snapshot, a new session, the briefing with the plan — and the task it
    held before becomes vacant. The agent that was gone, once it is back, is free: it does not get its old task
    back, and may be given a vacant one or another's work by the same rules. With nobody free the existing course
    stays: an exhausted account is waited for and its agent woken at the reset; a failed task stays failed, the
    human is told, and the round shows it `OPEN`.
    Expected result:
    - Files: `provider` and `account` in `crates/agent-core/src/config.rs`, the exhausted accounts in the state,
      the decision in `crates/agent-core/src/decide/takeover.rs`, the daemon in `crates/agent/src/daemon/owner/`,
      the texts in `prompts/harness/takeover/`, `[takeover]`, `config/agents.example.toml`,
      `docs/configuration.md`, `docs/architecture.md`, `docs/state.md`.
    - Adds: providers and accounts; an account's exhaustion; when an agent is gone and when one is free; the
      takeover decision for the crew; the change of holder with what 8.9 gives it; the message to the coordinator
      and its request; the records in the history.
    - Tests: the ground rule on testing these items applies. Configuration: an agent without `provider` or
      `account` is refused with its name. Accounts: one agent's quota pauses every task of its account and none
      of another; the reset wakes them all once. The decision: a table of every tier, every reason for being gone
      and every reason that is not one (a stop, a pause, a reset nearer than the limit), free and not free with
      the option on and off, the guard of two outside investigators at its edge (three configured, two, one), no
      candidate, several candidates with a seeded draw. End to end with fake CLIs, one of which reports an
      exhausted quota with a reset in two hours: a free member on another account takes the task in the same
      worktree with a briefing that names the uncommitted files, and the round merges; with the reset in five
      minutes nobody takes over and the same agent is woken; an agent of the same account is never chosen; the
      agent that returns holds no task and takes the next work; a member whose own item comes back from review
      while it holds another's task: that item's task is taken by a free agent by the same rules; the
      coordinator's request for another holder is followed within the rules and refused outside them; a daemon
      restarted in the middle of a takeover ends with one holder.
    - Why: several subscriptions exist so that work goes on when one of them is used up.
    Checks:
    - With fake CLIs on two accounts and a fake quota error with a reset two hours away: `agent status <round>`
      shows the task `OPEN`, then `BUSY` with the second agent as holder and the first before it.
    - The same with both agents on one account: nobody takes over, and the task is woken at the reset.
    - `agent log --round <id>` shows the takeover: who was gone and why, who took the task, the snapshot.
[ ] 12.2 Checkpoints of reviewers and workers
    The two roles outside the crew's own work get the same safety, each in its own way.
    A reviewer keeps a plan like any task (8.10) — its stages are review passes, its steps the steps of its review
    rules — and writes what it finds as it goes into `.agent/findings.md`, in the form of its report's findings. A
    reviewer that is gone (12.1) is replaced by another investigator of the configuration that is free, outside
    the team, not the other reviewer of the pull request and not on an exhausted account; with none, the review
    waits as it does today. The new reviewer takes the task and its worktree and gets the findings so far as leads:
    it checks each one itself and reads the whole diff again — the steps the reviewer before marked done are shown
    and are not taken as done, because a review is one mind over the whole change. Its plan starts anew.
    `reviewer` and `worker` join `[plan] roles` and its default.
    A worker's request that was being served when its worker went (a failed run, a provider that refuses, a
    daemon that stopped) goes back to the queue with its checkpoint: the task with its worktree, the snapshot, the
    plan. It is taken by a free worker drawn at random among those of another provider, and among those of the same
    provider only when there is no other; the new worker takes the task as in 8.9. The request is answered once,
    by whoever finishes it, and the requester is told who that was.
    Expected result:
    - Files: the reviewer's plan and findings in `crates/agent-core/src/plan.rs` and `prompts/harness/plan/`, the
      decisions for reviewers and workers in `crates/agent-core/src/decide/takeover.rs`, the requests in
      `crates/agent/src/daemon/team/`, the reviews in `crates/agent/src/daemon/owner/rounds/`, the texts in
      `prompts/harness/takeover/`, `docs/architecture.md`.
    - Adds: the reviewer's plan and findings file; the takeover of a review with findings as leads; the return of
      a worker's request to the queue with its checkpoint; the draw of a worker by provider.
    - Tests: the ground rule on testing these items applies. The decisions: tables — a reviewer replaced, the
      other reviewer never chosen, an investigator of the team never chosen, an exhausted account never chosen,
      none free; a worker of another provider drawn (seeded), the same provider only without another, none free.
      End to end with fake CLIs and the fake GitHub server: a reviewer whose fake CLI wrote two findings and
      failed is replaced; the new reviewer's prompt holds both findings as leads and the instruction to read the
      whole diff, its plan has no step done, and its verdict closes the review pass; the review pass keeps its
      number and names both reviewers; a worker killed in the middle of a request: the request is served by a
      worker of the other provider in the same worktree, answered once, and the requester's message names it; a
      daemon stopped while a request is served returns the request at its start (the known limit of 7.1 is gone).
    - Why: a review or a request cut off in the middle costs its whole time again, and until now a request in
      hand was lost with the daemon.
    Checks:
    - With fake CLIs: a reviewer that fails after writing a finding is replaced and the second reviewer's prompt
      shows that finding under leads; the pull request still gets two verdicts.
    - A worker's request whose worker is killed is finished by a worker of another provider; `agent log --round
      <id>` shows both runs and one answer.
