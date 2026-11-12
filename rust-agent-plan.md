Rewrite `bin/agent` in Rust — task list

This file is the plan for a new `agent`, written from scratch in Rust in this repository. The text before the first
feature is the brief every member reads; each item below is one task, one branch and one pull request.

Ground rules

- The rules, each read by its own role only:
  - Writing or fixing code: read `prompts/agent/rust-developer.md` (the Rust developer rules) first; it is binding.
    Its rule IDs (ERR-2, PR-1, ...) are the ones this plan and the reviewers cite.
  - Reviewing a PR: review against `prompts/investigator/code-review.md`, which lists every rule by its ID. Do not
    read `rust-developer.md` as a whole: open it only for the full wording of a rule a finding cites.
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
- Every item compiles and passes the whole gate on `main` without the other items of its feature. An item uses only
  code from the features named in its feature's "Depends on" line, all of them merged. Items of one feature are taken
  in order; features whose dependencies are merged run in parallel.
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
  `notify`, `rand`, `time`, `strum` (`derive`).
- An enum with a text form (a keyword enum: a CLI kind, a status, a verdict) takes it from `strum` derives: each text
  declared once, in a `#[strum(...)]` attribute. No hand-written text tables, no macro of our own for it.
- New code is developed and tested against a separate `AGENTS_HOME`; nothing here may touch the live `~/agents` of
  the running Bash harness until feature 10.

Target layout

    Cargo.toml                  workspace
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

Process model: `agent daemon` (a systemd user service) is the only owner of the state and of every model run. Every
other command — `agent run`, `answer`, `status`, `stop`, `gate`, ... — and the dashboard are clients of its Unix
socket. One owner, no file races (CON-3).

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

Checkpoints for the human, between features (the agents do not run these):

- After feature 2: read `docs/configuration.md` and `docs/state.md`. The configuration and the state decided there
  are expensive to change later.
- After feature 7: a trial on a scratch repository with fake CLIs and a separate `AGENTS_HOME`: a solo task and a
  team from a two-item list, one question answered, one stop and resume.
- After feature 8, before feature 10: a real run with cheap models on a toy repository and a separate
  `AGENTS_HOME`, in the default `--autopilot`: from a two-item list to two merged PRs. The cutover starts only after
  it passes.
- After feature 9: the dashboard opened through `tailscale serve` from another device: the teams listed, a question
  answered, a team paused and resumed.

Out of scope for this plan: remote nodes and the lead, investigation teams with phases and standalone investigation
requests, `agent live`, archiving, token usage and cost accounting, the precheck rules (B1–B4), automatic PR size
limits (reviewers check PR-1), an `agentbus` socket API (it is called as a process).

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

8. Autopilot
Depends on: 7
[ ] 8.1 Pull requests, reviews and the merge loop
    When an item's integration has a green gate at its head: push, open the PR with a body from a template, mark it
    ready; later pushes update it. Two free investigators from outside the team (the PR waits until two are free) review
    it with `prompts/investigator/code-review.md` and the item's full text, its expected result included; the verdict is
    parsed from the review rules' report format and posted on the PR. Both ready → merge through the API, then the next
    item; with `--no-merge` the human is notified instead. Changes requested → a fix pass for the team with the
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

9. Dashboard
Depends on: 6
[ ] 9.1 The dashboard
    `agent-web` (axum): listens on loopback only; requests from `tailscale serve` are accepted only for the
    configured logins, checked through the Tailscale LocalAPI (no `tailscale` process); a client of the daemon
    socket. Server-rendered HTML with htmx, refreshed live by server-sent events relayed from the daemon's
    subscription: teams and tasks with their status and task-list progress; a task's view with its recent activity,
    its result and its PR; the answer form for a pending questions file; stop, pause, unpause and resume. systemd
    user unit.
    Expected result:
    - Files: `crates/agent-web/src/`, its templates and static files (htmx vendored with its version and licence),
      `config/systemd/agent-web.service`.
    - Adds: the dashboard's routes, the login check, the server-sent events relay, the answer form and the controls.
    - Tests: pages rendered against a fake daemon socket; a login allowed and one refused through a fake LocalAPI;
      events relayed; an answer and each control sent to the daemon.
    - Why: the human's view of the teams, and a second way to answer and steer.
    Checks:
    - With a fake daemon socket: the start page lists the teams; a login not allowed gets 403; a request not from
      loopback is refused.
    - An answer posted from the form reaches the daemon; a change of state shows on the page without a reload.

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
