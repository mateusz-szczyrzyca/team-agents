Round: rust-agent/8.3

This round: item 8.3 of the plan "Rewrite `bin/agent` in Rust" — this item only. Its full text, with its
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
- The items of rounds and of the history (8.3 to 8.7, 11.1) are tested harder than the developer rules ask: a log
  that is wrong is worse than no log.
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

Out of scope for this plan: remote nodes and the lead, investigation teams with phases and standalone investigation
requests, `agent live`, archiving, the cost of tokens in money, moving the state file into the database, automatic
PR size limits (reviewers check PR-1), a proof by the harness that a RED commit fails (a reviewer runs one, TDD-1),
an `agentbus` socket API (it is called as a process).

This round's item

8. Autopilot, rounds and the history
Depends on: 7 (merged before this round)
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
   8.3 Rounds as an entity   <- this round
   8.4 The history: a database of what happened, and what every run did
   8.5 The history of a pull request, and the story of a round
   8.6 The daemon's own log
   8.7 `agent log`: questions to the history, search, following and what is kept
9. Dashboard
   9.1 The dashboard
10. Cutover
   10.1 Install and documentation
   10.2 Remove the old implementation
11. Usage
   11.1 The tokens of every run
