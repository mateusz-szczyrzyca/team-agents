Round: rust-agent/12.2

This round: item 12.2 of the plan "Rewrite `bin/agent` in Rust" — this item only. Its full text, with its
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

12. Checkpoints
Depends on: 10 (merged before this round)
[ ] 12.2 The plan of every task
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
    new_session_per_stage` (default on) the next run starts a new session whose prompt is the briefing of 12.1 with
    the plan in it, so a long piece of work is not one conversation that every turn reads again. A new holder's
    briefing carries the plan too.
    Expected result:
    - Files: the plan in `crates/agent-core/src/plan.rs` (on the task-list parser), the outcome `NEXT` in
      `crates/agent-core/src/status.rs` and its transitions, the decision in `crates/agent-core/src/decide/`, the
      watching and the sending back in `crates/agent/src/daemon/`, the plan's versions in
      `crates/agent-runtime/src/history/`, `[plan]` in `crates/agent-core/src/config.rs`, `prompts/harness/plan/`,
      `docs/task-list.md`, `docs/architecture.md`, `docs/configuration.md`, `docs/history.md`.
    - Adds: the plan format and its parser; the versions in the history; the four reasons to send a run back and
      their limit; `STATUS: NEXT` and the new session per stage; the plan in the briefing; `agent log --show plan`.
    - Tests: the ground rule on testing these items applies. The parser: every mark; two steps in hand; a step
      given up without a reason; an unknown mark; an ID used twice; a file of stages only; each with its line in
      the error. Versions: a step removed, an ID used again, a done step turned back are found; an added step, a
      new stage and a reworded title are not. The decision: a table of every reason, the count, its start again
      after a clean run, and the human asked after the limit. End to end with fake CLIs that write the file: a run
      that ends `DONE` with a step to do is sent back with that step named and finishes on its second run; three
      runs without a plan ask the human; `STATUS: NEXT` after a complete stage starts a new session (another
      session ID) whose prompt holds the plan and the commits, and with the option off resumes the same session; a
      `NEXT` with the stage not complete is sent back; every version is in the history once, identical ones not
      twice; a role left out of `[plan] roles` is not sent back; a daemon restarted between the end of a run and
      its sending back sends it back once.
    - Why: a plan outside the conversation is what makes work resumable by anybody — and lets a long task run as
      several short conversations instead of one that costs more with every turn.
    Checks:
    - With a fake CLI that ends `DONE` leaving `[ ] 1.2` in its plan: `agent status` shows the run sent back, and
      `agent log --round <id> --show plan` shows the versions of the plan with their times.
    - With a fake CLI that completes stage 1 and ends `NEXT`: the next run has another session ID and its prompt
      shows stage 1 done.

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
   12.2 The plan of every task   <- this round
   12.3 Accounts, providers and taking work over
   12.4 Checkpoints of reviewers and workers
