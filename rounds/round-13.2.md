Round: rust-agent/13.2

This round: item 13.2 of the plan "Rewrite `bin/agent` in Rust" — this item only. Its full text, with its
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
- From item 8.6 on, every check of an item has an ID (`A1`, `A2`, …): it is an acceptance criterion. The PR
  description answers each one under `## Acceptance` — the ID and how it was shown: the test, or the command and
  what it printed. A reviewer's report says for each one `met` or `not met`, with its evidence; one not met is a
  BLOCKER. A criterion that cannot be shown as written is said so in the PR description, with the reason.
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
- The items of rounds, of the history, of checkpoints and of review (8.3 to 8.10, 11.1 and the features 12 and 13)
  are tested harder than the developer rules ask: a log that is wrong is worse than no log, and a takeover that is
  wrong loses work.
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
- Review, after the cutover (feature 13): the checks every change must pass are written in the repository's
  `AGENTS.md` and run by the harness before any reviewer starts; what the reviewers find is kept in a ledger, the
  first review passes are full and the later ones look only at the open findings and the fixes; requests to
  workers have a form that leaves nothing to guess.

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

13. Review
Depends on: 10 (merged before this round)
[ ] 13.2 Findings that are kept, and review passes that narrow
    What a review found is kept, so that the next reviewer starts where the last one ended instead of discovering
    the pull request again. The daemon reads every finding out of a reviewer's report (the report format of the
    review rules) into a ledger of the pull request, in the state and the history: an ID that never changes
    (`F1`, `F2`, …), its severity (BLOCKER or MINOR), the rule, the file and lines at the head it was raised on,
    the text, the reviewer, the review pass, and its state — open, fixed, withdrawn. A report the daemon cannot
    read is sent back to its reviewer once, then the review counts as failed.
    The first `[review] full_passes` review passes (default 1) are full: every reviewer reviews the whole pull
    request on its own. From the second full pass on a reviewer is given the ledger too, after its instructions
    to review first and compare afterwards. Every later pass is narrowed. Its prompt holds the open findings and
    the diff of the fixes — from the head the last pass reviewed to the head now — and nothing else is to be
    reviewed: the reviewer answers each open finding `fixed` or `not fixed` with a place in the code, and may
    raise new findings only on lines the fixes changed. A finding on any other line is recorded as out of scope,
    blocks nothing and is listed for the human.
    Who reviews a narrowed pass is `[review] narrowed_reviewers`: `all` (default) — as many reviewers as a full
    pass; `requesters` — only the reviewers that asked for changes, the approval of the others stands unless the
    fixes change more than `[review] approval_expires_lines` lines; `one` — one reviewer, whose approval counts
    for all. A pull request is merged when `[review] approvals` approvals (default 2) stand at its head, no
    BLOCKER is open and the mandatory checks passed there.
    The team that fixes gets every open finding with its ID, the MINOR ones too. It may dispute one in the pull
    request's description (`## Disputed findings`: the ID and the reason); the next reviewer withdraws it or keeps
    it, and a finding disputed twice is put to the human. A pull request approved with only MINOR findings open is
    merged, the open ones listed in its description. `agent status <round>` and `agent log --round` show the
    ledger: each finding, its state and the pass that changed it.
    Expected result:
    - Files: the ledger in `crates/agent-core/src/state/` and `crates/agent-core/src/history.rs`, the report's
      parser in `crates/agent-core/src/review/report.rs`, the kind of a pass and its reviewers in
      `crates/agent-core/src/decide/round.rs`, `crates/agent/src/daemon/owner/rounds/`, the texts in
      `prompts/harness/review/`, the re-review part of `prompts/investigator/code-review.md`, `[review]` in
      `crates/agent-core/src/config.rs`, `docs/architecture.md`, `docs/configuration.md`, `docs/state.md`,
      `docs/history.md`.
    - Adds: findings with IDs and states; the report's parser; full and narrowed passes; the three ways to man a
      narrowed pass; `approvals`; disputes; the ledger in the prompts, in `agent status` and in the story.
    - Tests: the ground rule on testing these items applies. The parser: the report of the review rules whole,
      with every section, with none, with grouped places (`also:`), a line it cannot read. The ledger: the same
      finding raised by two reviewers is two findings; IDs are never used again; every change of state allowed
      and refused. The decision: a table — which pass is full, who reviews under each of the three settings, the
      approval that stands and the one that expires at the line limit, when a pull request may merge. End to end
      with fake CLIs and the fake GitHub server: two full reviews, three findings, a fix, a narrowed pass whose
      prompt holds exactly the open findings and the diff of the fixes and not the whole diff; a finding marked
      fixed closes, one not fixed goes back with its ID; a new finding on an unchanged line blocks nothing and is
      in the description; a disputed finding withdrawn, and one disputed twice asks the human; a merge with MINOR
      findings open lists them; `full_passes = 2` makes the second pass full with the ledger in it; each setting
      of `narrowed_reviewers` starts the reviewers the table says; a daemon restarted between two passes keeps the
      ledger.
    - Why: a review that starts from nothing every time costs the most and converges the least.
    Checks:
    - A1. With fake CLIs: after one fix pass the reviewers' second prompt names the open findings by ID and
      contains the diff of the fixes; `agent status <round>` shows each finding with its state.
    - A2. A finding the second pass raises on a line the fixes did not change does not stop the merge.
    - A3. With `[review] narrowed_reviewers = "requesters"` only the reviewer that asked for changes is started
      again.

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
   8.9 A round outlives its team: continuing a round
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
13. Review
   13.1 Mandatory checks: read from `AGENTS.md`, run by the harness
   13.2 Findings that are kept, and review passes that narrow   <- this round
   13.3 Requests to workers that leave nothing to guess
