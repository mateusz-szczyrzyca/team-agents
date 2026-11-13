Round: rust-agent/8.4

This round: item 8.4 of the plan "Rewrite `bin/agent` in Rust" — this item only. Its full text, with its
expected result and its checks, is under "This round's item". The rules below hold for every round. The overview at
the end lists the other items for context only: do not do their work, and do not start a later item.

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
  `notify`, `rand`, `time`, `strum` (`derive`), `rusqlite` (`bundled`; the history, from item 8.4).
- An enum with a text form (a keyword enum: a CLI kind, a status, a verdict) takes it from `strum` derives: each text
  declared once, in a `#[strum(...)]` attribute. No hand-written text tables, no macro of our own for it.
- New code is developed and tested against a separate `AGENTS_HOME`; nothing here may touch the live `~/agents` of
  the running Bash harness until feature 10.
- The items of rounds, of the history and of checkpoints (8.3 to 8.8, 11.1, 12.1 to 12.4) are tested harder than
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
- After feature 8, before feature 10: a real run with cheap models on a toy repository and a separate
  `AGENTS_HOME`, in the default `--autopilot`: from a two-item list to two merged PRs. Then `agent log --round` of
  each: the story is whole — the task, who was chosen and how, every prompt, every step, every change, both
  reviews — and `agent log --level warn` shows nothing unexplained. The cutover starts only after it passes.
- After feature 9: the dashboard opened through `tailscale serve` from another device: the teams listed, a question
  answered, a team paused and resumed.
- After item 12.1: stop a real solo task in the middle of its work and continue its round with another agent: it
  works in the same worktree and its first message shows that it knows what was done.
- After item 12.3: read the `provider` and `account` of every agent in the live `agents.toml`; they decide who may
  stand in for whom.

Out of scope for this plan: remote nodes and the lead, investigation teams with phases and standalone investigation
requests, `agent live`, archiving, the cost of tokens in money, moving the state file into the database, choosing
agents by the quota their accounts have left (load balancing), automatic PR size limits (reviewers check PR-1), a
proof by the harness that a RED commit fails (a reviewer runs one, TDD-1), an `agentbus` socket API (it is called
as a process).

This round's item

8. Autopilot, rounds and the history
Depends on: 7 (merged before this round)
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

Plan overview (context only)

1. Workspace and core vocabulary
   1.1 Create the Cargo workspace, its gate and the core vocabulary   (done)
2. Core models and contracts
   2.1 Configuration and the task list file   (done)
   2.2 State, control protocol and questions files   (done)
   2.3 Prompt catalog and the model CLI contract   (done)
   2.4 Replace the `keyword_enum!` macro with strum   (done)
3. Core decisions
   3.1 Team formation, assignment, the autopilot round and run outcomes   (done)
4. Runtime services
   4.1 State store, prompts, processes, git and notifications   (done)
   4.2 GitHub API and agentbus
5. Model CLI adapters
   5.1 Claude Code, Codex, Antigravity and opencode adapters
6. Daemon and runs
   6.1 The daemon, its socket and the run engine
7. Teams, questions and the gate
   7.1 Starting work and the team's life
   7.2 Questions, answers and controls
   7.3 Gate service
8. Autopilot, rounds and the history
   8.1 Pull requests, reviews and the merge loop
   8.2 Round checks before review
   8.3 Rounds as an entity
   8.4 The history: a database of what happened, and what every run did   <- this round
   8.5 The history of a pull request, and the story of a round
   8.6 The daemon's own log
   8.7 `agent log`: questions to the history, search, following and what is kept
   8.8 A run that reaches its turn limit goes on
9. Dashboard
   9.1 The dashboard
10. Cutover
   10.1 Install and documentation
   10.2 Remove the old implementation
11. Usage
   11.1 The tokens of every run
12. Checkpoints
   12.1 A round outlives its team: continuing a round
   12.2 The plan of every task
   12.3 Accounts, providers and taking work over
   12.4 Checkpoints of reviewers and workers
