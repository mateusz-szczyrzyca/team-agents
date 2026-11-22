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
[`docs/plan.md`](docs/plan.md) (the form and verification of a task's plan of steps), [`docs/tdd.md`](docs/tdd.md).
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
`commit-msg` hook that runs `agent guard-commit` (see "What the daemon does", the checks before a review) and then the
repository's own `commit-msg`, and links the repository's other hooks (its `hooks` directory, or the directory of its own
`core.hooksPath`), so a hook of the repository still runs in an agent's run, and one that fails still fails the commit. An
agent that wants to skip the hook can use `git commit --no-verify`; the checks before the review find the commit then.
To turn the early refusal off for a repository, set `commit_guard = false` under its `[repos.NAME.checks]` table (on by
default; the checks before a review stay as they are): `docs/configuration.md` has the table. The hooks directory acts for the
repository it was made for only: a commit that the agent makes in another repository during a run runs that repository's own
hooks, and the hooks path of an outer run (a `cargo test` started by an agent) is not taken for a repository's own.

### Monitoring and response

- Ship the daemon's log ([`docs/log.md`](docs/log.md)) somewhere you look at, and alert on warnings and errors.
  `agent log` shows what a round, a team or a task did, with every prompt and step. Its text shows times in the zone of
this machine with its offset, as `date` does; `agent log --utc` shows them in UTC, as the files hold them, and `--json` is
always UTC. A `--since` or `--until` without a zone is read in the zone of this machine.
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
agent status [--running] [--task] the teams the daemon knows, their rounds and a count of their tasks; --task adds the tasks
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

An agent has one run at a time, over all its teams and solo tasks (`[limits] runs_per_agent`, default `1`; the agents
of `[tiers] workers` use `runs_per_worker`). When an agent of a task that is queued already runs another task, the
queued task waits — it is not failed — and `agent status` says `queued, waits for claude-1, running in team-…`. It starts
when that run ends. `0` restores the old behaviour, where an agent may run as many tasks at once as are queued.

**The plan of a task.** A solo agent, a member and a coordinator keep the plan of their task in `.agent/plan.md` of
their worktree: its stages and steps, each marked to do, in hand, done or given up ([`docs/plan.md`](docs/plan.md)).
The daemon reads the file at the end of every run and keeps each new version in the history. A run of a role in
`[plan] roles` that ends `STATUS: DONE` while its plan is missing, does not parse, has a step still to do or in hand,
lost a step of its last version, or has a done step whose check failed (`mismatch`, sent back with the step, its check,
the reason and up to 40 trailing lines of output) goes back to the same agent with the reason, at most `[plan] bounces`
times per task (default 3; `0` only records); after that you are asked once, in the task's questions file, whether to go
on as it is or send it back once more. A step whose check matches `[plan] check_refuse` stays `declared`, and the task's
next run is told once in its prompt that the gate is never a step's check. Tasks created before this version are never
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
recorded judgement. `agent status <task>` and `agent status <round>` show these states beside the steps. Judgements
are retained in history schema 17, which automatically upgrades older histories; keep a backup before upgrading if
you need to return to an older daemon.


A run ending without a `STATUS:` line resumes automatically in the same session with a short prompt
(`prompts/harness/run/resume-no-status.md`), up to twice in a row. After that the session itself is taken to have gone
bad, and the task is started once in a new session of the same agent with a briefing built from facts — its prompt, the
commits of its branch, the files it left uncommitted (kept in a snapshot first), its plan of steps and the messages of its
team it had not read (`[replacement] new_sessions = 1`; `0` turns it off). The same happens after the watchdog's last
strike for a hung run and after the last turn-limit resume. When the new session goes bad too, the task is given — with
its worktree, its branch, its items and a briefing — to the first agent of `[replacement] agents` that is not in its team,
not in a run, and of the same or a higher place than the agent it replaces (`[replacement] replacements = 1`; `0` takes
none over; a takeover is not counted among the new sessions). On the team's bus the task keeps the name of the agent it
was first given to. With no such agent — also when no `agents` are listed — a member's task stops and the team's lead
gets its work in a message on the bus, with a briefing of what the member's runs left (`[replacement] mode = "lead"`
asks for that at once). With `mode = "off"`, and for a solo task or a coordinator's with nobody to take it, the task is
marked failed and the coordinator or you are notified, as before. When the agent that took a task over goes bad too, or
a lead given a member's work does, you are asked once, in the task's next questions file: `a` starts it once more in a
new session; `b`, `ok` and any other answer stop it. Each hand-over is logged as `task_handed_over`.

