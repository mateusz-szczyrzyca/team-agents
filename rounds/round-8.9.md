Round: rust-agent/8.9

This round: item 8.9 of the plan "Rewrite `bin/agent` in Rust" — this item only. Its full text, with its
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

This round's item

8. Autopilot, rounds, the history and checkpoints
Depends on: 7 (merged before this round)
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
8. Autopilot, rounds, the history and checkpoints
   8.1 Pull requests, reviews and the merge loop
   8.2 Round checks before review
   8.3 Rounds as an entity
   8.4 The history: a database of what happened, and what every run did
   8.5 The history of a pull request, and the story of a round
   8.6 The daemon's own log
   8.7 `agent log`: questions to the history, search, following and what is kept
   8.8 A run that reaches its turn limit goes on
   8.9 A round outlives its team: continuing a round   <- this round
   8.10 The plan of every task
9. Dashboard
   9.1 The dashboard
10. Cutover
   10.1 Install and documentation
   10.2 Remove the old implementation
11. Usage
   11.1 The tokens of every run
12. Taking work over
   12.1 Accounts, providers and taking work over
   12.2 Checkpoints of reviewers and workers
