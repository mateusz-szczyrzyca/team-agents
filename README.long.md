# team-agents — the full guide

The short introduction is in [README.md](README.md). This guide describes every command, option and behaviour of
the daemon.

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
  the harness checks each round, runs the repository's gate, asks investigators for independent reviews, takes the
  pull request through its fix passes and, in autopilot, merges it.
- **Plans of steps:** every solo agent, coordinator and member keeps the plan of its task in `.agent/plan.md`; the
  daemon checks the steps it can check and sends back a `DONE` whose plan is not finished.
- **A pool of workers:** cheap agents of `[tiers] workers` take single steps that an agent delegates, in worktrees of
  their own; the daemon checks each result before it brings the commit to the requester's branch.
- **Recovery:** a run that hangs, fails, ends without a status or is cut by a restart goes on in a new session with a
  briefing built from facts, or is handed to another agent; snapshots keep the work of a cut run.
- **Load spread across providers:** agents of different CLIs and tiers work side by side, so the load and the
  token usage are distributed over several providers and accounts.
- **Observability:** `agent status`, a history of every run and step (`agent log`), a diagnostic bundle
  (`agent debug`) and notifications.

The harness is written in Rust (`crates/`). Its first implementation — a Bash script, a rewrite of it in Go, a
terminal panel and a web interface in Go — was removed in 0.25.0; `agentbus` is the one Go program left. There is no
panel and no web interface now: `agent status`, `agent log -f` and the notifications are how you watch.

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
  - [Runs and sessions](#runs-and-sessions) · [Agents, workers and busy hand-overs](#agents-workers-and-busy-hand-overs) ·
    [Serena and the reminders](#serena-and-the-reminders) · [The plan of a task](#the-plan-of-a-task) ·
    [When a run goes wrong](#when-a-run-goes-wrong) · [Snapshots, restarts and disk](#snapshots-restarts-and-disk) ·
    [Investigators, tiers and splits](#investigators-tiers-and-splits) · [`agent debug`](#diagnostics-agent-debug) ·
    [History and `agent log`](#history-and-agent-log) · [Answers and controls](#answers-and-controls) ·
    [Removing a team](#removing-a-team) · [`agent status`](#agent-status) · [Questions](#questions) ·
    [Pull requests, reviews and merges](#pull-requests-reviews-and-merges) ·
    [Checks before the review](#checks-before-the-review) · [The gate](#the-gate) ·
    [Compatibility and upgrades](#compatibility-and-upgrades)
- [MCP tools available to agents](#mcp-tools-available-to-agents)
  - [Requests to workers](#requests-to-workers) · [An agent in two teams](#an-agent-in-two-teams)
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
[`docs/plan.md`](docs/plan.md) (the form and verification of a task's plan of steps), [`docs/tdd.md`](docs/tdd.md),
[`docs/daemon-overview.md`](docs/daemon-overview.md) (what the daemon watches and when it acts),
[`docs/checkpoints.md`](docs/checkpoints.md) (how work survives a lost session).
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

### The commit hook of the runs

Every run of an agent gets the harness's hooks directory **through its environment**: the `git` the CLI starts has
`core.hooksPath` set in `GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n`, for that run only. **Nothing is written into the
repository** — not its `.git/hooks`, not its configuration — and your own checkout is not touched. The directory holds a
`commit-msg` hook that runs `agent guard-commit` (see [Checks before the review](#checks-before-the-review)) and then the
repository's own `commit-msg`, and links the repository's other hooks (its `hooks` directory, or the directory of its own
`core.hooksPath`), so a hook of the repository still runs in an agent's run, and one that fails still fails the commit. An
agent that wants to skip the hook can use `git commit --no-verify`; the checks before the review find the commit then.
To turn the early refusal off for a repository, set `commit_guard = false` under its `[repos.NAME.checks]` table (on by
default; the checks before a review stay as they are): `docs/configuration.md` has the table. The hooks directory acts for the
repository it was made for only: a commit that the agent makes in another repository during a run runs that repository's own
hooks, and the hooks path of an outer run (a `cargo test` started by an agent) is not taken for a repository's own.

### Monitoring and response

- Ship the daemon's log ([`docs/log.md`](docs/log.md)) somewhere you look at, and alert on warnings and errors.
  `agent log` shows what a round, a team or a task did, with every prompt and step
  ([History and `agent log`](#history-and-agent-log)); `agent debug` bundles what you need to diagnose a team.
- Set spending limits at the providers and watch the usage `agent log` records for each run (context, output, cost).
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

**Accounts.** `[accounts.NAME]` of `agents.toml` (an `owner`, optional `env`), `[account_env.NAME]` of `secrets.toml` and
`account = "NAME"` of an agent or an investigator describe more than one subscription or key. A run of an agent with an account gets
the account's variables in its environment, over `[agent_env]` (the daemon's environment, `[agent_env]`, the account's
`env`, the account's secrets: the later wins); the quota of an account is its agents' together: see "The pause of an account" below. A session belongs to its account: when the account of an agent changes while its task is open (a changed `account = "NAME"`, read at the next start of the daemon), the next run does not resume the old session but starts a new one from the briefing of `agent renew`, with the reason `account_changed`, and no renewal is counted (an investigator, which works for no round, is told the same in a note of its prompt, with the plan its session before saved in its worktree). An agent with an account is named `<owner>-…`; an account that no agent names, an
agent that names a missing account, a name without its owner, a variable set in both `env` and `[account_env.NAME]`
and an `[account_env.NAME]` without an account are refused when the home is loaded, naming the
account and never a value; the values of `[account_env.NAME]` are cut from `agent debug` bundles and the history.
Files without accounts load as before. See [`docs/configuration.md`](docs/configuration.md), `[accounts.NAME]`.

**The pause of an account.** When the provider refuses a run of an agent that has an account — an exhausted quota or a rate
limit — the daemon pauses the account: every queued task of every agent of the account that a team's own agent works on, and the
task of the refused run, is paused until the same moment (the reset the provider named, or `[limits] quota_pause_minutes` / `rate_limit_pause_minutes`),
without a run of its own, and you get one notification, `Account NAME is paused until ...`, that names the account's agents.
No run of the account starts while the pause lasts, also not one of a task started after it: it waits queued. The queued
tasks of its workers, reviewers and investigators are not paused and not counted in the notification: they wait in the
queue, not failed, and start when the pause ends. A run of the
account that is going finishes; if it is refused too, its task is paused until the same reset, without another notification. At
the reset the paused tasks are queued again. Another account — also another account of the same owner — and agents without an
account are not touched. `agent status` shows a line per paused account: `account comp-claude paused until 15:30 (quota)` (the
date too when the reset is more than a day away). The pause is in `state.json` (`accounts`, `account_held_tasks`): a restart of
the daemon keeps it and its reset. The pause and its end are `decision_taken` records with `decision` `account`
([`docs/log.md`](docs/log.md)).

An agent that runs with an account is written with it in brackets on its task line in `agent status`
(`comp-claude-1 [comp-claude]`); the line of an agent without an account is as it always was. The account comes from
`agents.toml` as it is when you ask, not from the name of the agent.

[`examples/notify/`](examples/notify/) has scripts for `notifications.command`.

## Usage

```
agent daemon                          runs the daemon: the owner of the state and of every model run
agent run <agent> "task"              a solo task on the repository the current directory is in
agent run <agents...> -f tasks.md     a team: one agent is a task of its own, more are a team
          [--coordinator NAME] [--short-desc TEXT] [--no-merge] [--allow-unpushed-base]
agent status [--running] [--task]     the teams, their rounds, their tasks and a count of their tasks;
                                      --running: only rounds that go on, with the workers that served them;
                                      --task: every task with its plan and the parts of a split
agent status <team|task|round>        one team, task or round; a task ends with the context of its last run
agent status --investigators          what every investigator does now
agent status --json                   the teams as one JSON array
agent answer <task> [-f <file>]       answers the questions of a task; it goes on with the answers
agent say <task|team> <words...>      a message to a task or a team, through the bus
agent stop | pause | unpause | resume <task|team>
agent remove <team> [--force]         takes a team out: unmerged branches bundled, pull requests closed, files deleted
agent gate [task]                     runs the repository's gate for a task and records its result
agent log [--round R | --team T]      the story of a round or a team from the history, or the daemon's log rows
          [--task|--run|--agent|--level|--event|--since|--until|--grep|-n|-f] [--json] [--utc]
agent log --reminders [--days N]      how often each agent was reminded and how it followed
agent log --stats                     the size and the tables of the history
agent debug --team T | --round R | --task K   a diagnostic bundle with a summary that names what is wrong
```

`agent <command> --help` has the options of each. Two commands are hidden, for the harness itself:
`agent guard-commit` (the `commit-msg` hook of the runs) and `agent _delegation-check` (called by `agentbus` when an
agent delegates a step). The next section describes what the daemon does, by topic.

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

The daemon starts every run, reads how it ended and decides what comes next. The subsections below go through what it
does, topic by topic; [`docs/architecture.md`](docs/architecture.md) has the design behind it and
[`docs/configuration.md`](docs/configuration.md) every key named here.

### Runs and sessions

A run is one start of an agent's CLI for a task; a session is the CLI's conversation, which later runs of the task
resume. A run ends with a `STATUS:` line in the agent's final message — `DONE`, `QUESTIONS`, `FAILED` or `SLEEPING`
(a coordinator or member waiting for the bus) — and the daemon acts on it.

A run that resumes its session after a report receives only what happened since that report: answers, checks, a fix pass and other continuation notices. Its working context, rules and brief are already in the conversation. The assigned item is repeated only if it changed (or the previous run had no round); a coordinator still receives the team items. Every new session keeps its complete first prompt in `tasks/<task>/session-prompt.md`, overwritten when another new session starts, so the agent can read it after context compaction. A new session, including one replacing a session above its bound, and a session resumed after a human stop receive the whole prompt.

Every run is instructed to read code, diffs and logs in small parts, find the place first, and print one dump per command.

A Claude Code run that exhausts its turn budget resumes in the same session. Configure `[limits] max_turns`
(default `300`) and `turn_limit_resumes` (default `3`); after that many consecutive resumes the task goes on in a new
session (see [When a run goes wrong](#when-a-run-goes-wrong)), or fails and notifies you with the count when no new
session is left. Any other ending resets the count. `agent log` records `turn_limit` and the reason of its continuation.

**Context of a session.** Each run records the context size, the output tokens and the cost its CLI reports
(Claude Code, opencode and Antigravity tell the context of each model call; Codex tells output tokens only, so its
context is `not told`). `agent status <task>` ends with the context of the task's last run, or `context: not told`.
`agent debug --team <team>` puts the context of each run into its summary and names every run whose last context is
above its agent's bound: `[session] max_context` (default 150000 tokens; `0` switches it off), or the agent's own
`max_context`. A session whose last run reached more tokens than the bound is not resumed: the task's next run starts
a new session, whose prompt opens with the size that ended the old one and then the briefing of the checkpoint (what
was to be done, what is done, what is left). A size the CLI did not tell resumes the session as before, and so does
a bound of `0`. The new session is not counted against `[replacement] new_sessions`.

An opencode or Codex agent may also name its model's window in its own `context` (tokens): the CLI then compacts the
session at that window, before it grows past the bound, instead of the run failing when the provider refuses it.
A window below the bound is allowed — that is what makes the CLI compact early. opencode is told the window only for
a model registered under `openrouter` or `deepinfra`; Codex gets `-c model_context_window=N`. Claude Code and
Antigravity are left as they were, and so is an opencode model of any other provider: for them `context` changes
nothing. `output` (tokens, at most `context`) is the model's output limit, written next to the window in opencode's
configuration; without it opencode is told `0`, which makes it use its fallback of 32000 output tokens instead of
the model catalogue's, so set it for a model whose output limit is below that. Other CLIs ignore `output`.

### Agents, workers and busy hand-overs

An agent has one run at a time, over all its teams and solo tasks (`[limits] runs_per_agent`, default `1`; the agents
of `[tiers] workers` use `runs_per_worker`). When an agent of a task that is queued already runs another task, the
queued task waits — it is not failed — and `agent status` says `queued, waits for claude-1, running in team-…`. It starts
when that run ends. `0` restores the old behaviour, where an agent may run as many tasks at once as are queued.

The agents of `[tiers] workers` form one installation-wide pool. Any team or delegating solo task is served by a
free worker of the pool that meets its `requires`. The workers are chosen alternately across providers (taking turns
in the order their first worker appears in `[tiers] workers`), skipping busy providers without waiting. Within a
provider the workers take turns too: the provider's next request goes to its free worker after the one that served it
last, in configured order. A worker whose last request ended without a usable result (no result, or one sent back) is
passed over while any other worker that meets the request is free, of any provider; it serves again when nothing else
is free, and takes its turns again after a run that brought its result. The daemon remembers the turns and the failed
workers until it restarts. A worker serves each request in a dedicated task and workspace worktree in the requester's
repository from the requester's commit (`tasks/<requester task>/request-workspace-<n>.json`, `docs/state.md`), with no
place in the requester's team crew. A second try after a wrong result goes to another provider where one is free.
Passing an explicit worker to `agent run` is still accepted: that worker is assigned to the team's crew as a legacy team worker and serves requests of that team alone as before, held exclusively by that team for its lifetime.

A queued task that waits longer than `[replacement] busy_after` (300 seconds by default) for its busy agent is handed over to
a free agent of `[replacement] agents` of the same tier — normal for normal and investigator for investigator — choosing
the candidate whose CLI runs the fewest runs at that moment: the same worktree and branch, briefed with the reason
`busy`, the bus tells the whole team who holds each item now,
and `agent status` shows `for claude-1` next to the agent that holds it now and a `runs: <cli> <count>, …` line for the runs
going per CLI. Team notice obligations survive daemon restarts and transient bus failures, which are retried.
A notice that the bus permanently refuses, or that cannot be rendered, is logged and dropped; the obligation clears
when every notice has either been accepted or dropped. Only the agents listed in `[replacement] agents` are candidates: an empty `[replacement] agents` list (or
`busy_after = 0`) performs no busy hand-over and the task keeps waiting for its own agent. Tasks that are running, waiting
for an answer of the human or for a review, a coordinator while one of its members runs, a reviewer, or a worker are never
handed over. Pausing or stopping a queued task ends its busy wait; resuming starts a fresh `busy_after` interval.

**A refused agent is kept out.** A run that its provider refuses (a rate limit, an overload or an exhausted quota) pauses
its task until the time the provider names or the configured pause (`[limits] rate_limit_pause_minutes` or
`quota_pause_minutes`), and the agent is refused until that same time: it gets no new task, review or takeover before
then, and comes back to the pool by itself. The task is tried again in the same session `[replacement] limit_retries`
times (default `1`); when the retries are used up, or a quota's reset is later than `[replacement] limit_wait_minutes`
(default `60`), the rule answers "change the agent", which is logged as a `decision_taken` with `decision` `limit`.
The daemon hands a solo task or team member's task to the first free agent of another provider from `[replacement]
agents`, then `[tiers] normal`. A lead goes to a free investigator. Candidates already in the team, at their run
limit, or refused by their provider are skipped. The task keeps its worktree, branch and history, and the new agent receives a briefing. A team task moves to
the new agent's name on the bus before its next run. Each takeover counts against `[replacement] replacements`.
One notification and a
`task_agent_changed` history log record name the old agent, its provider and return time, and the new agent.
When none is free, the task stays paused and records `task_wait_for_free_agent` once. It starts when a suitable agent
becomes free or its original provider returns, whichever happens first, including after a daemon restart.
`agent status` shows a refused agent with its return time.

**What the notice of a refusal says.** The notice of a pause names what the provider said, from the fields of the CLI's
record when it writes them: the kind of refusal (an overload, a rate limit or a quota), then the status, the message and
the provider's code, for example `an overload: 429 · Model busy, retry later · engine_overloaded`. The raw line stays in
the run's files. The resume time in the notice is in the zone of the host, with that moment's offset, as the notice's
header is. Claude five-hour and seven-day usage windows are shown as quotas, with the final result's HTTP status and
message. A rejected window uses its future reset time (`resetsAt` of the `rate_limit_event`), shown in the host's zone;
a missing, expired or invalid structured reset falls back to the reset in the text. Long messages are shortened while
the status and provider code remain visible.

### Serena and the reminders

**Serena: finding code.** With `[binaries] serena` set, every run is given Serena, the MCP server that finds code
through language servers, and its first prompt says so in a short section (`prompts/harness/run/serena.md`) right after
the working context: find a symbol with `find_symbol`, its callers with `find_referencing_symbols`, what a file declares
with `get_symbols_overview`, and read only the body you need; keep `grep` and `rg` for text that is not a symbol — a
message, a configuration key, a log event, Markdown, a diff — with a narrow path and a limit of lines. Every role gets
it, a worker with the others (the delegation text names the same tools), and so does a new session; a resumed one does
not repeat it, and a run without Serena gets the prompt it always got, byte for byte. `agent debug` counts each run's
tool calls, Serena's and the shell's among them, so you can see whether the agents use it. A run that searches code
without Serena — `grep`, `rg`, `sed -n`, `cat`, `head`, `tail` or `find` over a file or a directory of code, or the CLI's own
`Grep`, `Glob` or `Read` over one — is reminded once in its session, at `[serena] remind_after` searches (40 by default,
`0` turns it off) and only if it never called Serena: one reminder of the prompts' catalog, naming the count and asking
for `find_symbol`, `find_referencing_symbols` and `get_symbols_overview`, while `grep` stays for text. The daemon puts it
on the bus while the run goes on, so the agent reads it at its next call of the bus; a daemon restarted in between does
not remind a session twice. It is logged as
`serena_reminded` and recorded in the history, and `agent debug` tells whether a run was reminded and how many calls
of Serena it made after. No tool is blocked. See
[configuration](docs/configuration.md#binaries) and [`[serena]`](docs/configuration.md#serena).

**What a Claude Code run is given.** A Claude Code run gets only agentbus and Serena (when `[binaries] serena` is set):
none of your own MCP servers and none of your claude.ai connectors (Claude Docs, Gmail, Drive, Calendar, ...), because
every run starts with `--strict-mcp-config`; no skills and no slash commands (`--disable-slash-commands`); and not the
built-in tools it has no use for in a headless run (`--disallowedTools`): `CronCreate`, `CronDelete`, `CronList`,
`DesignSync`, `EnterWorktree`, `ExitWorktree`, `ListAgents`, `ListMcpResourcesTool`, `Monitor`, `NotebookEdit`,
`PushNotification`, `ReadMcpResourceDirTool`, `ReadMcpResourceTool`, `RemoteTrigger`, `ReportFindings`,
`ScheduleWakeup`, `SendMessage` and `Workflow`. All of these only cost context at the start of every run. A session
started by an earlier version resumes as before, with the same flags as a new run. A run also reads no auto memory: the
settings of every run set `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`, so your own memory
(`~/.claude/projects/<repo>/memory/`) stays out of it, and `ENABLE_CLAUDEAI_MCP_SERVERS=false`, so your claude.ai
connectors stay out even if a run were not strict. Codex, opencode and Antigravity are not changed.

**Reminder to commit.** A solo agent, a coordinator and a member whose round is open and who hold many uncommitted
lines are told once, on the bus, to commit what is finished: when the worktree holds `[commit_reminder] lines` changed
lines (300 by default; added plus removed) and the run has made no commit for `[commit_reminder] minutes` minutes (30 by
default, counted from the later of its last commit and its start). Both bounds must be reached; a key at `0` turns the
reminder off. The daemon looks at most once a minute per task, never reminds a reviewer or a worker, and reminds again
only after the run has committed. It never commits, deletes or resets anything in the worktree. Each reminder is the log
event `commit_reminded`, and `commit_followed` tells when the run committed after it; the reminder is kept in memory, so a
restart of the daemon arms it again. See [`[commit_reminder]`](docs/configuration.md).

**Cut of a run that ignores the reminder.** A run that was reminded and has not committed `[commit_reminder] cut_after`
minutes later (20 by default; `0` turns the cut off) is stopped, once for the reminder, unless a gate of `agent gate`
runs for its task or a test run started in its worktree holds the machine's test lock (`TEST_LOCK_FILE`, by default
`/tmp/.make.command.lck`): it is then waited for. What the run left in its worktree is snapshotted, and the task is
queued at once to resume the same session, whose prompt is the order to commit what is finished first and then go on.
The cut counts as a stop of the harness, not of the human: no retry of the session, turn count, rejection or
replacement of the task is raised and the task never becomes `failed`, `stopped` or `paused` by it. A stop or a pause by
the human between the cut and the resume drops the order. The order is kept in memory: after a restart of the daemon
between the cut and the resume the task is resumed as after any interrupted run, without the order: with `[restart]
session = "new"` (the default) in a new session with the briefing of the task, with `"same"` in the old session with the
note that the harness restarted. Each cut is the log event `commit_cut`.

**Reminder of a full context.** A session whose context reaches `[commit_reminder] context_percent` of its bound (80 by
default; the bound is `[session] max_context` or the agent's own) is told once, on the bus, to commit what is finished and
to write in its report where it is and what is next: the next run may start a new session that sees only the commits, the
list of uncommitted files and that report. While a run goes on, the engine keeps the context of its latest model call in
`tasks/<task>/context.json`, and the daemon reads it at each look at the team. Claude Code, opencode and Antigravity tell
their context; Codex tells none, so it has no file and is never reminded, and a session with no bound is not either. A
resumed session is told once, a new one anew; the reminder is kept in memory, so a restart of the daemon arms it again.
`context_percent = 0` turns it off. Each reminder is the log event `context_reminded`.

### The plan of a task

A solo agent, a member and a coordinator keep the plan of their task in `.agent/plan.md` of
their worktree: its stages and steps, each marked to do, in hand, done or given up ([`docs/plan.md`](docs/plan.md)).
The daemon reads the file at the end of every run and keeps each new version in the history. A run of a role in
`[plan] roles` that ends `STATUS: DONE` while its plan is missing, does not parse, has a step still to do or in hand,
lost a step of its last version, or has a done step whose check failed (`mismatch`, sent back with the step, its check,
the reason and up to 40 trailing lines of output) goes back to the same agent with the reason, at most `[plan] bounces`
times per task (default 3; `0` only records); after that you are asked once, in the task's questions file, whether to go
on as it is or send it back once more. A step whose check matches `[plan] check_refuse` stays `declared`, and the task's
next run is told once in its prompt that the gate is never a step's check. Tasks created before plans were checked are never
sent back. The plan is kept out of git and out of the pull request: a plan an agent commits by mistake is taken out of
its branch by a commit "Keep the plan out of the delivery". See [configuration](docs/configuration.md#plan).

At every run's end, the daemon also judges done steps, for every role and outcome. `exists PATH` confirms a file or
folder inside the worktree; an absent path or a symlink outside it is a mismatch. A command check runs in the
background in the task's worktree, one at a time per task without blocking the daemon, with the agent's environment
and a timeout of `[plan] check_seconds` (default 120; `0` disables command checks and leaves them declared; commands
matching `[plan] check_refuse`, default `["agent gate", "make gate"]`, are refused and remain declared): exit `0`
confirms the step, while a non-zero exit, a signal or a timeout is a mismatch with output saved to
`tasks/<task>/checks/<run>/<step>.log`. Steps without checks or with `reviewer` remain declared. Stopping or
restarting the daemon kills any running check process group (`tasks/<task>/checks/running`) and, on startup, finishes
judging and settling any run that was waiting for its check without re-running the agent CLI or duplicating a
recorded judgement. `agent status <task>` and `agent status <round>` show these states beside the steps.
Recovery tolerates an unrelated process exiting while the daemon scans process groups, so that process cannot
prevent the daemon from restarting a pending check. Malformed process records still fail recovery with context.

### When a run goes wrong

A run ending without a `STATUS:` line resumes automatically in the same session with a short prompt
(`prompts/harness/run/resume-no-status.md`), up to twice in a row. After that the session itself is taken to have gone
bad, and the task is started once in a new session of the same agent with a briefing built from facts — its prompt, the
commits of its branch, the files it left uncommitted (kept in a snapshot first), who worked on the task and how each of
the newest ten runs ended, its plan of steps grouped as the harness judged them (confirmed, declared, mismatch, not done;
a step edited since it was judged, or never judged, is declared), the newest ten answers you gave to its questions and
the messages of its team it had not read (`[replacement] new_sessions = 1`; `0` turns it off). The same happens after the watchdog's last
strike for a hung run and after the last turn-limit resume. A run that is the first of a session and ends `STATUS:
FAILED` without calling any tool is retried once in a new session with a note telling it the bus tools are listed in
its CLI's tool catalog under the MCP server `agentbus`. This applies to every role, including coordinators and reviewers.
A second such failure names the agent, CLI and model in the human notice and shows the reason in `agent status`.
This one startup retry always runs, including with `[replacement] new_sessions = 0`, and counts against that session
budget. With the default `1`, a later hang, missing status or turn limit has no further ladder session available.
A run of a team member that reports `STATUS: FAILED` after it called a bus tool is not the end of its task either: the task is started once
more in a new session, briefed with the reason `failed` and the words its agent last wrote, then goes to another agent of
`[replacement] agents` or to the team's lead (who may use the member's worktree and branch), and the human is asked only when
no rung is left. The defaults of a configuration with no `[replacement]` section are `mode = "auto"`, `new_sessions = 1`,
`replacements = 1` and `agents = []`: one new session, then the team's lead. A solo task or a coordinator that reports `STATUS: FAILED` after a tool call still
ends failed. `[replacement] mode = "off"` keeps a member's failure final.
The named log events `run_no_tool_retry` (`info`) and `run_no_tool_failed` (`warn`) record both transitions with the
task, agent, CLI and model, so they can be monitored independently.
Every failed team member also wakes its coordinator with the member, item and last words, whether or not it used tools.
The coordinator may wait for the human or take the work only when its role permits it. When the new session goes bad too, the task is given — with
its worktree, its branch, its items and a briefing — to the first agent of `[replacement] agents` that is not in its team,
not in a run, and of the same tier as the agent it replaces — normal for normal and investigator for investigator
(`[replacement] replacements = 1`; `0` takes none over; a takeover is not counted among the new sessions). Before the replacement's first run, the daemon moves the old agent's membership on the team bus to the new agent;
the lead receives a message about the change. This also applies to a busy hand-over and to the coordinator.
If the bus is unavailable, the daemon saves the pending change and retries it on a later pass; the replacement
waits until the bus accepts it. A missing old member on an open team is treated as complete only when the
replacement is already a member; otherwise the change stays pending. A missing or closed team also keeps the
change pending. On upgrading, the daemon also queues hand-overs saved by earlier versions, so their
current holders join the bus before starting another run. Solo tasks make no bus membership change. With no such
agent — also when no `agents` are listed — a member's task stops and the team's lead
gets its work in a message on the bus, with a briefing of what the member's runs left (`[replacement] mode = "lead"`
asks for that at once). With `mode = "off"`, and for a solo task or a coordinator's with nobody to take it, the task is
marked failed and the coordinator or you are notified, as before. When the agent that took a task over goes bad too, or
a lead given a member's work does, you are asked once, in the task's next questions file: `a` starts it once more in a
new session; `b`, `ok` and any other answer stop it. Each hand-over is logged as `task_handed_over`, whose `reason` says
why (`no_status`, `hang`, `turn_limit`, `busy`, `round_overdue`, ...).

**A lead that repeats itself or cannot be reached.** When a task waits for you after "not published", or a lead answers
`agent say` and `agent resume` with the same run again, `agent renew <task>` is the way out that keeps the team and its pull
requests: the next run of the task starts in a new session of the same agent, briefed as after a bad session (the plan,
the commits, the files left in the worktree and the messages of its team it had not read — so say what you want with
`agent say <task> "..."` first, and it reads that before anything else). The briefing names the reason `human`; the reason
is also kept in `state.json` and in the history. A renewal you ask for is not counted against `[replacement]
new_sessions`, however often you use it. It works for a lead or a member, solo or in a team, and does what `agent resume`
does for the task afterwards. It is refused, with the reason, for a task that is running (stop it first with `agent stop`),
done, never run, a worker or a reviewer. The notice of a round that waits for your instruction names the command.

**A round that goes on too long.** A solo agent, a member or a coordinator that works on an item for more than
`[limits] round_minutes` (off by default; the example configuration gives `120`) without its pull request in review is stopped by the daemon, and the task is
handed over as above at once, without a new session of the same agent first: to the first fitting agent of `[replacement]
agents`, which is briefed in a new session on the same worktree and branch; to the team's lead; or, with nobody to take it
(`mode = "off"`, a solo task with no agent listed), the task fails and you are notified with the round, the item and the
minutes. You need no `agent swap` for it. The clock starts at the latest of: the opening of the round, the hand-over of the
task to the agent that holds it now, and the first run after a run that ended stopped — a task you stopped and resumed
starts its clock again. The daemon's cut of a run that did not commit (`[commit_reminder] cut_after`) resumes it in its
session and does not start the clock again, so an agent that keeps its work uncommitted is still stopped at the limit.
The time the task waits for your answer to its questions is left out of the clock, so a round you answer late is not
stopped for it.
A round in review, approved or being fixed after a review
is never stopped by this limit, nor a worker or a reviewer. The cut counts as no retry, turn or rejection. Each cut is the
debug decision `round_overdue` in the history, and its hand-over the log event `task_handed_over` with
`reason = "round_overdue"`. `round_minutes = 0`, the default, turns it off.

**Returns of a lead.** When the lead of a team sends the work of a member back instead of taking it, it ends its final
message with a line `RETURNED: <item>` beside `STATUS:` (its role text says so). The daemon counts the line only where
its own facts agree: the member's last run on the item ended `DONE` and the member is started again for it. The count is
kept with the item, survives a restart, and `agent status` shows it on the member's line as `returned 3 of 5`
(`[replacement] rejections = 5`; `0` has no limit) and on the round's line when one more return reaches the limit. A
lead that writes no such line is never counted. The count is shown only; reaching the limit does not yet replace the agent.
`agent status` shows such a task as `in a new session`, and as `, for AGENT` when another agent took it over from AGENT. Automatic retries apply only to resumable tasks with a session (solo tasks,
coordinators, and team members), excluding workers serving requests, reviewers, tasks without a session, or failures
during startup or execution. Runs ending without a status line upon daemon shutdown or interruption (exit code `None`)
count toward the consecutive limit; an interruption that exhausts the budget fails the task before another run starts.
When a member fails or ends without status while
holding an item, the coordinator receives a notice naming the item and the member's branch
(`prompts/harness/team/bus-member-failed-item.md` or `bus-member-no-status-item.md`). For a member that fails after a hang, quota limit, rate limit, stop or unusable reported result, the notice preserves how its run actually ended and includes its last words or the end reason. When the lead has reported
`STATUS: DONE` and waits while an item remains blocked behind a failed member, the human is told immediately once per
item (`prompts/harness/pr/notice-standing-item.md`) with the member's last-run details and commands to speak to the
member and resume it (`agent say <task>`, `agent resume <task>`). Resuming a task clears the notification deduplication
for items assigned to that task. Existing notification files identify the item and the failed run, preserving suppression
across daemon restarts; a later failed run can notify again. An item does not stand behind a member that the human stopped,
or that is no longer in the team: when the lead ends `STATUS: DONE` with a passed gate, the item is published with the
lead's branch.
A member that failed after it called a bus tool does not hold the round either when its lead's branch already holds its
work: when the lead ends `STATUS: DONE` with a passed gate at the tip of its branch, the member's branch has no commit the
lead's branch lacks and its worktree holds no uncommitted file the lead was not told of, the member's task is closed as
taken over and the item is published, with no hand. A member that never called a tool did no work: it keeps holding the
round and the human is told, as before. Work the lead's branch lacks is never dropped, also that of a member the human
stopped: the lead is told once, in one message for all members, which commits and files
(`prompts/harness/pr/member-work-lacks.md`; the files are kept in a snapshot, and a file that changed afterwards is told
again), and when it ends `STATUS: DONE` again without the commits, the human is asked once with the list, also after a
restart of the daemon (`prompts/harness/pr/notice-member-work-left.md`).

### Snapshots, restarts and disk

Snapshots keep a worktree's work when its run was cut. When the daemon starts, it takes a snapshot of every task whose run did not end, if the round that run worked for is still open: everything in the worktree (uncommitted, staged, deleted, untracked and binary files; ignored files are left out) becomes a commit under `refs/agents/snapshots/<round>/<task>/<n>` in the repository, and the worktree, its index and its files do not change. `agent status <round>` shows the last snapshot of each task with its number, time and size. A snapshot above `[snapshot] warn_mb` (default `200`) is still taken and logged as `snapshot_large`; a snapshot that fails is logged as `snapshot_failed` and the daemon goes on. Snapshots are never pushed. The snapshots of a cut run are never pruned yet: remove their references with `git update-ref -d` when you no longer need them; the snapshots of checkpoints (below) are removed by the daemon. Two events and one history table (`snapshots`) describe them, see [`docs/history.md`](docs/history.md).

**Checkpoints.** After every step of a round the daemon records a `checkpoint` in the history (`history.db`, schema 34) and keeps a snapshot of the holder's worktree under `refs/agents/snapshots/<round>/<task>/c<n>` (numbered apart from the snapshots of a cut run, which have plain `<n>`), so that a restart of the agent, of the daemon or of the machine, a replaced agent or a replaced team can go on from the last step and not from the start of the task. A checkpoint is taken after: the opening of a round's work (`round_opened`), each new local commit of a lead or a member, also one that is never pushed (`commit`; the daemon looks at the worktree of every task with a run going once a minute and takes one checkpoint for all the commits that came since the last), the pull request opened as a draft or not, pushed, marked ready, checked, approved, merged, handed over or closed, a split decision, every review submitted (one checkpoint per reviewer), a fix pass started, every answer to a finding and its judgement, a question asked and answered, a merge refused for a conflict, a failed member's item that waits for you after "not published", the end of a run, the round limit cutting a round (before the task is taken over), a hand-over, a renewal or a new session (`holder_changed`), and the start of the daemon for each task of an open round whose worktree differs from its last checkpoint (`daemon_started`). A checkpoint says the round, the item, the holder's task and tier, the cause, the pull request and whether it is a draft, the branch's tip (`head`), the phase and the review and fix pass, and the snapshot reference. It also says, in three parts, what is meant (the task, the item and its numbered requirements `R1`, ..., `K1`, ...), what is done (each requirement with its state `met`, `partly met` or `not started`, the steps and the commits with their `Step:` trailer that show it, the findings resolved and the review passes passed; steps and commits tied to no requirement come last) and what is left (the requirements not met, the open findings and one `next` line for the phase: commit and hand the round over, wait for the reviews of a pass, fix the open findings, wait for the human or wait for the merge). A list longer than 40 lines is cut with `<k> more left out`. When the worktree is the same as at the round's last checkpoint (same head, same tree) the new checkpoint reuses its reference and takes no new one. The references of a round's checkpoints are kept while the round is open and removed when it is merged or closed. A checkpoint that cannot be taken (the worktree is gone, git or the disk fails) is logged as `checkpoint_failed` and recorded without a snapshot; the daemon goes on and never stops a round for it. The daemon waits at most 2 seconds for the history to have a checkpoint's record before it acts on the next step; a history writer that is stuck (a database another process holds) is logged once as `checkpoint_failed` with `cause=history_stalled`, and the round is not held up.

**Checkpoints of workers, reviewers and investigators.** A worker's request, a review and an investigation keep checkpoints too, with the same record, the same three parts (what is meant, what is done in relation to it, what is left) and the same rule: whoever goes on starts from the last one. The `job` of the record says which (`item` for the checkpoints above, `worker_request`, `review` or `investigation`). For a worker's request, what is meant is the request's text and its check, what is done is the commits and changed files of the request's worktree and the last result of the check, and `next` is `make the check pass and commit`; its checkpoints are taken after `request_started`, each `commit`, `request_checked` and `run_ended`. For a review, what is meant is the pull request, its head and the pass, against the item's requirements; what is done is the findings the reviewer has written so far; `next` is `finish the review of pass <p> and submit it with a verdict`; its causes are `review_started`, `findings_written`, `run_ended` and `review_submitted`. The review prompt now names a findings file in the task's directory (`review-pr<n>-pass<p>-findings.md`): the reviewer appends each finding to it as it has it, and the once-a-minute look takes a `findings_written` checkpoint when it changed. For an investigation, what is meant is the question, what is done is the notes and files of its worktree, and `next` is `answer the question`; its causes are `investigation_started`, `run_ended` and `investigation_answered`. The once-a-minute look for new commits covers the worktrees of these jobs while their run is going, and a start of the daemon takes a `daemon_started` checkpoint of each job that was going. A worker's request that goes back to the queue is started by the next worker in the same worktree with a briefing of the last checkpoint ("This request was started before you"); a reviewer that is replaced hands its findings to the next reviewer, which keeps the pass number ("This review was started before you"); an investigation tried a second time starts from the first investigator's checkpoint ("This investigation was tried before you"). When a job passes to another holder the daemon takes a `holder_changed` checkpoint of the new holder (the same task for a worker's request) that tells what the last checkpoint of the holder before told, and the new run is briefed from the newest checkpoint of its own task: a reviewer that runs next to another one of the same pass is therefore not told it replaces anybody, and a task that runs again after a restart starts from its own last checkpoint. After a start of the daemon the last result of a worker's check is read back from the checkpoint that tells it. A findings file that cannot be read is logged as `checkpoint_failed`. The checkpoint of a job is never mixed with a round's own item: `agent checkpoints` of a round lists both, and a line of a job ends with `job <name>`.

**`agent checkpoints <round|task>`** lists the checkpoints of a round (`round-…`) or of a task (`task-…`), oldest first, one line each: the time, the cause, the pull request and whether it is a draft or ready (`PR none` before one), the head, the review pass, how many commits and steps are done, how many steps and findings are left, and the snapshot reference (or `none`); the line of a checkpoint of a worker's request, a review or an investigation ends with `job worker_request`, `job review` or `job investigation`, and `agent checkpoints <task>` works for the task of a worker, a reviewer and an investigator as it does for a lead's. `--show [N]` prints checkpoint N (1 is the oldest; without N, the last) as an agent is told it: what is to be done, what is done and what is left. `--json` prints the records as JSON lines, as `agent log --json` does. A target with no checkpoint prints `No checkpoints for <target>.` and exits 0 (with `--json` the words go to the error stream, so that what is printed stays JSON lines and is empty); a round or task the history does not know exits 3 and a word that is neither a round nor a task exits 2, as `agent log` does. The command reads the history file itself, like `agent log`, and needs no running daemon. To restore a snapshot by hand, in the worktree of the task: `git checkout <snapshot reference> -- .` (the reference is the last column of the listing).

A run the daemon's stop or crash cut goes on, by default, in a **new session** of the same agent, in the same worktree and on the same branch: its first prompt is the briefing (`prompts/harness/run/briefing.md`) with the reason `daemon_down` — the run was cut from outside, not by the agent's fault. The briefing lists who worked on the task and how each run ended, the commits, the files left uncommitted (the snapshot above is taken first; they stay in the worktree byte for byte), the plan's steps as the harness judged them (confirmed, declared, mismatch, not done), your answers so far, and tells the agent to find the real state first, trust the confirmed steps, check the declared ones and repair the mismatch before anything else. The round keeps its pull request and its review passes, and the cut counts as no retry of the task. `[restart] session = "same"` in `agents.toml` resumes the session the run was cut in, as before. A stop of the daemon at the last retry of a session is no retry either. The briefing tells only the last run before a `daemon_down` session as cut by the daemon: the record of a run keeps no mark of an earlier cut, so an earlier stop is told as "no status". A reviewer, a worker and a task whose round is closed resume as before.

The briefing also has a section **The pull request and its review**, built from what the daemon already keeps of the item of the task's open round (no new state): the pull request (`Pull request #<n> on branch <branch>, opened <time>.`, or `No pull request yet.` when the task has no open round or its item has no pull request), where the review stands (in review, in a fix pass `n` of the limit, waiting for the human after a pass was sent back to its limit, approved, or not in review yet) and the findings of its reviews, one line each by permanent number with its severity, place, text, status (`open`, `resolved`, `not confirmed`), the pass and the reporter. With more than 40 findings the open ones are listed in full and one line says how many others are left out; a round from before the numbered findings says so and points to `gh pr view <number> --comments`. The briefing then tells the new session that the pull request is its own: it goes on in that pull request, opens no other and fixes every `open` finding. The section is part of every briefing, so a renewed task (`agent renew`), a task taken over because its agent was busy and a task taken over after failures all get it.

The briefing opens, after its first paragraph, with **the last checkpoint** of the task's round, told as `agent checkpoints --show` tells it (what is to be done, what is done per requirement, what is left), and the line "This is where the work stands. Go on from it: do what is left, do not do again what is done." It is the task's own newest checkpoint of the round, or the round's newest when the task has none; a change of holder, and the end of a run that left the worktree as the checkpoint before it kept it, are skipped, because they tell the same work as the checkpoint before them. A round with no checkpoint says `No checkpoint yet.` The snapshot of the briefing is the snapshot of that checkpoint when the worktree is as the checkpoint kept it (the same head and the same tree): no new snapshot reference is made, and the briefing still names the checkpoint. A worktree changed since gets a new snapshot. Every new holder's first prompt is this briefing, from the last checkpoint: `agent renew`, an agent that was busy elsewhere and is replaced, a failed member, a round the round limit cut (`round_overdue`: the commits that were never pushed are under "What is done", and the plan's steps that are not done and the line `commit what is finished, run the gate and hand the round over` under "What is left"), a new session after the daemon or the machine went down, and a session that ended three times without a status. A lead that is given the work of a member is told the same checkpoint in a section of its own, "Where the member's work stands". A history that cannot be read fails the start of the new session, with the error of the runs, as before.

Build directories (`target/`) are released when a round closes, and when a reviewer finishes while free disk space is below `disk.low_free_gb` (default `100`), logging `build_directory_removed`. Only `target/` within harness worktrees is ever removed. Free disk space is continuously tracked: below `disk.low_free_gb`, a notification lists the five largest worktrees and `agent status` reports the free space; below `disk.min_free_gb` (default `20`), `agent run` is refused with cleanup guidance, open rounds wait with a single notification, and waiting rounds resume automatically once free space is restored.
`agent status` reads the last measured free space from `AGENTS_HOME/.free_space`, a plain number of bytes that the daemon writes at each measurement; the line is absent until the daemon has measured, and shows the reason when the file or `config.toml` cannot be read. When a state file cannot be saved, the daemon first runs the notification command once with the reason as its only argument and no file is written; a failure of that try is logged and the daemon stops as before.

### Investigators, tiers and splits

The daemon keeps each team's investigator activities in `state.json`: investigator, task, rounds,
assignment time and terminal outcome. Activities distinguish `team-member` (coordinator or member), `reviewer`
(first review, verification or second opinion), `investigator` (questions and explanations) and `task-allocator`
(task splitting and integration). Crew membership and review work are recorded today; the latter two kinds do
not introduce new workflows. Reports remain recorded between reviews and after the item advances; stopped and
failed assignments retain their ending time and the reason known by the daemon.

A state written before the activities were kept has them rebuilt from its crew, reviews, fix passes and second
opinions. The activity record also supplies the investigator selection order (see
[Pull requests, reviews and merges](#pull-requests-reviews-and-merges)) and the view `agent status --investigators`.
See the [activity record](docs/state.md#investigator-activities).

Task-list items can declare an optional column-zero [`Tier:` directive](docs/task-list.md#the-lines), defaulting
to `normal`. Core decisions compare capabilities as investigator > normal > worker and prefer a fitting member
before a fitting coordinator. The daemon applies the requirement to initial items and every split part,
including the first. A busy qualified member keeps its place; the coordinator takes an item only when no member
in the crew meets its tier. If capability is absent, the item stays pending across restarts and `agent status`
says, for example, `1.1  waiting for investigator agent`. Assignment prompts name the required tier and the
selected agent. A failed member’s work also keeps its required tier when offered to the coordinator.
Review fixes follow the same minimum tier: when the coordinator does not qualify, a qualified member makes the
fix and answers the findings, then the coordinator integrates its commits before the same pull request is verified.
A review fix with no qualified agent remains waiting, including after a restart. Workers
remain available through delegation. Approved split plans can retain who approved them.

A coordinator can submit a [split proposal](docs/task-list.md#coordinator-proposal-form) by writing
`split-proposal-<item>.json` in its task directory and reporting `SPLIT: <item>` before `STATUS: SLEEPING`.
The daemon approves a complete valid plan without human intervention when it keeps the scope, sends faults back
up to [`[split] corrections`](docs/configuration.md#split), and asks the human once for scope changes or exhausted
corrections. Estimates are compared against `part_limit` in the configured `unit` (default `lines`). Members'
requests go to their coordinator. Pending input, corrections and existing questions survive restarts.
Decisions appear in history as `split_decision` and, at debug log level, as `decision_taken` with decision
`split_proposal`. A `SPLIT:` line the harness cannot take — the item is split already, or the team has no item of that name —
is answered, not dropped: it is recorded in the log and in the history as a correction with its reason, the coordinator is
sent a text of the catalog on the bus that says what was refused, why and what to do instead (deliver the part of its round;
what does not fit is described for a task of its own), and it is woken for it, with no question to you. The status the run ended with is acted on as before, and a coordinator that repeats the
line is told as many times as a proposal is corrected (`[split] corrections`, and at least once); after that only its own status counts. Every refusal is in the log (`request_refused`) and the history. Only `SPLIT:` and an
item number is a submission. Requested tiers are retained;
items and parts are assigned only to agents meeting their minimum tier. When none fits, the harness writes one
questions file for the lead and keeps the item waiting. An answer goes to the lead without permitting a lower-tier
assignment; restarts do not repeat the question.

A coordinator can ask the human to split an item into independently deliverable parts in one questions file.
Approve using the [explicit split verdict](docs/task-list.md#approved-split-plans-core-format); `agent answer`
lists the recorded parts, or explains a rejected form. The current round delivers part a. Later parts receive
separate rounds, branches and pull requests after their dependencies merge; the original item and team remain
open until all parts merge. Each part's pull request and a line in `agent status` name its original item, letter,
sibling pull requests so far, approval authority, requested tier and assigned agent. Historical executors remain
visible after review and restart; facts missing from older states say `not recorded`. Definitions and progress
survive restarts.

### Diagnostics: `agent debug`

`agent debug` writes a read-only diagnostic bundle for one team, round or task — exactly one of them:

```sh
agent debug --team TEAM_ID      # or --round ROUND_ID, or --task TASK_ID
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

The summary has a section `## Accounts` before the machine's: each account of `agents.toml` with its owner, the agents
and investigators that run with it, the names of the variables of its `env` and the names of the variables of its
`[account_env.NAME]` in `secrets.toml` — never a value. A configuration that does not load is reported as an unreadable
part of that section.

Task files and git output are bounded to 4 MiB each; task files together to 64 MiB, with at most
4096 entries per task and 16 directory levels. Limits and skipped symlinks are reported as read failures.
History is read from a temporary copy of the database and its WAL/SHM files, including committed events while
the daemon holds the home. Each source file is bounded to 64 GiB; up to three snapshot attempts accommodate
source changes before they are reported as unreadable. SQLite creates no files beside the original database. Summary timestamps use the machine's local time zone at each moment (`YYYY-MM-DD HH:MM:SS`, or with `UTC` after it when the zone cannot be found), as `agent log` does.
Each task shows its latest run's reason, final message and steps, and how many tool calls that run made, Serena's and
the shell's among them (`tools: 214 (serena 12, shell 160, other 42)`, or `tools: none recorded` when its steps name
no tool): the count comes from the steps the run recorded in the history, so it tells whether the agents reach Serena
or grep the code. Each earlier run of the task has its own count on a line of its own, before the latest run's. Earlier
runs remain in the bundle.
Questions are ordered numerically, and gate outputs name their source and modification time. Worktrees removed
after a merged round are explained once, with retained branch facts read from the caller's clone.
An unpublished member branch is expected; the coordinator's origin ref is still checked.
The bundle of a team (`--team`, `--round` or `--task`) also includes the team's pool worker tasks, taken from the
history: for each worker task its `context.json` and the `out.jsonl` and `err.log` of its last three runs, bounded and
cut of secrets like member files, and a `workers` list in the summary; and `--task` also accepts a worker task and
writes the bundle of the team that requested it.

The summary uses the pure anomaly assessment and stays below 100,000 bytes; full collected details are separate
artifacts. Summary redaction precedes truncation and byte budgeting, including daemon logs and process command lines.
Collection includes machine metrics (load average, memory and swap, filesystem space and inodes for home and temporary directories, daemon descendant processes with age and redacted command lines, and directory sizes for tasks and worktrees) and the fifty newest untracked files per worktree from a single git status listing. Filesystems at least 90% full by space or inodes trigger an anomaly. Complete untracked listings are saved as `untracked/<task>.txt`, separate from copied task files. See the
[diagnostics design](docs/architecture.md#debug-diagnostics-commands-and-pure-decisions).

### History and `agent log`

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
`operation`. The tries in between log only the `github_call_failed` warnings of the calls themselves.

Read a round with `agent log --round <round-id>`, or a team's rounds with `agent log --team <team-id>`. Both read the history directly and work with the daemon stopped.
Use `--prompts` for whole prompts, `--no-steps` to omit individual steps, or `--json` for every record exactly once
as JSON lines. In text, adjacent reviewer run blocks of the same team, pull request and pass follow their first recorded
review requests, and the pass's review reports follow all of those runs; if any request is missing, their original order
remains. Every other event stays in place, run timestamps and contents are preserved, and JSON stays chronological. An unknown round or team exits with code `3`. See [`docs/log.md`](docs/log.md) for the story format,
ordering and exit codes. `agent log` reads a history of an older schema without changing it; the daemon migrates it
when it opens it for writing. Worker tasks are shown like member tasks: `agent log --task task-…-e8j7`
prints the runs and steps of a pool worker.

Bare `agent log` prints the newest `100` daemon log rows. Use `--show log` with a round or team to
select rows instead of its story, or add a row filter: `--task`, `--run`, `--agent`, `--level`, `--event`,
`--since`, `--until`, `--grep`, `-n` or `-f`. Filters intersect; `--json` emits one log row per line in this mode. Combining
`--round` and `--team` also selects rows. A row query with no matches succeeds with empty output.

A line of a row names the task, the agent, the run and what changed (`task-… (claude-1) …: queued ← waiting`); other events
print their fields as `key=value`. With `--task`, `--agent`, `--run`, or with `-f` and a round or a team, `agent log` also
prints the steps of the runs it covers — the agent's messages, commands and tool calls, and their results — between the
rows, as one text of the newest `-n` lines (default `100`); `-f` goes on with the new rows and steps as they are recorded,
across runs and daemon restarts, and says what the round waits for when a run ends. `--no-steps` leaves the steps out;
`--json` is unchanged. See [`docs/log.md`](docs/log.md).

`agent log --reminders [--days N]` tells, agent by agent, whether the reminders of the harness work: from the rows
of `commit_reminded`, `context_reminded`, `commit_followed` and `commit_cut` of the last `N` days (`7` by default) it
prints one line for each agent and model — how many times a run was reminded to commit (`commit`), how many times a
session was told its context is large (`context`), how many commit reminders were followed by a commit (`followed`),
the median minutes from the reminder to that commit (`median`, the lower middle value for an even count; `-` when
none was followed) and how many runs were cut (`cuts`); the model is the one of the run's record, `-` when it has
none. It reads the history without the daemon and takes no round, team, row filter or `--json`. With no rows it
prints `no reminders in the last 7 days`, also for a history written before these events.

```text
agent     model       commit  context  followed  median  cuts
claude-1  sonnet-5-5  3       1        2         9 min   1
```

Use `--grep TEXT` for literal FTS5 search, `-f` to follow new rows across daemon restarts, and `--stats` for database
size, table counts and the oldest timestamp. A scoped `--show` selects `tasks`, `agents`, `worktrees`, `runs`, `prompts`,
`changes`, `reviews`, `log` or `usage`. `[history] keep_days` removes expired closed rounds and their gate output;
`[log] keep_days` removes old rows of no round. Both default to `0` (keep all), and open rounds always stay.

The text of `agent log` shows times in the zone of this machine, as `date` does, without the offset written after them (`2026-11-21 17:29:53`; a time that cannot be shown locally is written in UTC with `UTC` after it); `agent log --utc` shows
them in UTC, as the files hold them, and `--json` is always UTC. A `--since` or `--until` without a zone is read in the
zone of this machine.

The story of a round or a team tells each run's usage at its own end, in one line that names the run:
`run <id> usage: context first N, last M; output K; cost C USD`. The context is what the CLI's stream told: the input of
the run's first and last model call with what came from its cache. A part the CLI did not tell reads `not told`, and so
does a whole run without a row. `agent log --task <task>` and `--run <run>` tell the same line right after the
`run_ended` row of each run (after the text, for a run whose end row is not in it), and `--show usage --json` gives a
row of numbers (`null` for a part not told) for each run that has usage. A history written before usage was kept prints
no usage line, and `--json` stories carry none.

### Answers and controls

How the human answers the agents and steers a task or a team, through the daemon (`agent daemon`) that owns the state:

| Command | What it does |
|---|---|
| `agent answer <task>` | opens the questions file of the task in your editor (`[questions] editor`, else `$VISUAL`, `$EDITOR`); when you close it, the answers are sent and the task goes on with them |
| `agent answer <task> --file answers.md` | sends the answers written after `**Verdict:**` in a questions file instead of opening the editor |
| `agent stop <task\|team>` | ends the run; nothing of it starts until it is resumed |
| `agent pause <task\|team>` / `agent unpause <task\|team>` | holds the task or team, and lets it go on in the same session |
| `agent resume <task\|team>` | starts a stopped, failed or waiting task again; when steps of the team wait for GitHub or git, it tries those at once and starts no task |
| `agent renew <task>` | starts the next run of a task as a new session, briefed from its checkpoint, its commits and its unread messages; not counted against `[replacement] new_sessions`; see [When a run goes wrong](#when-a-run-goes-wrong) |
| `agent checkpoints <round\|task>` | lists the checkpoints of a round or a task, oldest first; `--show [N]` prints one, `--json` prints JSON lines; see [Checkpoints](#snapshots-restarts-and-disk) |
| `agent remove <team> [--force]` | removes a team after bundling its unmerged work; `--force` first ends its active runs and checks. See the removal details below. |
| `agent say <task\|team> words…` | a message to the agent or the team, which reads it at its next look at the bus; a word to a task reaches the agent's session in that task's team only |
| `agent status` | the short view: every team with its rounds, a line for each of its tasks with what it does, and a count of all its tasks, by what they do (`tasks: 1 running, 1 waiting, 1 with questions; questions number 1 wait for answers (claude-1, task-…)`). The tasks listed are the team's coordinator, members, workers and reviewers (a solo task too), whatever their status; a reviewer whose review is over is listed with when it finished. The plans of the tasks and the parts of a split are not listed; a question that waits for the human is named with its number. When workers are configured, a `pool:` section shows each worker, whom it serves (team or task, step) or that it is free |
| `agent status --task` | as `agent status`, with the lines of all the tasks, their plans and the parts of a split, as `agent status` printed them before 0.58.0. Refused with a team, a task or a round, which already show their tasks, and with `--investigators` |
| `agent status --running` | only the teams that have a round that goes on, with those rounds: what is being worked on now and who works on it. The short view as `agent status`; the parts of an item whose every part merged are left out, its work being over. Under each team, before its count of tasks, a line for every request of its tasks that a worker took, in the columns of the tasks' lines: `  task-20261125-075745-w8vj  worker       glm-1  step 6.1  accepted, 2026-11-25 08:57–08:59` — the worker's task (`agent log --task` takes it; its runs are in `tasks/<task>/runs/`; `no task yet` before the request was given one), the role `worker`, the worker and the step in the column of the agents (a worker with its step wider than every agent widens that column for the whole list), what became of the result (`accepted`, `accepted after correction`, `rejected`, `not rated yet`, `working`, `queued`, `failed`, `unclear, asked back`), then when the worker's runs started and ended (`since …` while it works, `not started` before). Its `pool:` section names only the busy workers of teams not listed; a free worker is not shown |
| `agent status --running --task` | as `agent status --running`, with all the tasks, their plans and the parts |
| `agent status <team>`, `<task>`, `<round>` | the team or task with its tasks, or the round as before |
| `agent status --investigators` | what each configured investigator is doing: its last two activities in the teams that are not closed (no versions line) |

### Removing a team

`agent remove <team>` removes only that team's state, local branches, worktrees and task directories.
It refuses an unknown team or a team with active runs, gates or worker checks. With `--force`, it ends and joins
that work before deleting files. Previously halted checks are joined even without `--force`; pending disk cleanup
finishes before the freed bytes are measured. Plain document or artwork directories are removed without Git inspection,
including teams whose worktrees are all plain directories. Their freed bytes are included in the result.

Before deletion, unmerged local branches and worktree HEAD commits go into `AGENTS_HOME/removed/<team>.bundle`.
The command verifies the bundle and reports its path; restore a branch with
`git fetch /path/to/removed/<team>.bundle refs/heads/<branch>:refs/heads/recovered-work`.
Detached HEADs are available as `refs/agent-removed/<task>` in that bundle. A failure to create or verify the
bundle prevents deletion. Every worktree with uncommitted tracked changes is named in the output; the bundle
keeps commits, so save those changes separately before removal if you need them. Untracked files alone do not
produce that warning. Open pull requests are closed with an explanatory comment; their remote branches stay.

The team disappears from `agent status`, including after a daemon restart. History and existing notifications
stay. One new notification and a `team_removed` log event report the team, reason, bundle and bytes freed.
The byte count covers successfully deleted directories; cleanup failures are listed as left behind with their
causes. Removal saves the new state before deleting local files; a failed state save preserves those files and
the live team, although a pull request already closed stays closed.

**Current limitations:** removal requires a surviving worktree to locate a Git repository. If none remains,
it refuses without deleting state or files and names the known local branches: keep one worktree, or bundle and
delete the branches by hand.
The command marks the team done on the bus, but **the bus itself is neither read for wake-ups nor cleaned**.
The output reports known reservations with their paths and each removed task's persisted wake-up budget
(task, message number, count and state-file path). Bus unread messages and reservations remain; reservations
last until expiry or manual release. Releasing these bus records is a separate follow-up.

**Automatic idle removal:** by default, the daemon checks at startup and once per minute and removes a team
after two hours without a run or new commit. It uses the same bundle, cleanup and notification safeguards as
`agent remove`, including for document and artwork teams. Running work and checks, human pause/stop, quota or
rate-limit reset waits, and accepted claims waiting for another team's pull request to merge protect the team.
Work queued for an agent, including provider retries and rounds awaiting a free investigator, also protects the team.
An unanswered question does not protect it. Idle teams are removed **without starting replacements**;
`[limits] idle_team_restarts = 1` is reserved and has no effect yet: the state already keeps what a replacement will
need ([recovery state contract](docs/state.md#recovery-foundation)), but the daemon does not start one.

Set `[limits] idle_team_minutes = 0` to keep idle teams. Idle time counts from the later of the team's last run or
commit and the current daemon startup, so downtime never counts and an old team gets a full interval after a restart.
Observations and a reported refusal survive restarts.

Before automatic deletion, enabled history must acknowledge a snapshot of the last activity, items, split parts,
rounds and branches. If history or removal fails, the team stays and one notification names the refusal and its
branches. While the team remains idle, the daemon retries once a minute without repeating that notice. Each
retry requires the history acknowledgement and removal safeguards again. Disabling history is an explicit
opt-out from the history requirement; bundles and removal safeguards still apply.

### `agent status`

Every list of `agent status` starts with the versions of the four crates (`agent 0.58.0 · agent-core 0.58.0 · agent-runtime 0.58.0 · agent-web 0.58.0`). The daemon tells its versions with the answer to a status request; when they differ from the command's, a second line says the daemon's versions and that they differ, and a daemon that tells none is shown as `daemon: version unknown`. `agent status --json` stays the bare array of teams, byte for byte as before, with and without `--task`: it carries no versions, since adding them needs a wrapper object that would break scripts that read the array.

The short view of a team, as `agent status` and `agent status --running` print it: the team, its rounds, its tasks
and the count. A task that runs says when it was last active; a reviewer says what it does or did (`working`, when its review
finished, or when and why it stopped) instead of the status of its task:

```
team-20261111-153000-t5m8  team-agents  autopilot  1 pending, 1 in progress, 0 in review, 0 approved, 0 done
  round-20261111-153001-q1w2  item 1.1  open, only local  started 2026-11-11 19:15
  task-20261111-153001-k3x9  coordinator  inv-opus   running, last activity 2026-11-11 19:20:11
  task-20261111-153002-k3x9  member       claude-1   questions, questions number 2 wait for answers
  task-20261111-153003-k3x9  worker       glm-1      done
  task-20261111-153004-k3x9  reviewer     inv-codex  working, last activity 2026-11-11 19:19:58
  task-20261111-153005-k3x9  reviewer     inv-codex  done, 1st review finished 2026-11-11 19:16:30
  task-20261111-153006-k3x9  reviewer     inv-codex  done, 2nd review finished 2026-11-11 19:18:05
  tasks: 2 running, 1 waiting, 1 with questions, 2 done; questions number 2 wait for answers (claude-1, task-20261111-153002-k3x9)
```

Every task of the team has a line, whatever its status: a reviewer is listed after its review as it is while it works,
with when its review finished, or how it stopped or failed. `--task` lists every task, and under each its plan
(`[x] 1.1 …`), and the parts of a split.

After a split part merges, the next part uses an available, qualified previous holder, then the configured
`[replacement]` policy: an eligible replacement in configured order, or the lead. Stopped and failed tasks are
excluded. If no fitting participant is allowed, the human gets one capability question; a busy fitting participant
waits until available. An outside replacement keeps the task and worktree but starts its own CLI session, retaining
the previous holder's history. Existing states, including splits left waiting by older versions, load and resume
this scheduling on the next daemon poll. The optional `holder_after_run` state field records the session boundary;
older tasks omit it and retain their existing session behavior until a handover.

Every view of `agent status` prints the same state for a part of a split, whether its round is open or closed:
`done` when a round of the item merged the part's pull request, `in progress` while its round is open, `to do` when it
is next, and `waiting for 1.1a` (and the other parts it depends on) until they are over. A part whose round was closed
without a merge is not `done`. In `agent status --running`, an item whose every part merged shows no part line.

`agent status` ends the line of a task that runs with `last activity YYYY-MM-DD HH:MM:SS`: the local time the output of its run last
changed (the file `tasks/<task>/runs/<run>/out.jsonl`), which is how to tell a task that works from one that hangs. Every time `agent status` prints is in the zone of this machine, as `agent log` writes it, without the offset after it; `agent status --json` stays in UTC.

The line of a reviewer says what it did or does, from the record of the team's investigators, instead of the one word of
its status (a reviewer that waits between its reviews said only `waiting`):

| Says | Means |
|---|---|
| `working`, with `last activity YYYY-MM-DD HH:MM:SS` while its run writes | a review goes, or one is queued for it |
| `done, 1st review finished 2026-11-19 12:08:17` | it gave the report the harness asked for; the ordinal counts the reviews it gave for the round (the first review, the verifications and the second opinions alike; a review that ended without its report is not counted) |
| `stopped at 2026-11-19 12:10:00, reason: the run showed no activity for too long` | the task was stopped or failed; the reason is the one the daemon knows (`the human stopped it`, `the run ended without a status`, and so on) |

A reviewer task without recorded activity shows the word of its status as before. The lines of the coordinator, the
members and the workers do not change.

`agent status --investigators` shows every investigator of `[investigators.NAME]` in `agents.toml`, in the order of
their names, with the last two of its activities in the teams that are not closed, the newest first: the task, the kind of
the activity (`team-member`, `reviewer`, `investigator`, `task-allocator`), the investigator, the team, and what it did:
`done at 2026-11-19 12:15:15, 1st review finished`, `working, 2nd review`, `done at 2026-11-19 12:00:00, tasks allocated`, or, for one that
ended without its report, `stopped at 2026-11-19 12:10:00, reason: …` as on a reviewer's line. A team counts as open while it has a
round that goes on. An investigator with no activity in an open team is listed as `free, no activity in an open team`:

```text
task-20261119-094409-saal  reviewer  inv-gemini-argon  for team-20261119-094000-saal  working, 2nd review
task-20261119-094408-mhdx  task-allocator  inv-kimi-deepinfra  for team-20261119-094000-p4vt  done at 2026-11-19 12:00:00, tasks allocated
inv-opus  free, no activity in an open team
```

The line of a round ends with when the round started, in local time (`started YYYY-MM-DD HH:MM`), and for a round that
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

### Questions

A run that ends with questions leaves `questions-<n>.md` in the task's directory and a notification file; with
`[questions] open = "vscode"` the file is also opened in the VS Code window connected to the host. Saving that file with the
`**ANSWERED**` marker on its last line sends the answers as well (the daemon watches the file; a file saved while it was
down is found when it starts). Answers saved in the file while the run that asked is still going — the file is there from the
moment the agent wrote it — are taken up when that run ends with its questions; `agent answer` refuses a task that is still
running and says so. A task that waits for answers is not stopped, paused or resumed: answer it first. Every
control is written to the activity log of each task it reached, with who did it, and `agent status` shows the new status.

VS Code server discovery skips plain files beside builds (such as `cli/servers/lru.json`) and in place of a build's
`remote-cli` directory in either server layout. Directory symlinks are followed; other listing errors are reported.

### Pull requests, reviews and merges

With a `github_token` in `secrets.toml` the daemon runs the autopilot: when an item's branch has a green gate it opens the pull
request, `[review] min_first_full_reviews` investigators from outside the team review it (default `2`). The harness publishes
the passing gated commit directly to the pull request branch without switching worktree branches; agents do not switch
branches in their worktree. All their approvals merge it and start the next item. Set the key in `agents.toml`, for example:

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

A reviewer of a review pass (not of a second opinion, which keeps the rule above) whose provider refuses it is not held
to `[review] replace_after_minutes`. The first refusal pauses its task and the reviewer is tried again in the same
session after the pause (`[replacement] limit_retries`, default `1`), however long the pause is. When the refusal rule
gives its task to another agent — the retries are used up, or a quota's reset is further away than
`[replacement] limit_wait_minutes` — its review goes at once to a free investigator of another provider (for OpenCode
the part of its model before the first `/`, for the other CLIs the CLI itself) that its provider has not refused. When
none is free the review waits for the first that is free or for the refused reviewer's return, and you are told once,
with the provider and the time; a restart of the daemon does not tell it again. You are also told once when a refused
reviewer is replaced. A reviewer that is quiet for another reason is replaced after `replace_after_minutes` as before.

A merge GitHub refuses because the pull request conflicts with its base does not reach you at once: the lead of the round
is sent to merge the base into its branch (at most `[merge] conflict_passes` times per round, default 3). Its change is
gated and pushed, and the merge is tried again; a change to files the task does not allow is reviewed again first. When
the passes are used up, the daemon asks an investigator once for that item and pull request head. A completed answer
gives the lead one extra pass with the proposed resolution. If the request or that pass fails, the lead asks you once
in its questions file. A new head may receive a new investigation. Set `[merge] investigator_pass = false` to keep
the direct human fallback. With `conflict_passes = 0`, you are told at once regardless of `investigator_pass`. The
`merge_conflict_investigated` log event records the item and request ID. A conflict pass that fails during publication does not consume
a pass from the budget; successful publication accounts for the pass once, surviving daemon restarts. Exhausted
split rounds whose resolutions were never published resume merge attempts automatically once gated work is ready.

When a branch cannot be published to GitHub (push failure or pull request error), the harness retries with backoff below
`[publish] tries` (default 3), tells the lead once when the limit is reached so it can fix the issue, and
asks you once in a questions file if publication still fails, with the normal notification and history entry.
Both ordinary and conflict publication respect backoff and wait for a running lead or an existing question.
A retry answer starts another paid run of the lead; a stop answer stops the task. After a daemon restart, a saved
answer is adopted even if the interruption happened before question registration. Old states and histories still
load, including complete question identities in the former flat format; incomplete markers never authorize an
unrelated answer. Set `tries = 0` to be asked at once.

`agent run --no-merge` stops before the merge and tells you; after `[review] fix_passes` rounds of "changes
requested" a question reaches you like any other. Answering grants another `[review] fix_passes` allowance.
The notice names the allowance and open blocking findings. How it works, and its limits, are in
[`docs/architecture.md`](docs/architecture.md) ("Pull requests, reviews and the merge").

The lead writes its contribution to `tasks/<lead>/description.md` under `AGENTS_HOME`, outside the repository.
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

Each posted review starts with the investigator, CLI and configured model, a green **APPROVED** (`✅✅`) or
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

The first run of the lead after a verdict with findings gets the full text of every open finding, with its number, its
reviewer and its level, and the same text is written to `fix-pass-<n>-findings.md` in the lead's task directory; the line
that says the pass is still open and the request to correct a response name that file, so a lead that has lost the text
reads it again instead of asking you. A lead's response is judged only when it was given in a run that started at or
after the verdict, never a message from before the review. Before a fix pass the harness switches the task's worktree to the item's branch when the tree is clean (no change to a tracked file, nothing staged), and otherwise tells the lead to commit its changes and switch itself. An item's pull request is always pushed from the tip of the item's own branch, never from the branch of another item that the worktree has out (work the lead did on a detached `HEAD` or on a branch of its own, going on from the item, is still the item's work); when the worktree has another branch out and no gate has passed at that tip, nothing is pushed and the lead is sent back to switch to the branch and run `agent gate`.

An idle open round's line in `agent status` and `agent status <round>` names what it waits for: a harness gate,
a review, a free independent investigator (for a review, a second opinion or the replacement of a reviewer that cannot
work, when none is free), a second opinion that an investigator is giving, the human's answer, the lead's correction,
or the queued run of its task to start. A round waits for that run while a task of it is queued — the lead's, the
one of the member the item or part is given to, or one that worked in the round — whether the daemon has not started it
yet or holds it for a free agent or a limit of runs: the run goes on with the round, so it is no defect, and the round
opened right after a merge of a part writes no error. A task that waits for a message, an assignment or a merge has no
run to come. `nothing: this is a defect` means the harness has lost its next action, as when every task of the round
waits; it logs one `daemon_task_failed` error for that round with `operation=round_wait`. Every other line also names who
is obliged and since when the round last went idle (`the lead's correction, owed by the lead since 2026-11-22 03:12`):
the lead and the reviewers are told what is expected of them in their next prompt or a message that wakes them, you in a
question or a notification, and the harness's own work (a gate, a merge, a queued run, a free investigator) needs no telling.
A test lists every state a round can wait in with its owner and fails when one is added without.

When a task of the round (a reviewer, a member, a lead or a solo task) is paused because its provider refused its run,
the line names the agent and the time it returns, in the local time zone, for example `waits for the provider of inv-gemini-argon until 10:07`
(with the date, `2026-11-27 10:07`, and never an offset, when the return is more than a day away);
it never says the round waits for an answer of the human unless a question is open; a pause the human asked for with `agent pause`
still says the human. When the only thing the round waits for is a task you stopped with `agent stop`, the line says
what you owe, `waits for you to resume task-20261125-161204-67e1 (stopped at 17:12)` (with the date when the stop is more
than a day away): `agent resume <task>`, not an answer. A task that failed, or a question that is open, is still the human's as before.

When another pull request of the same team already contains an item's deliverable (code, documents, images or
anything else), the coordinator can report it instead of creating an empty pull request. It writes the standalone
line `DELIVERED: #N` before its final `STATUS: DONE`. The harness checks that the report identifies one current
item, the PR belongs to another round of this team and uses its recorded branch, the member's worktree is clean,
and the item has no commits outside the published PR or base branch. For an approved split part, the coordinator
can likewise deliver the part with `DELIVERED: #N` when an external or already merged PR of the same repository
delivers the task's or part's branch, without running the round's agent or reviewers. Git or GitHub failures, unmerged or closed PRs,
or foreign repositories and branches refuse the claim and the coordinator receives a correction with the reason in its
next prompt; the item stays in progress.

An accepted open claim says `delivered by PR #N, waits for its merge` in `agent status` and
`agent status <round>`. It creates no separate PR and keeps waiting across daemon restarts. An already merged PR
completes it immediately; after the merge the line says `done, delivered by PR #N`.
If the daemon observes the PR closed without merging, the old round says
`closed, abandoned, delivery PR #N closed without merging`. The coordinator receives the reason on the bus, and
the member resumes the same item on its branch in a fresh round whose status says `open`. The item then gets its
own PR on a separate publication branch through the usual gate, review and merge process, even while the old PR
stays closed. This return survives a daemon restart at either step.
Reopening a PR before the daemon observes its closure preserves the claim; reopening it after observation never
restores that claim. A stopped team resumes this recovery when its member is resumed.

A step of a round that GitHub or git fails does not stop the daemon: the round says `waits for GitHub: refresh the
description of the pull requests, tried 4 times, next try 2026-11-21 01:07` and the step is tried again by itself, after 30
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
is replaced too; runs its provider refused (a rate limit, a used-up quota) do not end the silence, so a reviewer
refused run after run is replaced that long after the review was asked. Replaced queued, waiting or quota-paused tasks are stopped and their scheduled wake is removed,
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

The history records `second_opinion_asked`, `finding_confirmed`, `finding_not_confirmed` and `second_opinion_ended`
(replacement and exhaustion), and each request names its selected investigator.

### Checks before the review

Before the review the daemon checks the round's own diff and commits through git, with no model, if the repository has a
`[repos.NAME.checks]` table in `agents.toml` ([`docs/configuration.md`](docs/configuration.md)): the lists of path patterns that
say what is a test, what is production code and what judges a round. A round that changes an existing test other than by added
lines, or a protected file, waits for your decision. The questions group findings by kind and judge: one question lists all
paths of that kind with their findings. Answer every question with `allow`, `refuse`, or `allow except <path> <path> ...`.
The exception paths must belong to that question: they are refused and the others allowed. A refusal in any group refuses
the path. `agent answer` and the daemon reject `ok`, blank verdicts, unsupported words and unknown exception paths before
accepting any answers. Other questions still accept `ok`. A pending checks file written by an older daemon keeps its old
per-file questions and verdict rules, including `ok` accepting the recommendation to refuse.

A task file can say what it is in one line: `Short desc: Fix the typo in the README`, among the header lines before the
brief, at most 70 characters ([the format](docs/task-list.md)). For a task given as text use
`agent run --short-desc "Fix the typo" claude-1 "..."`. Every notification the daemon sends about a task starts with
`Repo`, `Time` (the minute in the daemon's local time zone, `Time: 2026-11-21 17:30`) and `Task:` (that description), then the agent, `Task ID:`, the team, the status and the note; when it
concerns an item it names it, number and title, on a line of its own after the fields (the title of the part for a
split part), and it ends with a block `TASK (<task>):` with the first sentences of the task's brief. A task without the
line has a description derived from its brief: its first sentence cut to 70 characters. `agent status` ends the line of
each team with its description in quotes, and `agent status --json` (alone, or with a team or a task) prints the teams as one
JSON array in which each team has its `short_desc`. A notification about an item names the item the notification itself is
about (a notice about a pull request or a failed member, for instance, names that item), and the notification that a whole
team is done names none. `--short-desc` takes one line only.

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

A round that changes code without adding tests, or adds a skipped test, a lint suppression or code that asks whether it
runs under test goes back to the team with the reasons, `[review] check_passes` times (2), and then you are asked. If a check also finds changes that only you may allow, you are asked about those
changes first, before the team starts its next pass. After your answer, the remaining findings go back to the team;
the question itself uses no team pass, and what you allowed is asked again if the team changes that file
afterwards. If the passes are already used up, you are asked about all remaining findings.
A commit that takes lines out of a test its `RED:` commit froze, and is not a `TEST-FIX:` commit of test files only, is a
finding the team cannot repair: the commit is in the branch for good and published history is never rewritten. Such a
finding is never sent back to the team and spends none of its passes: you are asked about it at once, all the commits of
the round in one question that shows each commit's subject, its author and the lines it added to and took out of the
test. What you answer is kept for the item until it is closed, by the commit and the path: `allow` is not asked again
through fix passes, merges of the base, a split into parts or a restart of the daemon, and `refuse` is recorded, sends
nothing back to the team (it cannot undo a commit) and is not asked again either. A new commit that changes a frozen test is a new question, which says what was allowed
before. A file you allowed or refused before this was so (`checks.json` of an earlier version) covers the commits that were in
the round then, and no later one. A `TEST-FIX:` commit that also changes a file that is not a test (by the `test_paths` of the repository) is
no test fix: it is such a finding too. Answer with `allow` or `refuse`; `allow except` names paths and does not apply to
commits.
A file you refused stays refused until the item is closed, and the round goes back to the team until it undoes the change,
but only until the team changes that file again: a commit to a refused file made after the refusal lifts the refusal, and you are
asked about the file again, together with the new commit, as for any change. `allow` starts the review; `refuse` sends the round
back once more and the file is not asked again until another commit changes it. A refused file that nobody has changed since is
sent back and not asked, as before. The decision line of the log (`conclude_checks`) says how many refusals were lifted
(`lifted=<n>`).
**A commit of a frozen test is refused while it can still be undone.** The `git` of every run an agent starts is given a
`commit-msg` hook (through `core.hooksPath` in the run's environment: nothing is written into the repository, and the
repository's own hooks keep running). The hook runs `agent guard-commit`, which asks the rule above while the commit is only
staged: a commit that takes lines out of a test a `RED:` commit of the branch froze, and is not a `TEST-FIX:` commit of test
files only, is not made. The agent is told the rule, which file and which `RED:` commit, and how to make the commit that is
allowed (a `TEST-FIX:` commit of the test files; the rest in a commit of its own). The hook costs no question to you, works
for every CLI that commits through `git`, reads the branch and writes nothing, so a commit that is already made — on the remote
or not — is never touched, and it never tells anyone to rewrite published history. Commits made by a merge, a cherry-pick, a
revert or a rebase are not judged, an amend is judged against the parent of the commit it replaces, and the worktree of a
repository with no `[repos.NAME.checks]` table or with `commit_guard = false`, a directory that is no task's worktree and a
home that cannot be read are not judged (the last one says so). Only the guard's refusal, which has a status of its own
(86), stops a commit; any other status of `agent` lets it through. An amend is recognised from the command line of `git`
(`--amend` and its abbreviations, a value of `-m` is not an option); an alias that hides `--amend` in its definition is
judged as a new commit. `git commit --no-verify` skips the hook; the checks
before the review find the commit then, as before.
**Every commit names the step it serves.** The plan of a task of an item says which requirement each step serves: the
harness numbers the item's requirements when it gives the item to the agent (`R1`, `R2`, ... for the bullets of its
`Expected result:`, `K1`, `K2`, ... for its `Checks:`), and a step carries a `For: R1, K2` line in `.agent/plan.md`
([`docs/plan.md`](docs/plan.md)). A run that ends `STATUS: DONE` with a step that is not given up and names no requirement,
or one the item does not have, is sent back for its plan (counted by `[plan] bounces` like the other corrections). A plan
with no `For:` line that was started before this version reads as before. In the worktree of a task whose plan has `For:` lines
the same `commit-msg` hook refuses a commit whose message has no `Step: N.M` line naming a step of the plan — `RED:`,
`TEST-FIX:` and merge commits too — and tells the agent what to add; the `Step:` line sits in the trailers that end the message (the last paragraph holds trailers only; the scissors line of `git commit -v` and what follows it are not read). The commits the harness itself makes are not checked, and
`git commit --no-verify` skips the check. Git runs no commit hook for a plain cherry-pick, revert or rebase.
It is "Round checks before the review" in [`docs/architecture.md`](docs/architecture.md).

### The gate

A gate is run with `agent gate`, by an agent in its worktree (the human can run it too):

| Command | What it does |
|---|---|
| `agent gate [task]` | runs the repository's gate command (`[repos.NAME] gate`) in the task's worktree and says what it said; the task is the one named, or the task of the worktree the command runs in (also from a subdirectory) — outside a worktree, with no task named, it fails at once |

The gates of all the tasks wait in one queue in the daemon and run one at a time on the machine, in the order they were
asked; the command waits for its turn and then for the gate (at most 16 wait; the next is refused). The command is stopped,
with everything it started, after `gate_timeout_minutes` (default 30, counted from the start of the gate, not from the
wait) and is recorded as failed. The daemon runs the command with `TEST_LOCK_WAIT=1`, so a gate that finds the machine's
test lock held by another test run waits in line for it instead of being refused; that wait is part of the gate's time. Only `tools/test-lock.sh` itself holds the lock, not the
commands it runs, so a process a test leaves behind does not keep it once the gate is over; stopping or pausing the task stops its gate too. The result is recorded with the commit
(`HEAD`) and whether the tree was clean, in `tasks/<task>/gate.json` and the task's activity log, and the full output is
in `tasks/<task>/gate-<n>.log`. The preceding record is kept in `tasks/<task>/gate-previous.json` to disclose a
second-run pass; older task directories without that optional file remain readable. A tree is not clean when a file is changed or untracked, when a merge waits to be committed
or when the gate itself changed the tree or `HEAD`: such a gate may run, but its pass is not a pass of the commit. A run
that waits for its own gate is never cut by the watchdog. The command succeeds for a pass and fails (status 1) for a
failure, a timeout and a refusal. The command says first where the result is recorded: a model CLI cuts a shell command that runs long (the Bash tool of Claude Code has a time limit, in minutes, and a gate may take 30) while the gate goes on, and the agent then reads `tasks/<task>/gate.json`.

A gate is not run again when its answer is already known. When the tree is clean, `HEAD` is the commit of the task's
last gate and the gate command has not changed, a gate that passed there is answered with that pass, and one that failed
there twice in a row is answered with its last failure: the command says the same as that gate did, the new
`gate-<n>.log` holds that gate's output and ends with a line `gate: not run again: ...` that says why and what to do (for
a failure: fix the cause and commit, or, when the failure is not caused by the change, say so in the questions file and
wait for the answer). A gate that the busy test lock still refused (a gate command that does not wait in line exits with 75 and says "another tests are running now") ran no check and does not count: it is neither a failure nor a pass of the commit, it does not reset the count of the failures before it, and the next gate runs. A new commit, a changed tree or a changed command runs the gate again. The daemon logs each such
answer as `checks_decided` with `outcome` `gate_reused`, the `head`, the `gate` it repeats (`passed`, `failed` or
`timed_out`) and the `failures` in a row. The daemon remembers the last gates in memory only: after a restart of the
daemon the first gate of each task runs again. Details: [`docs/architecture.md`](docs/architecture.md), "The gate".

### Compatibility and upgrades

Each version reads what an older one wrote: the state (`state.json`, schema 15 now) is migrated when the daemon loads
it, and the history (`history.db`, schema 35 now) when the daemon opens it for writing — `agent log` reads an older
history as it is. A version that adds a field, a log event or a table migrates forward and keeps every record. The way
back is closed: **an older daemon refuses a state or a history of a newer schema**, so keep a copy of `state.json` and
`history.db` before an upgrade if you may need to return. The CLI and the daemon talk over `agent.sock` with protocol
negotiation (versions 3 to 10 now): a newer CLI falls back to what an older daemon speaks, and loses only the fields the
older protocol lacks; an older daemon refuses a command declared with a newer protocol version with `UnsupportedVersion`,
such as `swap` (protocol 10). The socket API introduces `Command::Swap(SwapRequest { task, agent })` and replies with
`Reply::Applied` or structured `SwapRefusal` failure causes; the full `agent swap` CLI command is delivered in a later item.
A human-selected holder may be of any tier except a worker; workers serve requests and cannot hold solo, coordinator or member tasks.
Existing state files and historical states without reason `swap` read as before; an older daemon cannot read a state
holding reason `swap`. Restart the daemon after installing a new binary, and replace the CLI together with it. [`WhatsNew.md`](WhatsNew.md) says what each version
changes and what you must do (**Action:**); [`docs/state.md`](docs/state.md) and [`docs/history.md`](docs/history.md)
have the formats.

Team tasks can queue investigation requests. When an investigator is free, the daemon selects the least recently
used eligible investigator and creates a new task in the requester's team, detached at the requester's branch tip.
Its prompt asks for instructions and exact commands, forbids pushing, opening pull requests or messaging others,
and ends with `STATUS: DONE` or `STATUS: FAILED`. The daemon saves a completed answer in the request's file,
marks the request done, and sends the whole answer directly to the requester, waking its task. Investigation
work stays local; it is not published. Requests wait while all investigators are busy, and another team's requests
remain for that team. Unknown request kinds are failed once and logged as `investigation_kind_unknown`.

Team state retains the `investigator` task role beside reviewer tasks and records investigation activity; older
state without the role field still loads. History schema 33 accepts investigator runs. Schema 35 adds
`investigation_started`, `investigation_answered` and `investigation_kind_unknown`, preserving existing logs,
round links, search entries and cursors. It also accepts `investigation_requeued` and `investigation_failed`.

An unsuccessful attempt is closed and retried once with a different investigator. If no other configured
investigator is eligible after excluding previous failures and the requester, the request fails immediately
and the requester receives the failure notice. A temporarily busy investigator can still take a later retry. Any unfinished answer stays
in the request file under `## Unfinished answer of <investigator> (<end>)`; it is never sent to the requester
or included in the replacement's prompt. A second failure, a human stop, or expiry of
`[investigations] answer_minutes` (default `30`, allowed `10..=1440`) ends the request and tells the requester
to decide or ask the human in its questions file. The deadline starts when the current investigator task is
created, after worktree setup; queue and setup time do not count. Ended requesters and removed teams fail their requests without a message.
Provider refusals close the attempt instead of pausing, switching agents or retrying the same task; an
investigator remains unavailable until its provider cooldown ends. After restart, live assignments continue,
and requests with no live assigned task return to the queue. Failure decisions are saved before stopping
the investigator and sending the notice; the bus request closes after both succeed. A restart retries
unfinished cleanup or delivery. A crash after sending but before closing may deliver the notice again.

Each team's optional `investigations` records store the request `id`, current `task`, `started` timestamp and
`failed_with` investigator names. The optional `investigation_failures` map retains failure reasons by request
ID so recovery preserves the original decision, including after a cancellation reports a stopped run. Old state
without these fields still loads; empty lists and maps are omitted.

`agentbus inv-add` accepts question, review and conflict requests. Request splits through
`request_investigation`, which requires and validates their item identifiers.

`agentbus inv-add --kind conflict --item <item>` accepts a conflict request for an item and prints the new request id.

For daemon integrations, `agentbus inv-next --task TASK` restricts selection to requests from that task; repeat
`--task` for every live task of the team. Without `--task`, selection remains global. A nonmatching request stays
queued, and the investigator slot limit remains global. The Rust bus wrapper exposes this as `inv_next_for_tasks`;
an empty task set retains the unfiltered behavior of `inv_next`.

## MCP tools available to agents

| Tool | Purpose |
|---|---|
| `list_agents`, `send_message`, `read_messages`, `read_message`, `wait_for_message`, `set_status` | communication (`read_message` reads one long message whole by its number) |
| `reserve`, `release`, `list_reservations` | shared resources |
| `notebook_read`, `notebook_append`, `notebook_write_section` | the repository notebook (progress, decisions and next steps) |
| `finding_add`, `finding_resolve` | add or resolve a note about repository paths |
| `checkpoint` | save a resumable work state; a non-coordinator member saves its own archived task record, not carried into a resume |
| `request_investigation`, `investigation_status`, `record_decision` | investigation requests, results and decisions (team coordinators and members only) |
| `delegate`, `delegation_status`, `delegation_feedback` | worker requests (normal tier; an investigator in a team or on a request may ask too, a worker not); optional `requires` matches against pool worker capabilities |
| `team_status`, `claim_lead`, `handover_lead` | teams |
| `quiz_start`, `quiz_answer`, `quiz_results`, `team_exclude` | the qualification quiz |
| `board_add`, `board_list`, `board_claim`, `board_progress`, `board_done`, `board_release` | the team board (`board_add` with `for=<member>`: only that member may claim the item; `board_progress` records how far the claimed item got) |
| `consult_start`, `consult_reply`, `consult_results` | ask the other normal members with a time limit (e.g. for recommendations); silence is shown as NO ANSWER |

The daemon gives investigation tools only to coordinator and member tasks, including those run by an
investigator-tier agent. Solo tasks, workers, reviewers, investigator tasks and pool worker runs do not see these
three tools; calls to them are refused. `agentbus serve --investigations=off|on` controls this access. Omitting the
option defaults to `on` with the legacy role checks, so an older daemon with a newer bus keeps its previous behavior.
Explicit `on` grants the tools to a team task regardless of its agent tier.

Use `request_investigation` only when an investigator would help. Commit first: question and split investigators
start from your branch tip. Accepted, waiting-room and already-pending replies ask you to end your turn with
`STATUS: SLEEPING`;
the answer arrives as a message, which wakes the requester.
Coordinators and members ask an investigator before writing a technical question for the human, following the
exceptions in their role prompts.
Split and conflict items use `FEATURE.ITEM`: each number is 1–65535, without leading zeros. Invalid items are
refused before queueing; optional item metadata on other kinds does not constrain their decoding.

The optional `kind` argument defaults to `question`. Only the team's coordinator may request `split` or `conflict`,
and both require an `item` argument. A split investigator reads the task-list item and code, then returns a complete
proposal in a JSON block with parts within the configured line limit. The assignment records a task-allocation
activity. The coordinator saves the answer's JSON as its own `split-proposal-<item>.json` and submits it through the
normal split checks; the investigator writes no proposal file for it.

A coordinator can request a conflict investigation with `kind: "conflict"` and `item: "5.1"`. The daemon fetches
the item's open pull request branch and starts the investigator detached at its current remote tip. The investigator
fetches and merges the configured base locally, resolves conflicts and runs affected tests; it never pushes. Its
answer gives the coordinator exact commands, resolved hunks (or complete files when shorter) and the version to set.
An item without an open pull request fails the request and notifies the coordinator. Answers and execution failures
use the same delivery as other investigations. Conflict requests are explicit; exhausted merge passes still ask the
human as before.

Older bus states without these kinds or the optional item still load; legacy question and review kinds retain
their meaning.

### Repository findings

Every role can call `finding_add(kind, paths, text)` to record a short note about repository files.
The kinds are `flaky_test` (a test that fails without a cause in the code), `trap` (misleading code or
documentation), and `environment` (a tool, toolchain or host issue). `paths` is an array of at most ten
repository-relative file or directory paths; `flaky_test` and `trap` require at least one. Paths contain no
spaces or `..` components, have at most 200 characters, and are normalized without `./` or a trailing `/`.
Text has 1–600 characters. A task may have at most ten open findings in each repository.
The result is `ADDED F-<n>`; IDs start at one and increase even after findings are resolved.

Call `finding_resolve(id, reason)` when the cause is fixed or the note is wrong, including a reason of
1–300 characters. Any task may resolve any finding of its repository. The result is `RESOLVED F-<n>`, or
`ALREADY RESOLVED F-<n>` on a repeated resolution. An unknown ID is a not-found error.
Keep progress notes, decisions and remaining work in `notebook_append`.

`agentbus findings --repo <repo> --json` prints a JSON array of open findings, oldest first, with
`id`, `ts`, `author`, `task`, `kind`, `paths` and `text`. Like other bus commands, `--dir` selects an
isolated bus directory; an omitted repository is an invalid-argument error, and storage failures use the
storage error exit class. Addition and resolution publish `finding_added` and `finding_resolved` bus events
to the task's team and log events with those same names.

The append-only journal is `notebooks/<repo>/findings.jsonl` in the bus directory. An addition record has
`op: "add"` and the listing fields; a resolution has `op: "resolve"`, `id`, `ts`, `author`, `task` and
`reason`. A missing journal means no findings. Existing bus state and notebooks are unchanged and require
no migration. The pure run selector ranks exact paths before ancestor paths, then siblings with a parent at least
three components deep. It excludes findings from the same task, already shown IDs and findings older than
`[findings] keep_days` (default `30`, range `0–365`; `0` disables selection). Ties use the newest date, then the
highest numeric ID. `[findings] max_shown` (default `8`, range `1–20`) limits the count; `max_chars` (default `3000`,
range `500–10000`) limits the sum of the texts' character lengths by removing findings from the end. Repository-wide
environment findings match every run. Run paths may contain spaces or exceed the bus finding-input path limit;
backtick path extraction still excludes tokens containing whitespace. Finding text line breaks become spaces in the
rendered section, so each finding occupies one line.

### Durable findings delivery

At the start of each new or resumed run, findings from other tasks on files the run touches are shown once in a dedicated
section of the prompt. These findings help the agent avoid repeating mistakes or rediscovering issues that other tasks
have already found. The agent's own findings and findings that are already resolved are excluded from this list.

Run paths combine committed and uncommitted changes against the merge base with `origin/main`, including untracked
and deleted files, with existing repository paths named in backticks in the item's text. Investigators use the branch
of the task they answer; reviewers use the branch under review. Workers use the files assigned to their step.

If the bus query or delivery bookkeeping fails, the daemon logs `findings_unavailable` and starts the run without the
section. Selected IDs are saved before the run starts; `findings_shown` records their IDs and relevance scores.

The findings are selected according to the `[findings]` configuration section: `keep_days` controls which findings are
fresh enough to be considered (default 30 days, range 0-365 days), `max_shown` limits how many findings are displayed
(default 8, range 1-20), and `max_chars` limits the total character count of all findings text (default 3000,
range 500-10000). The selection algorithm prioritizes path relevance, then recency and numeric finding ID.

The selected findings are shown only once per task, even across daemon restarts. The harness records which findings
were shown in a task file (`findings-shown.json`) so that they won't be repeated in subsequent runs of the same task.
This ensures that agents get helpful context at the start of their work without being repeatedly reminded of the same
information.

### Requests to workers

**A request to a worker names its work.** A run whose role is in `[plan] roles` (a solo agent, a member or a
coordinator, the investigator among them when it leads or works as one) sends the four new arguments beside `goal`,
`workdir`, `steps` and `returns`: `for`, the ID of a step of its plan that is still to do or in hand; `files`, the paths
the worker may read and, when it is to produce something, write (relative to the repository, no globs); `check`, a
command or `exists PATH` that shows the result is right — never `reviewer`, never the repository's gate (`[plan]
check_refuse`); and `do`, what to do in the requester's words. The bus asks the harness's `agent _delegation-check` at
the moment of the request, which reads `.agent/plan.md` of the requester's worktree: a request that lacks one of the
four, names a step the plan does not have or that is done or given up, a path outside the repository, or a refused
check is refused — a check is one command and nothing after it: one that goes on as a sentence (`grep -n x README.md
shows the line`: a word such as `the`, `and`, `shows`, `gives` or `exits`, a word that ends with a comma, or `…`,
unquoted after the command's name) is a refused check too, with the word in the cause; a word that belongs to the
command is quoted (`grep -q 'the' README.md`). Each is refused with a cause of its own (`NoSuchStep`, `StepClosed`, `PathOutside`, `CheckRefused`, `PlanUnreadable`,
`MissingFor`, `MissingFiles`, `MissingCheck`, `MissingDo`, `CheckFailed`), nothing is queued and no worker starts. Such a request is also refused at once (`NoFreeWorker`) when every worker
of the installation pool is running, reserved, awaiting settlement or promised to an earlier request. Legacy teams
with worker members retain their own admission path. The requester does a refused step itself; a higher tier never serves it. The worker's request then quotes the step in full, then `Request files:`, `Request check:`
and `Do:`, and the bus records the step's ID with the request (`step` in the bus state; a request recorded before has
none and is served as before). An agent without a plan — a worker, a reviewer, an investigator answering an
investigation request, a run of a home that keeps no plan, or a run whose worktree has no `.agent/plan.md` yet — sends
the old form, which is for look-ups only and waits
in the queue as before.

**A worker's result is checked before it reaches your branch.** The bus keeps, next to the step, the paths the request
names (`files`), its check (`check`) and what to do (`doing`) — a request that names no path and no check is a look-up, in
the old form, and its result goes to the requester as before. When a worker's run ends `STATUS: DONE`, the daemon runs the
request's check in the worker's worktree (a command, with `[plan] check_seconds` as its limit, or `exists PATH`; never the
repository's gate), decides what the result is worth, and only when it is good brings the worker's commit to the requester's
branch: it takes a snapshot of the requester's worktree first, and the requester's uncommitted and untracked files are left
as they were. The commit that is brought over is what the worker committed since its request was given to it, not
everything its branch holds. The worker's prompt (`prompts/harness/team/delegation.md`) therefore tells it to commit the files a request names
in its own worktree before it ends, and to do the job there even when the requester's worktree seems to have it already;
work it leaves uncommitted is not brought over and the request fails. That rule holds over the request: a worker
commits the files also when the request says not to, and says so in its notes; the texts of a member, a coordinator and a
solo agent tell them never to ask for uncommitted edits. A commit that conflicts, or one that would write over a file the requester changed or has, is
not applied: the requester is told, with the files. A result that is wrong — the check failed or ran past its limit, the worker changed a
file the request did not name, it left a file the request names uncommitted, it committed nothing, its run lasted past
`[workers] seconds`, or the daemon cannot read what it left (the commit the worker started from, its commits) — is returned
to the requester with the reasons, and the request is answered as `failed`; the requester reads of the one result when the
step had one try, and of every result, each named by its try, when an earlier worker's result was wrong too; what cannot be judged is never brought over. The
check runs beside the daemon, which goes on with the other tasks meanwhile; a worker whose result is not settled yet (its file is
still in its task directory) is busy and is not given the next request, and only a request the bus still has running for that
worker's task is settled from a file. A good result is told to the requester with the step,
the commit, the files and the end of the check's output, and the step is recorded `confirmed` in the history with the
worker's name. Each stage is written to `tasks/<worker task>/worker-request-<n>.json` before the next starts, so that a
daemon that stops in between goes on from the stage the file names: the check runs once and a commit is brought over once.
A request the bus no longer has has its file removed. **A file that cannot be used is the fault of its request only.** A
`worker-request-<n>.json` that is cut short, has a field this version does not know, holds another request than its name says, cannot be read, or a stage that cannot
be written: the daemon logs a `worker_request_damaged` warning that names the file (and where it is kept), moves the file aside
as `worker-request-<n>.json.damaged` (kept, never deleted, never met again — also after a restart of the daemon), settles
the request as a wrong result — the bus request ends `failed` and the requester is told why and that the worker's work
cannot be judged — and goes on with the other requests, the other teams and the owner loop. Look at the kept file and the
log line to learn what happened to the worker's work: nothing was brought over.
**A wrong result is tried once elsewhere.** When a new local request ends `STATUS: FAILED` or `STATUS: QUESTIONS`, or its `STATUS: DONE` result is wrong (the check failed or
ran past its limit, a file outside `files` was changed, nothing was committed, the run lasted past `[workers] seconds`) and
`[workers] tries` is above 1, the request goes once more to a free worker of another provider (of the same provider when no other is
free): its prompt says who served the request before and why that result did not serve, and the requester is told that the
request went again. A second wrong result returns the step to the requester with the reasons of both results, and no third
worker starts; `tries = 1` never gives a request to a second worker. What the first worker committed is not brought over. The
try is kept in `tasks/<worker task>/worker-try-<n>.json`, and a daemon that stops while the request waits for the second
worker goes on from the file of the first result. A worker whose result is not settled is busy; a request the daemon has
answered keeps it busy until its workspace is released. The lines of `agent status` for the tries and the pull request's description of the returned steps come with a later version.

**Owned settlement.** New local requests bind each execution to its worker, task and run. The final requester
notice names the serving worker and is published once, even when settlement is replayed after a daemon restart.
Late results are dropped and logged. Failed preparation leaves a request dispatchable and never removes a
running worker's workspace. Receipt recovery reads only newly appended messages after its first scan.
Recovery log events `worker_request_requeued` and `worker_request_returned` are also stored in daemon history.

Before a wrong-result retry, the daemon retains a snapshot of the failed
worker's work; a pool retry starts in a fresh workspace from the original requester's commit. The journal lives
in the requester's task directory and preserves both workspace identities and the observed ending.
Existing open requests without the opt-in marker keep their previous handling; loading never opts them in.
A dead CLI, missing status, provider refusal or quota failure returns the request to the queue with its worker
worktree and snapshot. Startup recovers queued and running requests the same way. If the daemon stops after recording a worker's ending but before saving its pending result, startup rebuilds that result from the saved run outcome before dispatch. A fresh execution reuses that
worktree without consuming `[workers] tries`: provider failures prefer another provider, with same-provider
fallback; other interruptions take the first compatible free worker. With none free the request stays queued.
After three no-result returns, the next interruption returns the step to the requester with all causes.
The original workspace branch stays fixed even when the serving worker changes. No new configuration key is needed.

**A request nobody takes is returned.** A request that stays queued because no worker of the pool is free is noted at the
daemon's first look at it (`tasks/<requester task>/worker-wait-<n>.json`; `agent status` shows `waiting since 21:10 (12 min)`).
After `[workers] wait_seconds` (default `600`; `0` waits for ever) the daemon answers it `failed` and tells the requester, in the
request's summary, to do the step itself; the log event is `worker_wait_returned`.
See the [durable format](docs/state.md) and [protocol](docs/architecture.md#request-recovery-protocol).

**Every end of a request is a record of the history.** The daemon writes one `worker_request_ended` record when the result of a
request is settled (`outcome` `report`, `brought`, `conflict`, `again` or `returned`) and when an interrupted request goes
back to the queue or is returned for its interruptions (`worker_request_requeued`, `worker_request_returned`); a request
read again after a restart of the daemon is not recorded twice. The record belongs to the requester's team and round and has
`request`, `task`, `requester`, `step`, `worker`, `worker_task`, `try`, `outcome`, `started`, `seconds` and `cost_usd` (empty
when the worker's CLI did not tell it); `agent log` shows it with the others. Opening an older history upgrades its schema to
29 while preserving its records; older daemons refuse the upgraded history.

**When to give a step to a worker.** The texts of the member, the coordinator and the solo agent, and the bus's
instructions of every run that may call `delegate` (an investigator answering an investigation request among them),
say it: a step with one result, named files and a check a machine runs, the reading and listing of many files, a
mechanical edit by a stated rule — never a decision, a design, a step whose check is `reviewer` or anything that needs
the requester's uncommitted work changed; a refused or failed request is done by the requester itself. A plan marks
such a step with the line `By: worker` in its text (`docs/plan.md`).

**The requests are seen.** `agent status <task>` (and `--task`) shows the requests for workers of each task under its
plan, as the bus has them (`agentbus helper-dump --task <task>`, JSON):

```text
    request H3  step 1.2  worker glm-1  done
    request H4  no step  no worker yet  queued
```

When the bus cannot be read, a line says why (`requests unknown (…)`); a home without `agents.toml` shows none. The
description of a pull request gets the harness's section `## Steps done by a worker`, with each request of the team's
tasks that is done and names a step: `- step 1.2 of claude-1's plan — done by glm-1 (H3)`. A request that came back to its
requester (`failed`) and names a step gets a row of the section `## Steps returned to their requesters`, after the first
one: `- step 1.3 of claude-1's plan — returned to claude-1 (H5): Result: the file is not written.`; the reason is the first
line of the request's summary, cut to 120 characters, or `no reason given`. A lead cannot write either section itself.

### An agent in two teams

An agent may be a member of several open teams at once, with a task, a worktree and a
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

### `agentbus team-replace`

`agentbus team-replace [--dir D] --team ID --from OLD --to NEW` replaces one team member with another in
one atomic operation: the membership moves from `--from` to `--to`, every board item `--from` claimed becomes
`--to`'s, and the unread messages of `--from`'s team session move to `--to`. Messages already read keep their original recipient.
The team reads a `member_replaced` event naming `from` and `to`; the command prints `REPLACED from to` on
stdout. Missing or finished teams and missing members exit with status 3, an existing replacement member
with status 6, and identical or empty agent names with status 2. The `member_replaced` log record has `team`, `from` and `to` fields.
A failed replacement restores membership, unread messages and receipt offsets together. Interrupted replacements
are rolled back under the bus lock before the next state load; internal `replacement-undo` backups do not change
the state or message formats. The replacement lead can read the notice, and the moved member keeps its state.

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