**Returns of a lead.** When the lead of a team sends the work of a member back instead of taking it, it ends its final
message with a line `RETURNED: <item>` beside `STATUS:` (its role text says so). The daemon counts the line only where
its own facts agree: the member's last run on the item ended `DONE` and the member is started again for it. The count is
kept with the item, survives a restart, and `agent status` shows it on the member's line as `returned 3 of 5`
(`[replacement] rejections = 5`; `0` has no limit) and on the round's line when one more return reaches the limit. A
lead that writes no such line is never counted. In this version the count is shown; it does not yet replace the agent.
`agent status` shows such a task as `in a new session`, and as `, for AGENT` when another agent took it over from AGENT. Automatic retries apply only to resumable tasks with a session (solo tasks,
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
across daemon restarts; a later failed run can notify again. An item does not stand behind a member that the human stopped,
or that is no longer in the team: when the lead ends `STATUS: DONE` with a passed gate, the item is published with the
lead's branch.

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
item number is a submission. Existing configurations and split plans remain readable;
history migration 15 preserves previous rows while admitting decision records. Requested tiers are retained;
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
survive restarts. All status views and show requests use socket protocol negotiation: the CLI probes `6`, then
falls back to `5`, `4` and `3`
when an older daemon refuses or closes a probe. Version `6` carries executor assignments and capability waits;
versions `3`–`5` omit both so older strict clients remain compatible. Version `5` also carries plan summaries,
which versions `3` and `4` omit. Split acknowledgements use protocol `4`, with a fallback to `3`; version `3`
also omits split fields. Negotiation uses read-only status requests and caches its choice on the connection.
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
source changes before they are reported as unreadable. SQLite creates no files beside the original database. Summary timestamps use the machine's local time zone at each moment (`YYYY-MM-DD HH:MM:SS +HH:MM`, or `UTC` when the zone cannot be found), as `agent log` does.
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

A line of a row names the task, the agent, the run and what changed (`task-… (claude-1) …: queued ← waiting`); other events
print their fields as `key=value`. With `--task`, `--agent`, `--run`, or with `-f` and a round or a team, `agent log` also
prints the steps of the runs it covers — the agent's messages, commands and tool calls, and their results — between the
rows, as one text of the newest `-n` lines (default `100`); `-f` goes on with the new rows and steps as they are recorded,
across runs and daemon restarts, and says what the round waits for when a run ends. `--no-steps` leaves the steps out;
`--json` is unchanged. See [`docs/log.md`](docs/log.md).

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
| `agent status` | the short view: every team with its rounds, a line for each of its live tasks with what it does, and a count of all its tasks, by what they do (`tasks: 1 running, 1 waiting, 1 with questions; questions number 1 wait for answers (claude-1, task-…)`). The live tasks are the team's coordinator, members and workers (a solo task too), whatever their status, and the reviewers whose review is not over; a reviewer that gave its report or was stopped is only counted. The plans of the tasks and the parts of a split are not listed; a question that waits for the human is named with its number |
| `agent status --task` | as `agent status`, with the lines of all the tasks, their plans and the parts of a split, as `agent status` printed them before 0.58.0. Refused with a team, a task or a round, which already show their tasks, and with `--investigators` |
| `agent status --running` | only the teams that have a round that goes on, with those rounds: what is being worked on now and who works on it. The short view as `agent status`; the parts of an item whose every part merged are left out, its work being over |
| `agent status --running --task` | as `agent status --running`, with all the tasks, their plans and the parts |
| `agent status <team>`, `<task>`, `<round>` | the team or task with its tasks, or the round as before |
| `agent status --investigators` | what each configured investigator is doing: its last two activities in the teams that are not closed (no versions line) |

Every list of `agent status` starts with the versions of the four crates (`agent 0.58.0 · agent-core 0.58.0 · agent-runtime 0.58.0 · agent-web 0.58.0`). The daemon tells its versions with the answer to a status request; when they differ from the command's, a second line says the daemon's versions and that they differ, and a daemon that tells none is shown as `daemon: version unknown`. `agent status --json` stays the bare array of teams, byte for byte as before, with and without `--task`: it carries no versions in this version. Adding them needs a wrapper object, which would break scripts that read the array, so that is left to a later item.

The short view of a team, as `agent status` and `agent status --running` print it: the team, its rounds, its live tasks
and the count. A task that runs says when it was last active; a reviewer says what it does or did (`working`, or when
and why its review stopped) instead of the status of its task:

```
team-20261111-153000-t5m8  team-agents  autopilot  1 pending, 1 in progress, 0 in review, 0 approved, 0 done
  round-20261111-153001-q1w2  item 1.1  open, only local  started 2026-11-11 19:15 +00:00
  task-20261111-153001-k3x9  coordinator  inv-opus   running, last activity 2026-11-11 19:20:11 +00:00
  task-20261111-153002-k3x9  member       claude-1   questions, questions number 2 wait for answers
  task-20261111-153003-k3x9  worker       glm-1      done
  task-20261111-153004-k3x9  reviewer     inv-codex  working, last activity 2026-11-11 19:19:58 +00:00
  tasks: 2 running, 1 waiting, 1 with questions, 2 done; questions number 2 wait for answers (claude-1, task-20261111-153002-k3x9)
```

The count has two tasks more than the lines: reviewers whose review is over (one waits for its next review, one is
done). A reviewer is listed while its task runs, and otherwise until its task is done or stopped or its last review
ended with its report, by your `agent stop`, or as a second opinion that was abandoned; a review that failed stays
listed, since `agent resume` can start it again. `--task` lists every task, and under each its plan (`[x] 1.1 …`), and
the parts of a split.

Every view of `agent status` prints the same state for a part of a split, whether its round is open or closed:
`done` when a round of the item merged the part's pull request, `in progress` while its round is open, `to do` when it
is next, and `waiting for 1.1a` (and the other parts it depends on) until they are over. A part whose round was closed
without a merge is not `done`. In `agent status --running`, an item whose every part merged shows no part line.

`agent status` ends the line of a task that runs with `last activity YYYY-MM-DD HH:MM:SS +HH:MM`: the local time the output of its run last
changed (the file `tasks/<task>/runs/<run>/out.jsonl`), which is how to tell a task that works from one that hangs. Every time `agent status` prints is in the zone of this machine with the offset of that moment, as `agent log` writes it; `agent status --json` stays in UTC.

The line of a reviewer says what it did or does, from the record of the team's investigators, instead of the one word of
its status (a reviewer that waits between its reviews said only `waiting`):

| Says | Means |
|---|---|
| `working`, with `last activity YYYY-MM-DD HH:MM:SS +HH:MM` while its run writes | a review goes, or one is queued for it |
| `done, 1st review finished 2026-11-19 12:08:17 +01:00` | it gave the report the harness asked for; the ordinal counts the reviews it gave for the round (the first review, the verifications and the second opinions alike; a review that ended without its report is not counted) |
| `stopped at 2026-11-19 12:10:00 +01:00, reason: the run showed no activity for too long` | the task was stopped or failed; the reason is the one the daemon knows (`the human stopped it`, `the run ended without a status`, and so on) |

A reviewer task without recorded activity shows the word of its status as before. The lines of the coordinator, the
members and the workers do not change.

`agent status --investigators` shows every investigator of `[investigators.NAME]` in `agents.toml`, in the order of
their names, with the last two of its activities in the teams that are not closed, the newest first: the task, the kind of
the activity (`team-member`, `reviewer`, `investigator`, `task-allocator`), the investigator, the team, and what it did:
`done at 2026-11-19 12:15:15 +01:00, 1st review finished`, `working, 2nd review`, `done at 2026-11-19 12:00:00 +01:00, tasks allocated`, or, for one that
ended without its report, `stopped at 2026-11-19 12:10:00 +01:00, reason: …` as on a reviewer's line. A team counts as open while it has a
round that goes on. An investigator with no activity in an open team is listed as `free, no activity in an open team`:

```text
task-20261119-094409-saal  reviewer  inv-gemini-argon  for team-20261119-094000-saal  working, 2nd review
task-20261119-094408-mhdx  task-allocator  inv-kimi-deepinfra  for team-20261119-094000-p4vt  done at 2026-11-19 12:00:00 +01:00, tasks allocated
inv-opus  free, no activity in an open team
```

The line of a round ends with when the round started, in local time (`started YYYY-MM-DD HH:MM +HH:MM`), and for a round that
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

A merge GitHub refuses because the pull request conflicts with its base does not reach you at once: the lead of the round
is sent to merge the base into its branch (at most `[merge] conflict_passes` times per round, default 3). Its change is
gated and pushed, and the merge is tried again; a change to files the task does not allow is reviewed again first. When
the passes are used up, or the lead gives no result, the lead asks you once in its questions file. Set
`conflict_passes = 0` to be told at once, as before. A conflict pass that fails during publication does not consume
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

The first run of the lead after a verdict with findings gets the full text of every open finding, with its number, its
reviewer and its level, and the same text is written to `fix-pass-<n>-findings.md` in the lead's task directory; the line
that says the pass is still open and the request to correct a response name that file, so a lead that has lost the text
reads it again instead of asking you. A lead's response is judged only when it was given in a run that started at or
after the verdict, never a message from before the review.

An idle open round's line in `agent status` and `agent status <round>` names what it waits for: a harness gate,
a review, a free independent investigator (for a review, a second opinion or the replacement of a reviewer that cannot
work, when none is free), a second opinion that an investigator is giving, the human's answer, the lead's correction,
or the queued run of its task to start. A round waits for that run while a task of it is queued — the lead's, the
one of the member the item or part is given to, or one that worked in the round — whether the daemon has not started it
yet or holds it for a free agent or a limit of runs: the run goes on with the round, so it is no defect, and the round
opened right after a merge of a part writes no error. A task that waits for a message, an assignment or a merge has no
run to come. `nothing: this is a defect` means the harness has lost its next action, as when every task of the round
waits; it logs one `daemon_task_failed` error for that round with `operation=round_wait`. Every other line also names who
is obliged and since when the round last went idle (`the lead's correction, owed by the lead since 2026-11-22 03:12 +01:00`):
the lead and the reviewers are told what is expected of them in their next prompt or a message that wakes them, you in a
question or a notification, and the harness's own work (a gate, a merge, a queued run, a free investigator) needs no telling.
A test lists every state a round can wait in with its owner and fails when one is added without.

When another pull request of the same team already contains an item's deliverable (code, documents, images or
anything else), the coordinator can report it instead of creating an empty pull request. It writes the standalone
line `DELIVERED: #N` before its final `STATUS: DONE`. The harness checks that the report identifies one current
item, the PR belongs to another round of this team and uses its recorded branch, the member's worktree is clean,
and the item has no commits outside the published PR or base branch. Git or GitHub failures refuse the claim and
the coordinator receives a correction with the reason in its next prompt; the item stays in progress.

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

