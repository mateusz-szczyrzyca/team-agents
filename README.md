# team-agents

Run a crew of headless LLM coding agents on your own Linux machine and let them work on your repositories
in parallel — alone or as teams — while you watch, answer their questions and review their pull requests.

team-agents does not implement an agent loop itself. It orchestrates existing agent CLIs —
**Claude Code**, **Codex CLI**, **Antigravity CLI (`agy`)** and **OpenCode** (for any API provider such as
OpenRouter or DeepInfra) — and adds what they lack when several of them work unattended on one host:

- **Isolation:** every task runs in its own git worktree and branch.
- **One daemon:** `agent daemon` owns the state and every model run; it survives its own restart and resumes what
  was going from the sessions of the CLIs.
- **Communication:** a small MCP server (`agentbus`, Go standard library only) gives all agents a shared message
  bus, resource reservations, a per-repository notebook and checkpoints.
- **Human in the loop:** agents never guess; they write questions to a Markdown file, the task waits, you answer
  with `agent answer <task>` and the agent goes on in the same session.
- **Teams and rounds:** a coordinator and its members deliver the items of a task list one pull request at a time;
  the harness checks each round, runs the repository's gate, asks investigators for reviews and takes the pull
  request through its fix passes.
- **Load spread across providers:** agents of different CLIs and tiers work side by side, so the load and the
  token usage are distributed over several providers and accounts.
- **Observability:** `agent status`, a history of every run and step (`agent log`), a diagnostic bundle
  (`agent debug`) and notifications.

The harness is written in Rust (`crates/`). Its first implementation — a Bash script, a rewrite of it in Go, a
terminal panel and a web interface in Go — was removed in 0.25.0; `agentbus` is the one Go program left.

