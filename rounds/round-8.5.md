Round: rust-agent/8.5

This round: item 8.5 of the plan "Rewrite `bin/agent` in Rust" — this item only. Its full text, with its
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
   8.4 The history: a database of what happened, and what every run did
   8.5 The history of a pull request, and the story of a round   <- this round
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