State schema 14 remains readable, including open claims written by 0.34.0. The optional
`delivery_closed_at` field retains the observed closure on the abandoned round; no migration is needed for those
states. Older binaries cannot read states containing the new field: keep a state backup before upgrading.
History schema 14 adds the `delivery_closed` warning and migrates existing logs, search entries and cursors.
Keep a backup of `history.db` before upgrading too: older agents cannot read history schema 14.

A step of a round that GitHub or git fails does not stop the daemon: the round says `waits for GitHub: refresh the
description of the pull requests, tried 4 times, next try 2026-11-21 01:07 +01:00` and the step is tried again by itself, after 30
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

A task file can say what it is in one line: `Short desc: Fix the typo in the README`, among the header lines before the
brief, at most 70 characters ([the format](docs/task-list.md)). For a task given as text use
`agent run --short-desc "Fix the typo" claude-1 "..."`. Every notification the daemon sends about a task starts with
`Repo`, `Time` (the minute in the daemon's local time zone with its offset, `Time: 2026-11-21 17:30 +01:00`) and `Task:` (that description), then the agent, `Task ID:`, the team, the status and the note; when it
concerns an item it names it, number and title, on a line of its own after the fields (the title of the part for a
split part), and it ends with a block `TASK (<task>):` with the first sentences of the task's brief. A task without the
line has a description derived from its brief: its first sentence cut to 70 characters. `agent status` ends the line of
each team with its description in quotes, and `agent status --json` (alone, or with a team or a task) prints the teams as one
JSON array in which each team has its `short_desc`. A notification about an item names the item the notification itself is
about (a notice about a pull request or a failed member, for instance, names that item), and the notification that a whole
team is done names none. `--short-desc` takes one line only. The summary of a team that the daemon sends gains the optional field
`short_desc` in the current protocol (version 6); older negotiated protocols do not carry it, and a client from before
this version must be replaced together with the daemon. The text of the notifications changed: a program that reads
them must follow.

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
| `delegate`, `delegation_status`, `delegation_feedback` | worker requests (normal tier; an investigator in a team or on a request may ask too, a worker not); optional `requires` routes to a capable node |
| `team_status`, `claim_lead`, `handover_lead` | teams |
| `quiz_start`, `quiz_answer`, `quiz_results`, `team_exclude` | the qualification quiz |
| `board_add`, `board_list`, `board_claim`, `board_done`, `board_release` | the team board (`board_add` with `for=<member>`: only that member may claim the item) |
| `consult_start`, `consult_reply`, `consult_results` | ask the other normal members with a time limit (e.g. for recommendations); silence is shown as NO ANSWER |

**A request to a worker names its work.** A run whose role is in `[plan] roles` (a solo agent, a member or a
coordinator, the investigator among them when it leads or works as one) sends the four new arguments beside `goal`,
`workdir`, `steps` and `returns`: `for`, the ID of a step of its plan that is still to do or in hand; `files`, the paths
the worker may read and, when it is to produce something, write (relative to the repository, no globs); `check`, a
command or `exists PATH` that shows the result is right — never `reviewer`, never the repository's gate (`[plan]
check_refuse`); and `do`, what to do in the requester's words. The bus asks the harness's `agent _delegation-check` at
the moment of the request, which reads `.agent/plan.md` of the requester's worktree: a request that lacks one of the
four, names a step the plan does not have or that is done or given up, a path outside the repository, or a refused
check is refused with a cause of its own (`NoSuchStep`, `StepClosed`, `PathOutside`, `CheckRefused`, `PlanUnreadable`,
`MissingFor`, `MissingFiles`, `MissingCheck`, `MissingDo`, `CheckFailed`), nothing is queued and no worker starts. In a
team, such a request is also refused at once (`NoFreeWorker`) when no worker of the team is free to take it — none in
the team waits for work (a worker in a run of its own is busy), or every waiting one is promised to an earlier request
of the team — and the requester does the step itself; it is never served by a higher tier. The worker's request then quotes the step in full, then `Request files:`, `Request check:`
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
as they were. A commit that conflicts, or one that would write over a file the requester changed or has, is not applied:
the requester is told, with the files. A result that is wrong — the check failed or ran past its limit, the worker changed a
file the request did not name, it committed nothing, or its run lasted past `[workers] seconds` — is returned to the
requester with the reasons, and the request is answered as `failed`. A good result is told to the requester with the step,
the commit, the files and the end of the check's output, and the step is recorded `confirmed` in the history with the
worker's name. Each stage is written to `tasks/<worker task>/worker-request-<n>.json` before the next starts, so that a
daemon that stops in between goes on from the stage the file names: the check runs once and a commit is brought over once.
A second try with a worker of another CLI, and the lines of `agent status` and of the pull request's description for the
returned steps, come with the next version.

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
tasks that is done and names a step: `- step 1.2 of claude-1's plan — done by glm-1 (H3)`. A lead cannot write that
section itself.

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