> **Security warning.** Agents run with their CLIs' permission prompts and sandboxes **disabled**. They can run
> any command as the user they run under, push to your repositories and use every credential that user can read.
> Read the [Security](#security) section before installing.

## Contents

- [How it works](#how-it-works)
- [Security](#security)
- [Requirements](#requirements)
- [Installation](#installation)
- [Configuration](#configuration)
- [Usage](#usage)
- [What the daemon does](#what-the-daemon-does)
- [MCP tools available to agents](#mcp-tools-available-to-agents)
- [Development and tests](#development-and-tests)
- [License](#license)

## How it works

One daemon per home directory (`AGENTS_HOME`) owns `state.json`, the history (`history.db`) and every run. The
other commands talk to it through a socket in that home. `agent run` creates a task, or a team from a task list
([`docs/task-list.md`](docs/task-list.md)), with a git worktree and a branch for each task. For every run the daemon
builds the prompt from `prompts/`, starts the agent's CLI with an MCP configuration that spawns `agentbus serve`,
records what the CLI does and acts on the `STATUS:` line of the agent's final message: done, questions for the
human, failed, sleeping.

The documents: [`docs/architecture.md`](docs/architecture.md) (the layout and the process model),
[`docs/configuration.md`](docs/configuration.md), [`docs/state.md`](docs/state.md) (what is kept and how old states
are read), [`docs/history.md`](docs/history.md), [`docs/log.md`](docs/log.md), [`docs/task-list.md`](docs/task-list.md),
[`docs/plan.md`](docs/plan.md) (the form of a task's plan of steps, not read by the daemon yet), [`docs/tdd.md`](docs/tdd.md).
[`WhatsNew.md`](WhatsNew.md) says what each version brought.

## Security

team-agents gives AI models a shell without supervision. Treat the agents' user as untrusted: anything it can
read, the agents can read — and possibly leak, if a prompt injection (instructions hidden in a web page, an issue,
a dependency's README, a file in the repository) convinces an agent to do so. Design the setup so that a
compromised or confused agent can do as little damage as possible.

### Where to run it

- **Linux only.**
- **A dedicated machine or VM** — a small server, a Raspberry Pi, a VPS or a local VM — with nothing else of value
  on it: no personal files, no production credentials, no browser sessions, no password manager.
- **A dedicated, unprivileged user** for the agents, never your own login user. Keep its `AGENTS_HOME` at mode
  `700`.
- **No sudo by default.** Passwordless sudo lets the agents install missing tools on their own, but it also lets
  them run anything as root: give it **at your own risk**, and preferably only on a disposable VM.
- **Nothing irreplaceable on the host.** Back up `agents.toml` and `secrets.toml`; be ready to rebuild the machine
  from scratch.

### Repositories and GitHub

Give the agents **their own GitHub account** ("machine user", allowed by GitHub's terms: one per person). Their
commits, pushes, pull requests and reviews then appear under that account instead of yours, their access can be
revoked in one place, and you can review and approve their PRs (an author cannot approve their own). How to grant
it access depends on who owns the repositories, because fine-grained tokens cannot be used for repositories where
their owner is only a collaborator:

- **Repositories in an organization (recommended).** Create a free organization for the projects, add the agents'
  account as a member with write access to the chosen repositories, and create a **fine-grained token** for that
  account with the organization as resource owner, only those repositories, Contents and Pull requests:
  read/write, Metadata: read, **no Workflows**, and an expiry date.
- **Repositories in your personal account.** Add the agents' account as a collaborator to the chosen repositories
  and create a **classic token** for it with the `repo` scope and **without** the `workflow` scope. A classic token
  is not limited per repository, but the account itself can reach only the repositories you shared with it.
- **No separate account.** A fine-grained token of your own account, limited as above. Commits still carry the
  agent's name ([`docs/configuration.md`](docs/configuration.md)), but pushes, PRs and reviews appear as you.

Put the token into `secrets.toml` ([`docs/configuration.md`](docs/configuration.md#secretstoml)). In every variant:

- **No permission to change workflows** (Workflows / `workflow` scope), so the agents cannot touch
  `.github/workflows/`: a modified workflow runs with your CI secrets.
- For git over SSH use an SSH key of the agents' account, or a **deploy key** per repository — never your
  personal key.
- **Protect the main branch**: require a pull request with your review before merging, block force pushes and
  deletions. The agents open PRs; merging stays with you. Consider CODEOWNERS for sensitive paths.
- Do not give agents repositories with secrets in their history or committed `.env` files; treat everything an
  agent can read as potentially public.

### Other hosts (Ansible, test servers, devices)

- When agents work on infrastructure (for example an Ansible repository tested against a VPS), generate a
  **separate SSH key for them**, used for nothing else, and authorise it **only on test hosts** — never on
  production. Restrict it in `authorized_keys` where you can (`from="<agent host>"`, `restrict`), and remove it when
  the work is done.
- Prefer **disposable test hosts** (VMs or VPS instances you can rebuild from a snapshot) over long-lived servers.
- Devices attached to the host (debug probes, serial ports) can be flashed and reconfigured by the agents: connect
  only development boards, never devices in production use.

### Network

- Keep the agents' host **off the public internet**: no open inbound ports. A mesh VPN such as **Tailscale** (or
  WireGuard) is a good fit — and use its **ACLs** to limit what the agents' host may reach: the test hosts it needs,
  not your NAS, your laptop or the rest of your network (for example a `tag:agents` that can only reach
  `tag:test-hosts`).
- Optionally restrict outbound traffic with an egress firewall (nftables, OpenSnitch) to the providers, GitHub,
  package registries and the test hosts.

### Secrets and providers

- Create **separate API keys and tokens for the agents** in every service — Anthropic, OpenAI, Google, OpenRouter,
  DeepInfra, GitHub, the notification service — never reuse your personal or production keys. Separate keys can be
  revoked without collateral damage and show the agents' usage on their own.
- Set **spending limits and usage alerts** at the pay-per-token providers (OpenRouter lets you set a credit limit
  per key; check what budget or limit settings your other providers offer) so a runaway loop cannot run up a
  large bill.
- Remember that `secrets.toml` and the CLIs' login files are **readable by the agents** — they run as the same user.
  Keep the scopes small and **rotate** the keys regularly, and immediately after anything suspicious.
- The notification command (`notifications.command`) runs as the same user: give it a token that can only send
  notifications.

### Security testing

Be careful with tasks such as "find vulnerabilities", "test the security of X" or penetration tests. An agent
looking for weaknesses may find them **on the machine it runs on** — a readable credential, a misconfigured
service, a local privilege escalation — and use them, because that is what the task asked for. Run such tasks only
against isolated, disposable targets, state the scope explicitly in the task ("only host X, nothing else"), and
never on the orchestrator's host or a network with anything you care about.

### Code, dependencies and review

- Agents install packages (pip, npm, cargo, go) — **malicious or typo-squatted packages** are a real risk. Use lock
  files, review dependency changes in the PRs, and prefer well-known packages.
- **Review every PR** before merging, especially changes to build scripts, CI configuration, install scripts and
  anything that runs with privileges. The reviews of the investigators are an addition to yours, not a replacement.
- Be wary of tasks that feed agents **untrusted input** (issues written by strangers, scraped web pages) while they
  hold write access: that is the classic prompt-injection path.

### Monitoring and response

- Ship the daemon's log ([`docs/log.md`](docs/log.md)) somewhere you look at, and alert on warnings and errors.
  `agent log` shows what a round, a team or a task did, with every prompt and step.
- Logs and the files of a task can contain secrets printed by commands: restrict who can read them.
- `agent stop <team>` stops a team; stopping the daemon (`systemctl --user stop agent-daemon`) ends every run at
  once, and nothing starts until it runs again.
- After an incident: stop everything, revoke the agents' keys and tokens (GitHub, providers, SSH), check the
  transcripts and the git history of the affected repositories, and rebuild the host if in doubt.


## Requirements

- Linux, `git`, `bash` ≥ 5, `make`.
- Rust through `rustup` (the toolchain of `rust-toolchain.toml` is installed by the first cargo command) and Go
  (the version in `agentbus/go.mod` or newer), to build `agent` and `agentbus`.
- The agent CLIs you want to use, installed and logged in for the agents' user: `claude`, `codex`, `agy`,
  `opencode`.
- A GitHub token for the agents ([Security](#repositories-and-github)).

## Installation

There is no installer. As the agents' user, in a checkout of this repository:

```sh
cargo build --release --locked -p agent              # target/release/agent
(cd agentbus && go build -o ~/.local/bin/agentbus .)
install -m 755 target/release/agent ~/.local/bin/agent

export AGENTS_HOME=~/agents-rs                       # the home of this installation
mkdir -p -m 700 "$AGENTS_HOME"
cp config/agents.example.toml "$AGENTS_HOME/agents.toml"   # then edit it
cp -r prompts "$AGENTS_HOME/prompts"                 # or start the daemon with --prompts-dir <checkout>/prompts
```

Write `secrets.toml` (mode `0600`) as [`docs/configuration.md`](docs/configuration.md#secretstoml) says, then start
the daemon — by hand (`agent daemon`, logs on stderr) or as a user service:
[`config/systemd/agent-daemon.service`](config/systemd/agent-daemon.service) says how to install it. The daemon reads
the configuration once, at its start, and the prompts from the directory it was given: after a change of either, and
after building a new version, restart it.

## Configuration

`agents.toml` (the agents and investigators, their CLIs and models, tiers, limits, identity, notifications, the
repositories) and `secrets.toml`, both in `AGENTS_HOME`. The reference is
[`docs/configuration.md`](docs/configuration.md); the template is
[`config/agents.example.toml`](config/agents.example.toml). A key the reference does not know is an error.
[`examples/notify/`](examples/notify/) has scripts for `notifications.command`.

## Usage

```
agent daemon                      runs the daemon: the owner of the state and of every model run
agent run ...                     starts a task or a team on the repository the current directory is in
agent status [--running]          the teams the daemon knows and what each of their tasks is doing
agent answer <task> [-f <file>]   answers the questions of a task; it goes on with the answers
agent say <task|team> <text>      says something to a task or a team, through the bus
agent stop | pause | unpause | resume <task|team>
agent gate                        runs the repository's gate for a task and records its result
agent log ...                     reads the history: rounds, teams, tasks, prompts, steps
agent debug --team <team>         a diagnostic bundle with a summary that names what is wrong
```

`agent <command> --help` has the options of each. The next section describes what the daemon does with a team, in
the order the features were built.

`agent run` starts a team even when one of its agents works in another open team, and after the tasks it prints a line
for each such agent: `  codex-1 is also in the open team team-20261119-165451-kfek (task task-20261119-165451-uv0g): its
messages there do not reach this team`.

`agent run` fetches the configured base branch from `origin` before creating a team, task directory or
worktree. If the local base has unpublished commits, it refuses with their count and the first commit's short ID
and subject. Push those commits, or review and manually run `git reset --hard origin/<base>` before trying again
(the reset discards local changes). Fetch errors also refuse the start. A base behind origin keeps the existing
behavior: the initial worktree starts from the local base. Repositories without `origin` keep their existing behavior.

Use `agent run --allow-unpushed-base <agents...> "task"` to explicitly start with unpublished base commits.
The CLI prints the count and first commit; the team retains that evidence across daemon restarts, and the harness
adds it under `## Unpushed base` in every pull request description. The flag does not bypass a failed fetch.
Older clients and saved teams grant no such permission.

## What the daemon does


A Claude Code run that exhausts its turn budget now resumes in the same session. Configure `[limits] max_turns`
(default `300`) and `turn_limit_resumes` (default `3`); after that many consecutive resumes the task fails and
notifies you with the count. Any other ending resets the count. `agent log` records `turn_limit` and the reason
of its continuation. This build supports legacy socket protocol `3` and split-aware protocol `4`, reads existing
state schema `2`, and migrates history to schema `8`. See [configuration](docs/configuration.md) and
[state compatibility](docs/state.md#turn-limit-compatibility-item-88).

A run ending without a `STATUS:` line resumes automatically in the same session with a short prompt
(`prompts/harness/run/resume-no-status.md`), up to twice in a row; a third failure in a row marks the task failed and
notifies the coordinator (or human). Automatic retries apply only to resumable tasks with a session (solo tasks,
coordinators, and team members), excluding workers serving requests, reviewers, tasks without a session, or failures
during startup or execution. Runs ending without a status line upon daemon shutdown or interruption (exit code `None`)
count toward the consecutive limit; an interruption that exhausts the budget fails the task before another run starts.
When a member fails or ends without status while
holding an item, the coordinator receives a notice naming the item and the member's branch
(`prompts/harness/team/bus-member-failed-item.md` or `bus-member-no-status-item.md`). When the lead has reported
`STATUS: DONE` and waits while an item remains blocked behind a failed member, the human is told immediately once per
item (`prompts/harness/pr/notice-standing-item.md`) with the member's last-run details and commands to speak to the
member and resume it (`agent say <task>`, `agent resume <task>`). Resuming a task clears the notification deduplication
for items assigned to that task. Existing notification files identify the item and the failed run, preserving suppression
across daemon restarts; a later failed run can notify again.

Snapshots keep a worktree's work when its run was cut. When the daemon starts, it takes a snapshot of every task whose run did not end, if the round that run worked for is still open: everything in the worktree (uncommitted, staged, deleted, untracked and binary files; ignored files are left out) becomes a commit under `refs/agents/snapshots/<round>/<task>/<n>` in the repository, and the worktree, its index and its files do not change. `agent status <round>` shows the last snapshot of each task with its number, time and size. A snapshot above `[snapshot] warn_mb` (default `200`) is still taken and logged as `snapshot_large`; a snapshot that fails is logged as `snapshot_failed` and the daemon goes on. Snapshots are never pushed and never pruned yet: remove the references with `git update-ref -d` when you no longer need them. Two events and one history table (`snapshots`, migration `12`) describe them, see [`docs/history.md`](docs/history.md).

Build directories (`target/`) are released when a round closes, and when a reviewer finishes while free disk space is below `disk.low_free_gb` (default `100`), logging `build_directory_removed`. Only `target/` within harness worktrees is ever removed. Free disk space is continuously tracked: below `disk.low_free_gb`, a notification lists the five largest worktrees and `agent status` reports the free space; below `disk.min_free_gb` (default `20`), `agent run` is refused with cleanup guidance, open rounds wait with a single notification, and waiting rounds resume automatically once free space is restored.
`agent status` reads the last measured free space from `AGENTS_HOME/.free_space`, a plain number of bytes that the daemon writes at each measurement; the line is absent until the daemon has measured, and shows the reason when the file or `config.toml` cannot be read. When a state file cannot be saved, the daemon first runs the notification command once with the reason as its only argument and no file is written; a failure of that try is logged and the daemon stops as before.

The Rust daemon keeps each team's investigator activities in `state.json`: investigator, task, rounds,
assignment time and terminal outcome. Activities distinguish `team-member` (coordinator or member), `reviewer`
(first review, verification or second opinion), `investigator` (questions and explanations) and `task-allocator`
(task splitting and integration). Crew membership and review work are recorded today; the latter two kinds do
not introduce new workflows. Reports remain recorded between reviews and after the item advances; stopped and
failed assignments retain their ending time and the reason known by the daemon.

State schema `11` automatically rebuilds this history when reading schemas `1`–`10`, using retained crew,
reviews, fix passes and second-opinion attempts. Old records without exact opinion times use the earliest task
run for assignment and its last run ending for completion, falling back to team creation or the latest retained
run evidence. The activity record also supplies the investigator selection order below and the view
`agent status --investigators`. See the
[activity record](docs/state.md#investigator-activities).

A coordinator can ask the human to split an item into independently deliverable parts in one questions file.
Approve using the [explicit split verdict](docs/task-list.md#approved-split-plans-core-format); `agent answer`
lists the recorded parts, or explains a rejected form. The current round delivers part a. Later parts receive
separate rounds, branches and pull requests after their dependencies merge; the original item and team remain
open until all parts merge. `agent status` shows the parts and their dependency states. Definitions and progress
survive restarts. The CLI negotiates socket protocol `4` for split acknowledgements and status details. Older clients keep protocol
`3`, whose replies omit split fields; ordinary replies retain their existing shape. A new CLI probes with a
read-only status request and reconnects once using protocol `3` when an older daemon rejects or closes the probe.
The actual answer is sent once. Showing the full state of a split team requires protocol `4`.
Schema `13` states from version 0.29.0 remain readable, with original round names and histories.

The Rust CLI provides a read-only diagnostic bundle for one team:

```sh
agent debug --team TEAM_ID
# Alternatively select --round ROUND_ID or --task TASK_ID; exactly one is required.
```

`--out DIR` selects the parent directory (the default is `<home>/debug/`), and `--stale-minutes N`
sets the activity threshold. The command prints the paths of the new directory and its `SUMMARY.md`.
It collects the saved state, status when the daemon answers, team tasks and their runs, prompts, final messages,
steps, raw output and standard error, questions and answers, gates, checks, worktree git facts and team history.
It also identifies the daemon by ownership of the home lock and collects its process facts and version.
The daemon log defaults to `<home>/daemon.log`; `--daemon-log FILE` selects another file. Its bounded tail
keeps team/task lines and warnings or errors about any team. An unreadable explicit log is reported in its section.
It works offline, reports unreadable parts, and changes no task or git ref. Before writing any artifact it replaces
exact values from `secrets.toml` and environment variables ending in `TOKEN`, `SECRET`, `PASSWORD` or `KEY` with
`<cut>`, including JSON-escaped values in daemon logs. An unreadable secrets file stops collection before any
bundle is written.

Task files and git output are bounded to 4 MiB each; task files together to 64 MiB, with at most
4096 entries per task and 16 directory levels. Limits and skipped symlinks are reported as read failures.
History is read from a temporary copy of the database and its WAL/SHM files, including committed events while
the daemon holds the home. Each source file is bounded to 64 GiB; up to three snapshot attempts accommodate
source changes before they are reported as unreadable. SQLite creates no files beside the original database. Summary timestamps show UTC and the machine's local offset.
Each task shows its latest run's reason, final message and steps; earlier runs remain in the bundle.
Questions are ordered numerically, and gate outputs name their source and modification time. Worktrees removed
after a merged round are explained once, with retained branch facts read from the caller's clone.
An unpublished member branch is expected; the coordinator's origin ref is still checked.

The summary uses the pure anomaly assessment and stays below 100,000 bytes; full collected details are separate
artifacts. Summary redaction precedes truncation and byte budgeting, including daemon logs and process command lines.
Collection includes machine metrics (load average, memory and swap, filesystem space and inodes for home and temporary directories, daemon descendant processes with age and redacted command lines, and directory sizes for tasks and worktrees) and the fifty newest untracked files per worktree from a single git status listing. Filesystems at least 90% full by space or inodes trigger an anomaly. Complete untracked listings are saved as `untracked/<task>.txt`, separate from copied task files. See the
[diagnostics design](docs/architecture.md#debug-diagnostics-commands-and-pure-decisions).

The daemon records teams, rounds, runs and their steps and changes in `history.db`, together with gates, checks,
pull requests, review and fix passes, questions, answers, controls and notifications. See
[`docs/history.md`](docs/history.md) for `[history]` settings, read-only queries and lost-record reporting in
`agent status`. Readers reject invalid variant fields even when SQLite constraints were bypassed.
Known values from `secrets.toml` are replaced with `[REDACTED]` before history persistence, including echoed CLI output;
secret-bearing structured identities cause counted omissions. Raw CLI output files are separate from this protection; Rust tasks no longer write `activity.jsonl`.
Git attribution may briefly hold a run at its boundaries: each snapshot has a `2`-second budget plus bounded process cleanup;
timeouts are counted and the work continues.

The daemon's own structured diagnostics go to both `stderr` and `history.db`. The optional `[log] level` setting in
`agents.toml` keeps `error`, `warn`, `info` (the default), or `debug` and more severe events; disabling history still
leaves `stderr` logging, and a history write failure is reported on `stderr` without stopping the daemon's work.
Repeated identical warnings are coalesced with a count. [`docs/log.md`](docs/log.md) describes the event vocabulary,
team/round/task/run scope, redaction, decisions and outside-call records; [`docs/configuration.md`](docs/configuration.md)
documents the setting.

Two events tell a step of a round that GitHub or git failed: `github_step_waiting` (`warn`), logged once when the step
starts to wait, with `team`, `operation`, `pr`, `outlook` (`may_pass` or `will_not_pass`), `next_try` and the line
`agent status` shows; and `github_step_resumed` (`info`), logged once when it goes through, with `team` and
`operation`. The tries in between log only the `github_call_failed` warnings of the calls themselves. A `history.db`
made before this version is migrated at the start of the daemon (history version 11) so that it keeps these events;
it also keeps `build_directory_removed` from then on, which such a file refused before.

Read a round with `target/debug/agent log --round <round-id>`, or a team's rounds with
`target/debug/agent log --team <team-id>`. Both read the history directly and work with the daemon stopped.
Use `--prompts` for whole prompts, `--no-steps` to omit individual steps, or `--json` for every record exactly once
as JSON lines. An unknown round or team exits with code `3`. See [`docs/log.md`](docs/log.md) for the story format,
ordering and exit codes. Schema version `6` adds the search index and durable follow cursor after the daemon-log schema; older history files remain readable without migration
by `agent log`, and the daemon migrates them when it opens them for writing.

Bare `target/debug/agent log` prints the newest `100` daemon log rows. Use `--show log` with a round or team to
select rows instead of its story, or add a row filter: `--task`, `--run`, `--agent`, `--level`, `--event`,
`--since`, `--until`, `--grep`, `-n` or `-f`. Filters intersect; `--json` emits one log row per line in this mode. Combining
`--round` and `--team` also selects rows. A row query with no matches succeeds with empty output.

Use `--grep TEXT` for literal FTS5 search, `-f` to follow new rows across daemon restarts, and `--stats` for database
size, table counts and the oldest timestamp. A scoped `--show` selects `tasks`, `agents`, `worktrees`, `runs`, `prompts`,
`changes`, `reviews`, `log` or `usage`. `[history] keep_days` removes expired closed rounds and their gate output;
`[log] keep_days` removes old rows of no round. Both default to `0` (keep all), and open rounds always stay.

How the human answers the agents and steers a task or a team, through the daemon (`agent daemon`) that owns the state:

| Command | What it does |
|---|---|
| `agent answer <task>` | opens the questions file of the task in your editor (`[questions] editor`, else `$VISUAL`, `$EDITOR`); when you close it, the answers are sent and the task goes on with them |
| `agent answer <task> --file answers.md` | sends the answers written after `**Verdict:**` in a questions file instead of opening the editor |
| `agent stop <task\|team>` | ends the run; nothing of it starts until it is resumed |
| `agent pause <task\|team>` / `agent unpause <task\|team>` | holds the task or team, and lets it go on in the same session |
| `agent resume <task\|team>` | starts a stopped, failed or waiting task again; when steps of the team wait for GitHub or git, it tries those at once and starts no task |
| `agent say <task\|team> words…` | a message to the agent or the team, which reads it at its next look at the bus; a word to a task reaches the agent's session in that task's team only |
| `agent status --running` | only the teams that have a round that goes on, with those rounds: what is being worked on now |
| `agent status --investigators` | what each configured investigator is doing: its last two activities in the teams that are not closed |

`agent status` ends the line of a task that runs with `last activity HH:MM:SS`: the local time the output of its run last
changed (the file `tasks/<task>/runs/<run>/out.jsonl`), which is how to tell a task that works from one that hangs.

The line of a reviewer says what it did or does, from the record of the team's investigators, instead of the one word of
its status (a reviewer that waits between its reviews said only `waiting`):

| Says | Means |
|---|---|
| `working`, with `last activity HH:MM:SS` while its run writes | a review goes, or one is queued for it |
| `done, 1st review finished 12:08:17` | it gave the report the harness asked for; the ordinal counts the reviews it gave for the round (the first review, the verifications and the second opinions alike; a review that ended without its report is not counted) |
| `stopped at 12:10:00, reason: the run showed no activity for too long` | the task was stopped or failed; the reason is the one the daemon knows (`the human stopped it`, `the run ended without a status`, and so on) |

A reviewer task without recorded activity shows the word of its status as before. The lines of the coordinator, the
members and the workers do not change.

`agent status --investigators` shows every investigator of `[investigators.NAME]` in `agents.toml`, in the order of
their names, with the last two of its activities in the teams that are not closed, the newest first: the task, the kind of
the activity (`team-member`, `reviewer`, `investigator`, `task-allocator`), the investigator, the team, and what it did:
`done at 12:15:15, 1st review finished`, `working, 2nd review`, `done at 12:00:00, tasks allocated`, or, for one that
ended without its report, `stopped at 12:10:00, reason: …` as on a reviewer's line. A team counts as open while it has a
round that goes on. An investigator with no activity in an open team is listed as `free, no activity in an open team`:

```text
task-20261119-094409-saal  reviewer  inv-gemini-argon  for team-20261119-094000-saal  working, 2nd review
task-20261119-094408-mhdx  task-allocator  inv-kimi-deepinfra  for team-20261119-094000-p4vt  done at 12:00:00, tasks allocated
inv-opus  free, no activity in an open team
```

The line of a round ends with when the round started, in local time (`started DD-MM-YYYY HH:MM`), and for a round that
is closed with how long it took (`took 3h 01m`). A round that goes on also says where its work is, after `open`:

| Says | Means |
|---|---|
| `only local` | the work is in the worktrees; no pull request is open |
| `PR draft` | the pull request is open as a draft: the checks before the review hold it |
| `PR ready` | the pull request is ready for review and no reviewer has started |
| `PR in review` | a reviewer works or has reported, or the team fixes what the reviewers asked for |
| `PR approved` | the reviews approve; the pull request waits for its merge (with `--no-merge`, yours) |

The draft is told from the history (`history.db`), the rest from the daemon. When the history cannot be read — it is
turned off, or its file is missing — such a round says `only local or PR draft`, and a last line says why.

A run that ends with questions leaves `questions-<round>.md` in the task's directory and a notification file; with
`[questions] open = "vscode"` the file is also opened in the VS Code window connected to the host. Saving that file with the
`**ANSWERED**` marker on its last line sends the answers as well (the daemon watches the file; a file saved while it was
down is found when it starts). A task that waits for answers is not stopped, paused or resumed: answer it first. Every
control is written to the activity log of each task it reached, with who did it, and `agent status` shows the new status.

VS Code server discovery skips plain files beside builds (such as `cli/servers/lru.json`) and in place of a build's
`remote-cli` directory in either server layout. Directory symlinks are followed; other listing errors are reported.

With a `github_token` in `secrets.toml` the daemon runs the autopilot: when an item's branch has a green gate it opens the pull
request, `[review] min_first_full_reviews` investigators from outside the team review it (default `2`). All their
approvals merge it and start the next item. Set the key in `agents.toml`, for example:

```toml
[review]
min_first_full_reviews = 3
```

An explicit value must be at least two and no greater than the configured investigator roster. Team creation also
requires that many investigators outside the team. Files without the key retain the default of two and remain readable.
The first pass consists of independent full reviews: reviewers have no bus and must not read or discuss peer reports.
The harness holds every report and its findings until the last verdict arrives, then posts reports in request order
and the findings table. A replacement reviewer performs its own full review while the received reports keep waiting.
Later passes retain the bus and post reports as they arrive. After a fix pass they verify the open findings and
changes since the preceding reviewed commit, with the lead's final fix message.

Every review, verification, second opinion and replacement uses the same deterministic order. Investigators with
no activity in this team come first; then the one whose latest assignment in this team is oldest. Ties use the
oldest latest assignment anywhere (never assigned comes first), then the investigator's name. All activity kinds
count. The investigators asked in the team's latest request, including both of a pair, go after all other eligible
candidates and are reused only when needed to fill the request. An investigator is free across teams when it has
no running or queued work; a sleeping coordinator or member can review elsewhere. Accepted review and opinion
requests reserve their investigator while awaiting scheduling. A team's own coordinator and members are always
excluded, as are a finding's reporter and previous opinion participants where independence requires it. Within the
same team, an investigator whose reviewer task there is stopped, failed, paused, waiting for your answers or done is
not offered again for a later review: the harness does not start a reviewer you stopped or held, and a task that
is done cannot start again; the investigator is free for every other team. If too few eligible investigators remain, the round
waits for a free independent investigator; these permanent exclusions are never relaxed. The text of
`agent log --round <round-id>` describes reviewers as selected among the available candidates, for both old and new histories.

Fewer than twice the review count outside the team produces a startup warning and does not prevent starting. A pull request keeps its review count
even if the configuration changes. Active pull requests migrated from an older daemon retain their original review protocol.

`agent run --no-merge` stops before the merge and tells you; after `[review] fix_passes` rounds of "changes
requested" a question reaches you like any other. Answering grants another `[review] fix_passes` allowance.
The notice names the allowance and open blocking findings. How it works, and its limits, are in
[`docs/architecture.md`](docs/architecture.md) ("Pull requests, reviews and the merge").

The Rust lead writes its contribution to `tasks/<lead>/description.md` under `AGENTS_HOME`, outside the repository.
Its prompt names the full path. Include what changed and why, tests and their results, deviations, out of scope,
skipped tests and lint suppressions with their paths and reasons. The harness builds the PR description from its
header, this file, measured **Size** and recorded **Gate**, and the tests changed after RED. It rebuilds the body
at publication, after a fix pass and while the PR is open, including when only this file changes. Edit the file
rather than the description on GitHub; manual body edits are replaced. A missing or empty file does not hold a
round: the description says the lead supplied none. Size and Gate headings in the contribution are removed.
Size follows PR-1 at the published commit, excluding tests. Gate names that commit, finish time, outcome and log;
a pass following a failed gate at the same commit is identified as a second run.

Review and verification requests carry the lead's gate record when it belongs to the reviewed commit. Reviewers
use that record and run single tests to investigate findings; they do not repeat the gate. Without a matching
record, the request asks for `agent gate`. A reported red gate must identify the failed step and log line.

Each posted Rust review starts with the investigator, CLI and configured model, a green **APPROVED** (`✅✅`) or
red **REQUEST CHANGES** (`❌`) verdict, and the review pass and linked commit. The report follows a horizontal
rule; its original `VERDICT:` line remains in local history. Commit IDs in pull request descriptions, branch
update comments and reviews become GitHub links when git resolves them. Longer inline code spans, code fences,
existing links (including bare URLs), HTML tags and comments stay unchanged. Lookup failures are logged and publication continues with the original text. The review layout is in
`prompts/harness/review/`; review reports and PR descriptions must label code fences with their language.

A review report lists numbered findings, each with one of three severities — `[BLOCKER]` (broken logic, build or gate: fixed in
the code), `[MAJOR]` (a deviation from the task, the developer rules or `AGENTS.md` with no explanation) and `[MINOR]` (advice,
never blocks) — a rule ID and a `CHECK:` line that tells how to see whether it still holds. The daemon reads them: a report whose
finding has no or another severity or no check, or whose verdict contradicts its findings (`APPROVE` with a `[BLOCKER]` or
`[MAJOR]`, `REQUEST_CHANGES` with neither), goes back to its reviewer. The findings of a pull request keep their numbers over
all review passes; a reviewer of a later pass answers for every open one (`FINDING 3: RESOLVED` or `FINDING 3: OPEN — why`).
After RESOLVED, an optional nonempty reason may use ` — reason`, ` - reason`, or `: reason`; the saved report retains it.
The harness keeps one comment on the pull request with all findings, edited after every pass, and the fix-pass
text names the findings that block by number. The comment is a list, not a table: a header line counts the open findings
of each severity (`open — 0 BLOCKER, 1 MAJOR, 0 MINOR; closed — 2`), then each open finding has an entry: a first line with
its number, severity, rule, state (`open`, `resolved`, `not confirmed`), reporter and place, and below it the text cut to
160 characters at a word boundary (`…` when cut). An explanation or a second opinion is a third line of its entry.
Closed findings (resolved, not confirmed, or explained and accepted) are folded in one `<details>` block, `Closed (N)`. `agent status <round>` counts the open ones of each severity. Pull requests
opened before the relevant protocol update keep their original review rules until they end.

For new pull requests, the lead's fix message answers every open blocking finding before `STATUS:`:
`FINDING 3: FIXED`, or, once for a MAJOR, `FINDING 3: EXPLAINED — why the deviation is right`.
FIXED accepts the same optional reason separators as RESOLVED. Lines for open MINOR findings are optional;
FIXED is recorded, and other answers for MINOR findings are ignored. Unknown finding numbers are refused.
A correction quotes the refused line, identifies its number and mistake, and lists the allowed forms for open blockers.
A confirmed BLOCKER requires a code fix. Missing or forbidden answers receive at most two corrections without consuming
another fix pass; after that a “lead’s fix response was refused” notice gives the reason, bounce count and recovery
commands, and verification stays held. A reviewer’s report with a verdict but an invalid finding answer receives a
“reviewer’s report was refused” notice with the refused line instead of a missing-verdict notice.
A report is checked against every item it reviews before any verdict or finding is recorded. If one item refuses it,
the whole report goes back to its reviewer, quoting the line and finding number and listing the answers allowed
by that item's findings. The reviewer has two corrections; exhaustion sends one notice to the human. A refused
report changes none of the reviewed items and does not stop the daemon, including when older items on the same
pull request retain different explanation states.
The verifier's brief names, beside each open finding, the answers it may get on an `ANSWER:` line — `ACCEPTED — why`
or `REJECTED — why` for an explanation that waits for a decision, `RESOLVED` or `OPEN — why` for the others, and
`RAISED [MAJOR|BLOCKER] — why` only where a raise is allowed. The report is read by the same list, so an answer the
brief offers is never refused for its form.
Items that share a pull request share its findings: a fix response and a verification are recorded for every item
of the pull request, counted by the pull request's fix pass (an item that joined later has fewer passes of its own).
When the daemon starts with a state in which such items disagree on a finding, the most advanced record holds for
all — a decided explanation over a pending one over none, a rejected over an accepted one, a closed finding over an
open one — and each repaired finding is logged as a `restart_corrected` warning with the fields `team`, `pr`, `items`,
`finding`, `explanation` and `status`. After a restart, a reviewer whose report of a pending review was not taken is
asked for the review again, and a reviewer that failed and was replaced no longer makes its round wait for the human.
A fix pass that claims `FIXED` must hand over a new commit. If its tip is still the reviewed commit,
the harness returns it to the lead to commit the fix and run `agent gate`. The same correction budget covers a
gate record that still belongs to another commit after the harness has finished its own gate. Three automatic
handover corrections are allowed; exhaustion sends one notice naming the round and `agent say <task> <instructions>`.
An unchanged tip can still enter verification when every blocker is explained or closed by a second opinion.
Legacy unanswered blockers require a committed fix too.

An idle open round's line in `agent status` and `agent status <round>` names what it waits for: a harness gate,
a review, a free independent investigator (for a review, a second opinion or the replacement of a reviewer that cannot
work, when none is free), a second opinion that an investigator is giving, the human's answer, or the lead's correction. `nothing: this is a defect` means the
harness has lost its next action; it logs one `daemon_task_failed` error for that round with `operation=round_wait`.

A verified item delivered through another team PR says `delivered by PR #N, waits for its merge`; after that
PR merges it says `done, delivered by PR #N`. It opens no separate PR and keeps waiting across daemon restarts.
This release includes the checked delivery lifecycle; the coordinator instructions and automatic return to work
when the delivering PR closes without merging follow in the next part. Until then, such an item remains waiting
and the daemon logs a warning. The state format advances to schema 14, which reads existing state through schema
13; older binaries need a state backup from before the upgrade.

A step of a round that GitHub or git fails does not stop the daemon: the round says `waits for GitHub: refresh the
description of the pull requests, tried 4 times, next try 01:07` and the step is tried again by itself, after 30
seconds, then after a pause that doubles up to 10 minutes. When GitHub refuses it for good (a 404, a token that may not
do it) the line says `refused: <GitHub's answer>`, you are told once while it waits, and `agent resume <team>` tries it
again at once after you fixed the cause. Such a resume is for the waiting steps only: it starts no task of the team; a
second `agent resume` starts them, as before. The same holds when git cannot read the commit of a worktree, for example
the lead's before a merge: `waits for GitHub: read the lead's commit before the merge #1, refused: ...`. The log has one
`github_step_waiting` warning when a step starts to wait and one `github_step_resumed` when it goes through; see
[`docs/architecture.md`](docs/architecture.md) for the steps this covers. A daemon that starts tries every step at once.

Verifiers answer a pending explanation with `FINDING 3: ACCEPTED — why` or `FINDING 3: REJECTED — why`.
Every verifier must accept before the finding becomes MINOR; one rejection requires a code fix and bars another
explanation. The table retains the explanation and every verifier's evidence. A verifier may raise severity using
`FINDING 3: RAISED [MAJOR] — why` (or `[BLOCKER]`), but cannot lower it independently. An accepted explanation
can be raised again only after its affected file changes from the commit at which it was accepted.
`agent log --round <round> --show reviews` includes explanations, acceptance, rejection and severity changes.
State schema 6 migrates schema 5 without adding this protocol to active pull requests; history schema 8 adds
these events while preserving existing records and search.

With `[review] allow_second_opinion_on_blocker = true` (also the default when omitted), the lead may answer
`FINDING 3: SECOND OPINION — why the blocker is false` once per open BLOCKER. Set the key to `false` to disable
this response and remove it from fix-pass instructions. MAJOR and repeated BLOCKER requests are returned for
correction; requests for open MINOR findings are ignored. The entire response is checked before any answer is recorded;
accepted FIXED answers survive restarts.

Each request waits for a free investigator outside the team who neither reported the finding nor reviewed the
pass being answered. The investigator examines only that finding at the handed-over commit, without a bus,
and reports `OPINION: CONFIRMED` or `OPINION: NOT CONFIRMED`, followed by what it read or ran and what it showed.
An invalid report receives the same two corrections as a missing review verdict. An unconfirmed finding closes
permanently under its original number; the PR table names the investigator and retains the evidence. A confirmed
blocker returns to the lead for a code fix in the same fix pass, with no additional charge against `fix_passes`.
Every opinion must be accepted and every confirmed blocker answered FIXED before verification begins at a new tip.
If all blocking findings close as not confirmed and the tip is unchanged, the pass is approved without another
review and follows the round's usual merge or approval policy.

A failed run, a run without a final status, or a report still missing its opinion after the allowed corrections
is assigned to another independent investigator. An investigator silent for `[review] replace_after_minutes`
is replaced too. Replaced queued, waiting or quota-paused tasks are stopped and their scheduled wake is removed,
so their investigators become available for other requests. Previous investigators are excluded from subsequent draws. A request has at most three
assignments (the original and two replacements); when nobody eligible is free it waits until one is available.
After the third unsuccessful assignment, one human notice lists the round, finding, all three investigators,
their tasks and how each assignment ended. Inspect those tasks and arrange an independent opinion before
continuing the round; no fourth investigator is asked automatically.

On daemon restart, an unanswered assignment resumes its existing task when that task can resume and its
worktree survives, or waits for another investigator when the task ended, awaits human answers, or its
task/worktree is gone.
Accepted opinions and FIXED answers survive;
recovery and replacements do not consume another fix pass. `agent status <round>` names each waiting finding,
its investigator and assignment time, or says it waits for a free independent investigator. `agent log <round>`
tells the requests, unsuccessful assignments and decisions, including the first evidence line and the full evidence.

State schema 9 migrates schema 8, including running fix passes, preserving FIXED answers and completed opinions.
The optional attempt history defaults to empty; findings without opinion fields have had no second opinion.
History schema 10 adds `second_opinion_ended` for replacement and exhaustion, alongside the existing
`second_opinion_asked`, `finding_confirmed`, and `finding_not_confirmed` events and `second_opinion` run reason.
Each request names its selected investigator. Existing records, links and search migrate automatically.

Before the review the daemon checks the round's own diff and commits through git, with no model, if the repository has a
`[repos.NAME.checks]` table in `agents.toml` ([`docs/configuration.md`](docs/configuration.md)): the lists of path patterns that
say what is a test, what is production code and what judges a round. A round that changes an existing test other than by added
lines, or a protected file, waits for your decision. The questions group findings by kind and judge: one question lists all
paths of that kind with their findings. Answer every question with `allow`, `refuse`, or `allow except <path> <path> ...`.
The exception paths must belong to that question: they are refused and the others allowed. A refusal in any group refuses
the path. `agent answer` and the daemon reject `ok`, blank verdicts, unsupported words and unknown exception paths before
accepting any answers. Other questions still accept `ok`. A pending checks file written by an older daemon keeps its old
per-file questions and verdict rules, including `ok` accepting the recommendation to refuse.

A task list can allow required path changes beforehand. Put `Allow: Cargo.toml` or, for example,
`Allow: tests/**/*.golden` before its brief (after `Coordinator:` if present). The paths are relative to the repository
root, with only `*`, `**` and `?` as glob operators. Invalid or overly broad patterns and patterns reaching `AGENTS.md`,
`CLAUDE.md` or `.github/workflows/` are refused before work starts. These allowances survive every round and daemon
restart, and `agent run` prints them. They permit covered test and protected-path changes, while other paths and
all markers still follow the checks above. Human answers continue to be recorded per path in the unchanged
`checks.json`. The harness lists changed covered paths with their covering directives under `## Allowed beforehand`
in each pull request; it omits the heading when none changed. See [the task-list format](docs/task-list.md).

A merge is checked against its first parent, on paths the round itself changes against the base. Merging the base branch
brings no edit of a frozen test of its own; `git merge -s ours` changes nothing. An edit made inside a merge, including a
conflict resolution that removes a frozen test line, still counts.

A round that changes code without adding tests, edits the test of its own `RED:` commit, or adds a skipped test, a lint
suppression or code that asks whether it runs under test goes back to the team with the reasons, `[review] check_passes` times
(2), and then you are asked. If a check also finds changes that only you may allow, you are asked about those
changes first, before the team starts its next pass. After your answer, the remaining findings go back to the team;
the question itself uses no team pass, and what you allowed is asked again if the team changes that file
afterwards. If the passes are already used up, you are asked about all remaining findings.
It is "Round checks before the review" in [`docs/architecture.md`](docs/architecture.md).

A gate is run with `agent gate`, by an agent in its worktree (the human can run it too):

| Command | What it does |
|---|---|
| `agent gate [task]` | runs the repository's gate command (`[repos.NAME] gate`) in the task's worktree and says what it said; the task is the one named, or the task of the worktree the command runs in (also from a subdirectory) — outside a worktree, with no task named, it fails at once |

The gates of all the tasks wait in one queue in the daemon and run one at a time on the machine, in the order they were
asked; the command waits for its turn and then for the gate (at most 16 wait; the next is refused). The command is stopped,
with everything it started, after `gate_timeout_minutes` (default 30, counted from the start of the gate, not from the
wait) and is recorded as failed; stopping or pausing the task stops its gate too. The result is recorded with the commit
(`HEAD`) and whether the tree was clean, in `tasks/<task>/gate.json` and the task's activity log, and the full output is
in `tasks/<task>/gate-<n>.log`. The preceding record is kept in `tasks/<task>/gate-previous.json` to disclose a
second-run pass; older task directories without that optional file remain readable. A tree is not clean when a file is changed or untracked, when a merge waits to be committed
or when the gate itself changed the tree or `HEAD`: such a gate may run, but its pass is not a pass of the commit. A run
that waits for its own gate is never cut by the watchdog. The command succeeds for a pass and fails (status 1) for a
failure, a timeout and a refusal. The command says first where the result is recorded: a model CLI cuts a shell command that runs long (the Bash tool of Claude Code has a time limit, in minutes, and a gate may take 30) while the gate goes on, and the agent then reads `tasks/<task>/gate.json`. Details: [`docs/architecture.md`](docs/architecture.md), "The gate".

## The live panel

## MCP tools available to agents

| Tool | Purpose |
|---|---|
| `list_agents`, `send_message`, `read_messages`, `wait_for_message`, `set_status` | communication |
| `reserve`, `release`, `list_reservations` | shared resources |
| `notebook_read`, `notebook_append`, `notebook_write_section` | the repository notebook |
| `checkpoint` | save a resumable work state; a non-coordinator member saves its own archived task record, not carried into a resume |
| `request_investigation`, `investigation_status`, `record_decision` | investigators (not for investigators themselves) |
| `delegate`, `delegation_status` | worker requests (normal tier); optional `requires` routes to a capable node |
| `team_status`, `claim_lead`, `handover_lead` | teams |
| `quiz_start`, `quiz_answer`, `quiz_results`, `team_exclude` | the qualification quiz |
| `board_add`, `board_list`, `board_claim`, `board_done`, `board_release` | the team board (`board_add` with `for=<member>`: only that member may claim the item) |
| `consult_start`, `consult_reply`, `consult_results` | ask the other normal members with a time limit (e.g. for recommendations); silence is shown as NO ANSWER |

**An agent in two teams.** An agent may be a member of several open teams at once, with a task, a worktree and a
session in each. A direct message (`send_message` to a name) sent from a session in a team is for the agent's session
in that team only: it wakes only that task, and only that session reads it. Each session of an agent has its own read
cursor, so reading in one team does not mark the other team's messages as read. A message to a name that is not a
member of the sender's team is refused, and the refusal lists the members. The exceptions are the human, `agentbus`, a
running or configured agent, and the worker of one of the sender's open requests, as long as that name is in no open
team. A session outside any team (a solo task among them), and `agentbus send`, write to an agent in one open team in
that team; to an agent in several open teams they must name the task (`send_message` with the task ID in `to`,
`agentbus send --task <task>` or `--team <id>`), otherwise the message is refused with the tasks to choose from — also
when the agent runs or is configured. `agentbus send --for-task <task>` writes to the session of that task alone, a
solo task too; `agent say <task>` uses it. The history of `read_messages` and `read_message` by number do not show a
session the messages of the agent's other teams or tasks. Messages to `team` and to `all` are delivered as before.
Messages written before 0.30.0 name no team or task and are read as before. A reservation belongs to the agent's task
that took it: the agent's run in another team cannot release it, and it is freed when the run that holds it ends.

## Development and tests

[`AGENTS.md`](AGENTS.md) holds the rules for whoever changes this repository, human or agent.

```sh
make gate        # the gate of a change: the Rust workspace's fmt, clippy, tests and docs, with short output
make gate-full   # the gate plus the lints and the tests of agentbus (Go): for a change of agentbus/ or tools/
make tools       # installs the pinned linters (golangci-lint, staticcheck, shellcheck) into ~/.local/bin
```

The tests need no network and no accounts: they run the daemon against a temporary `AGENTS_HOME` with fake CLIs and a
fake GitHub (`crates/*/tests/`), and build `agentbus` from its source. One test run at a time on the machine
(`tools/test-lock.sh`); `GATE_WAIT=1 make gate` waits in line. In the worktree of a task, run the gate as
`agent gate`, which records its result.

## License

MIT — see [LICENSE](LICENSE).
