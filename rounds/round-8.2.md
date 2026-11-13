Round: rust-agent/8.2

This round: item 8.2 of the plan "Rewrite `bin/agent` in Rust" — this item only. Its full text, with its
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
    prompts/agent/, prompts/worker-agent/, prompts/investigator/   each role's rules (in the prompt catalog)
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
requests, `agent live`, archiving, token usage and cost accounting, automatic PR size limits (reviewers check
PR-1), a proof by the harness that a RED commit fails (a reviewer runs one, TDD-1), an `agentbus` socket API (it
is called as a process).

This round's item

8. Autopilot
Depends on: 7 (merged before this round)
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
8. Autopilot
   8.1 Pull requests, reviews and the merge loop
   8.2 Round checks before review   <- this round
9. Dashboard
   9.1 The dashboard
10. Cutover
   10.1 Install and documentation
   10.2 Remove the old implementation
