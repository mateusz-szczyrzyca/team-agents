# team-agents

Run a crew of headless LLM coding agents on your own Linux machine and let them work on your repositories
in parallel — alone or as teams — while you watch, answer their questions and review their pull requests.

team-agents does not implement an agent loop itself. It orchestrates existing agent CLIs —
**Claude Code**, **Codex CLI**, **Antigravity CLI (`agy`)** and **OpenCode** (for any API provider such as
OpenRouter or DeepInfra) — and adds what they lack when several of them work unattended on one host:

- **Isolation:** every task runs in its own git worktree and branch, as a background `systemd --user` unit.
- **Communication:** a small MCP server (`agentbus`, Go standard library only) gives all agents a shared message bus,
  resource reservations (debug probes, serial ports, test hosts), a persistent per-repository notebook and
  checkpoints.
- **Human in the loop:** agents never guess; they write questions to a Markdown file, the task pauses, you answer
  with `agent answer <id>` (or just save the file marked `**ANSWERED**`) and the agent resumes in the same session.
- **Model tiers:** cheap "worker" models do mechanical work (searching, listing, summarising output) on behalf of
  stronger "normal" models; expensive "investigator" models are queued for hard problems and PR reviews.
- **Load spread across providers:** because agents of different CLI types (Claude Code, Codex, Antigravity,
  OpenCode with OpenRouter or DeepInfra) and different tiers work side by side, the load and the token usage are
  distributed over several providers and accounts instead of exhausting one subscription's limits or one API
  budget.
- **Teams:** several agents share one task; they elect a coordinator, run a qualification quiz, split work on a
  shared board, merge into one draft PR and finish together.
- **Robustness:** a hang watchdog, automatic resume, checkpoints and `agent run --resume`, provider load
  balancing, a kill switch (`agent stop --running`).
- **Observability:** a live terminal panel (`agent live`), unified activity logs across all CLIs, full transcripts
  and 7z archives of finished work.

It was built for embedded development on a Raspberry Pi 5 (agents flash and debug real boards), but nothing in
it is specific to that: it works for any repository on any Linux host with `systemd --user`.

> **Security warning.** Agents run with their CLIs' permission prompts and sandboxes **disabled**
> (`--dangerously-skip-permissions`, `--dangerously-bypass-approvals-and-sandbox`, `--auto`). They can run any
> command as the user they run under, push to your repositories and use every credential that user can read.
> Read the [Security](#security) section before installing.

## Contents

- [How it works](#how-it-works)
- [Concepts](#concepts)
- [Security](#security)
- [Requirements](#requirements)
- [Installation](#installation)
- [Configuration](#configuration)
- [Usage](#usage)
- [The live panel](#the-live-panel)
- [MCP tools available to agents](#mcp-tools-available-to-agents)
- [Files and state](#files-and-state)
- [Notifications](#notifications)
- [Logs](#logs)
- [Example: embedded development on a Raspberry Pi](#example-embedded-development-on-a-raspberry-pi)
- [Troubleshooting](#troubleshooting)
- [Development and tests](#development-and-tests)
- [License](#license)

## How it works

```
you ── agent run claude-1 "task" ──► systemd --user unit  agent-<task>-r<n>
                                       │  runner: worktree + branch, prompt, secrets
                                       ▼
                         claude -p | codex exec | agy -p | opencode run   (JSON event stream)
                                       │ MCP (stdio)                  │ events
                                       ▼                              ▼
                               agentbus serve  ◄──────►  ~/agents/bus   activity.jsonl ─► agent live
                                   (one per run)          state.json, messages.jsonl,
                                                          notebooks/, checkpoints/
```

- `agent` (bash) is the command you use. It creates tasks, starts runners as `systemd --user` units, builds
  prompts, classifies how each run ended, resumes sessions, wakes sleeping team members, dispatches
  investigators and workers, and sends notifications.
- `agentbus` (Go) is started by every agent CLI as an MCP server over stdio. All instances share state through
  files guarded by `flock`. It also provides helper subcommands for the script (queue handling, teams, the
  event normalizer that turns each CLI's JSON stream into one activity format).
- `agentlive` (Go, Bubble Tea) is the live panel behind `agent live`.

## Concepts

### Multi-instance development preview

The approved [multi-instance design](docs/multi-instance.md) is being implemented in milestones.
The hub dispatcher, authentication primitives and MCP forwarding client are available for isolated tests;
instance registration, node supervision and cross-instance task routing are still under development.
Keep production installations in single-instance mode until these milestones are complete.

With `LEAD_INSTANCE` set, `agentbus serve` forwards tool calls to that HTTP origin and never reads or writes
local bus files. Set `INSTANCE_NAME` to the unique local instance name. `INSTANCE_CAPABILITIES` (comma separated,
recognized names from `agentbus/manifest.go` only) declares what this instance offers; a misspelled name stops `agentbus serve`
at startup. Without it the instance offers only its operating system, so board items with `requires` stay unclaimable here.
In hub mode the hub decides from the registered manifests, not from the local setting. `HUB_AUTH=tailscale` is the default;
`HUB_AUTH=token` requires HTTPS, `HUB_TOKEN` from `secrets.env`, and `HUB_CERT_FINGERPRINT` containing the
SHA-256 leaf-certificate fingerprint. The `agent` runner exports these settings from `agents.conf`.
Redirects are refused so credentials cannot be forwarded to a different endpoint.

`HUB_RETRY_SECONDS` defaults to 30 (allowed range 1–600). This bounds a contiguous outage, including connection,
TLS and response time. Retries retain the original request ID; an `OutcomeUnknown` response is final and must
be reconciled before submitting replacement work. Long polls reattach to their original execution with a
630-second total cap. Requests are limited to 1 MiB and responses to 64 MiB. Transport exhaustion becomes the
attempted operation's typed `Unavailable` error (exit class 11). Without `LEAD_INSTANCE`, `HUB_LISTEN` or `HUB_SOCKET`, the
existing local-file mode is unchanged. Setting `HUB_LISTEN` or `HUB_SOCKET` routes lead-local `serve` calls
through the hub daemon's owner-only Unix socket, with the same request envelopes and identity checks.
`HUB_SOCKET` overrides the default `<bus directory>/hub.sock`; `serve --dir` selects that bus directory.
Unqualified address arguments, including a board item's `for` assignee, resolve relative to the caller's
instance; explicitly qualified names keep their selected instance.
The socket uses local filesystem permissions instead of a bearer token. Its directory must not be writable
by other users. A missing or refused socket returns `Unavailable` and never falls back to local bus files.
The temporary `lead_local_fallback` event name is retained for old logs but is no longer emitted.

### Tasks and runs

`agent run <agent> "task"` (inside a git repository) creates a task: a worktree under `~/agents/worktrees`, a
branch `agents/<agent>/<timestamp>` and a directory `~/agents/tasks/<task id>` with the prompt, the raw event
stream, the activity log and the final message. A task can have several **runs** (after answers, after an
investigation, after being woken up); the agent CLI resumes the same session each time.

Every turn of an agent must end with a `STATUS:` line. The runner maps it to the task status:

| Agent says | Task status | What happens |
|---|---|---|
| `STATUS: DONE` | `done` | notification; in a team the coordinator's DONE finishes the whole team |
| `STATUS: QUESTIONS` | `waiting` | questions file `questions-N.md` → notification → you answer |
| `STATUS: INVESTIGATING` | `investigating` | waits for an investigator; resumes automatically with the result |
| `STATUS: SLEEPING` | `sleeping` | team members only; woken by messages and board items |
| `STATUS: FAILED` | `failed` | notification |
| no `STATUS` line | `waiting` | the last message is passed to you as a question ("what should the agent do next?") |

### Questions and answers

The prompts include a set of clarification rules (`prompts/clarification.md`): agents must not guess; if
anything that affects the result is unclear they write numbered questions to a file and stop. Agents that ask only
in their final message get their questions moved into a file automatically.

Every question comes with a **recommendation**: the option that makes the task simpler and less likely to go
wrong, why, and whose recommendation it is. In a team the coordinator first collects the members'
recommendations (`consult_start`, `AGENT_CONSULT_MIN` minutes, default 5; a member that does not answer in time is
shown as NO ANSWER, not as agreement) and sends questions that need research to an investigator; where the team
disagreed, the question shows the other views. A questions file without recommendations goes back to the agent
once.

```
## 1. Where should the cache live?
a) in memory — lost on restart
b) on disk — survives restarts, needs cleanup

**Recommended:** a) — nothing to clean up, restarts are rare here (coordinator; gemini-1 agrees, codex-1 preferred b)

**Verdict:**
```

You write your decision after `**Verdict:**`; **`ok` accepts the recommendation** (the agent gets the recommendation
spelled out). The labels are always these English words, whatever language the questions are in.

Answering:

- **Save the file with `**ANSWERED**` as its last line** — a systemd path unit notices the save and the answers are
  sent at once. If a verdict is missing you get a notification and nothing is sent; save again when complete.
  (Saved without the marker = still working on it.)
- or `agent answer <id>` (opens `$EDITOR`, validates, resumes), or `agent resume <id>` after editing the file.

**VS Code.** When you work on the agents' host through Remote-SSH, a new questions file opens in the VS Code window
connected to it (the newest one that responds). Set `QUESTIONS_EDITOR=off` to disable it; `VSCODE_SERVER_DIRS`
lists the server directories searched (default: VS Code, VS Code Insiders, Cursor).

The path unit (`agent-<task>-answers-<round>.path`) does not survive a reboot of the host: after one, answer with
`agent resume <id>`.

### Model tiers and providers

- **Normal agents** (`NORMAL_AGENTS`) do the real work and may coordinate teams.
- **Workers** (`WORKER_AGENTS`) are cheap models. They never coordinate. They take `easy` items from a team's
  board and handle `delegate` requests: a normal agent hands over a mechanical job, the script starts a free
  worker as a helper task on a copy of the requester's branch, and the result comes back as a message.
  Requests are balanced across providers (the provider with the fewest working workers wins).
- **Investigators** (`INVESTIGATORS_TYPE`) are expensive models in a shared queue. An agent stuck on a hard
  problem calls `request_investigation`; it ends its turn, an investigator works on a copy of its branch, and
  the agent resumes with the result and must record a decision before acting on it. The queue holds one
  request in progress per investigator plus `INVESTIGATION_QUEUE_MAX` waiting ones.

**Why mix tiers and providers.** Every subscription has usage limits (per few hours and per week) and every API
key has a budget. Splitting the agents into CLI types and tiers spreads the load and the token usage across
providers and accounts:

- normal agents of different types (for example `claude-1`, `codex-1`, `gemini-1`) draw on three separate
  subscriptions, so a team can work in parallel without one account hitting its limit;
- the token-hungry mechanical work (reading many files, long command output) goes to workers on pay-per-token
  providers (OpenRouter, DeepInfra) at a fraction of the price, instead of eating the subscriptions' limits;
- worker requests and board items are assigned to the provider with the fewest working agents, so the work is
  balanced across providers, not only across agents;
- the most expensive models (investigators) are used only for the problems that need them, through a queue.

### Teams

`agent run --team a,b,c "task"` gives one task to several agents, each on its own branch:

1. A normal member becomes the **coordinator** with `claim_lead` (the first one wins).
2. The coordinator runs a **qualification quiz**: one precise question from the task's domain with a 60 s
   limit. Answers are private; members who fail are excluded for this task and you are asked whether to
   continue.
3. The coordinator splits the work on a **board** (`easy` items for workers, `normal` items for normal models),
   opens a **draft PR** from its branch and merges every finished part into it, saving **checkpoints** along the
   way. Members push their branches after every commit. Idle members sleep and are woken by messages or by
   board items they have not seen yet (see [Waking, pausing and quotas](#waking-pausing-and-quotas)). An item
   for one particular member is added with `for=<member>`: only that member may claim it and only it is woken.
4. Only the coordinator asks you questions; members ask the coordinator.
5. The coordinator's `STATUS: DONE` (after `gh pr ready`) finishes the team.

With `--verify` the coordinator's DONE is checked first (PR exists and is not a draft, everything is pushed,
members' branches are merged); if something is missing the coordinator is sent back once with the list. Then an
investigator reviews the PR and publishes a GitHub review (as a comment — GitHub does not let an author approve
their own PR), and you get a notification with the recommendation.

### Checkpoints and resuming

Agents commit and push in small steps and call `checkpoint(summary, next_steps)` after every stage. If you stop
work — typically to extend the task — `agent run --resume <team|task> "addition"` starts the same agents on the
same branches (and the same draft PR), with the checkpoints, the earlier answers and your addition in the
prompt. A resumed team keeps its coordinator and board and skips the quiz.

### Waking, pausing and quotas

A sleeping member costs nothing; every wake-up starts a run and costs tokens. So:

- A **message** to a sleeping member (or to its team) wakes it.
- The **board** wakes a member only for work it has not seen yet: a new item at its level, an item returned to the
  board, or an item designated to it (`for`). An item it has already seen — in `board_list` or because it was
  woken for it — never wakes it again, even if it stays open.
- **Wake-loop guard.** A member that keeps being woken without doing anything — `AGENT_WAKE_LOOP_MAX` (default 4)
  idle runs in a row, an idle run being shorter than `AGENT_IDLE_RUN_SEC` (120 s) with no new commit and no bus
  message, or more than `AGENT_WAKE_RATE_MAX` (30) runs within 10 minutes — is **paused** for
  `AGENT_WAKE_LOOP_PAUSE_MIN` (15) minutes. You get a notification and the coordinator a `member_paused`
  message; the log shows `wake_loop_detected`.
- **Exhausted quota.** When a provider stops a run because its quota or usage limit is exhausted ("Individual
  quota reached… Resets in 24m46s", "usage limit reached", "try again in 2 hours", "resets 3pm"…), the task is
  **paused** until the limit resets (plus a minute; `AGENT_QUOTA_PAUSE_MIN`, default 30 min, when the CLI does
  not say when) instead of failing. This applies to solo tasks, team members, investigators and workers alike.

A paused task (`⏸ paused` in `agent status`) does not run and is not woken by messages or the board; a systemd
timer (`agent-<task>-wake-<time>.timer`) resumes it at the announced time, in the same session. Its claimed board
items stay with it. To resume it earlier: `agent _wake <task>` after the time has passed, or edit `WAKE_AT` in
`~/agents/tasks/<task>/meta.env`; to give up on it: `agent stop <task>`.

### The hang watchdog

If a run shows no activity for `AGENT_HANG_MIN` minutes (default 30), it is killed. Normal agents,
coordinators and investigators are resumed in the same session and told not to repeat the hanging command; after
`AGENT_HANG_STRIKES` consecutive hangs without progress the agent is marked `hung`. Workers are not resumed —
the normal agents are told instead. A team fails only if the coordinator and all normal members are hung.

Time when the hub cannot be reached does not count. `agentbus` records each hub outage in
`$AGENTS_HOME/hub-outage.json` (`total_seconds` of closed outages and `open_since` of the current one, written only
when reachability changes). The node daemon and every `agentbus serve` process of the instance update it. The
watchdog leaves out only the outage time since the run's last activity, so a run that waits for the hub is not
killed, a hang that starts while the hub is up is still detected, and the budget resumes when the hub answers. An
unreadable record is logged and counts as no outage. A typed refusal from a reachable hub is not an outage; an
`Unavailable` result is, including a hub that cannot serve the request.

### Repository notebook

A persistent notebook per repository (an append-only log plus overwritable sections) carries knowledge between
tasks: device quirks, toolchain pitfalls, conventions. Agents read it at the start of every task.

## Security

team-agents gives AI models a shell without supervision. Treat the agents' user as untrusted: anything it can
read, the agents can read — and possibly leak, if a prompt injection (instructions hidden in a web page, an issue,
a dependency's README, a file in the repository) convinces an agent to do so. Design the setup so that a
compromised or confused agent can do as little damage as possible.

### Where to run it

- **Linux only for now** (it relies on `systemd --user`); macOS support is planned.
- **A dedicated machine or VM** — a small server, a Raspberry Pi, a VPS or a local VM — with nothing else of value
  on it: no personal files, no production credentials, no browser sessions, no password manager.
- **A dedicated, unprivileged user** for the agents, never your own login user. `~/agents` is created with mode
  `700`; keep it that way.
- **No sudo by default.** Passwordless sudo lets the agents install missing tools on their own, but it also lets
  them run anything as root: give it **at your own risk**, and preferably only on a disposable VM.
- **Nothing irreplaceable on the host.** Back up `~/agents/agents.conf`, `secrets.env` and `hardware.md`; be ready to
  rebuild the machine from scratch.

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
  agent's name (see [Commit identity](#commit-identity)), but pushes, PRs and reviews appear as you.

Put the token into `secrets.env` as `GH_TOKEN` (gh uses it instead of a stored login). In every variant:

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
- Remember that `secrets.env` and the CLIs' login files are **readable by the agents** — they run as the same user.
  Keep the scopes small and **rotate** the keys regularly, and immediately after anything suspicious.
- Notification hooks (`NOTIFY_CMD`) get their tokens from `secrets.env` too: use a token that can only send
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
  anything that runs with privileges. `--verify` adds an investigator's review, not a replacement for yours.
- Be wary of tasks that feed agents **untrusted input** (issues written by strangers, scraped web pages) while they
  hold write access: that is the classic prompt-injection path.

### Monitoring and response

- Ship the structured logs (`~/agents/logs/*.jsonl`, the journal) somewhere you look at, and alert on warnings and
  errors (see [Logs](#logs)). `agent transcript <task|team>` shows everything a task did, message by message.
- Logs and transcripts can contain secrets printed by commands: restrict who can read your log storage.
- The kill switch is `agent stop --running`: it stops every task, investigation and worker request at once.
- After an incident: stop everything, revoke the agents' keys and tokens (GitHub, providers, SSH), check the
  transcripts and the git history of the affected repositories, and rebuild the host if in doubt.

## Requirements

- Linux (macOS support is planned) with `systemd --user` and lingering enabled for the agents' user:
  `sudo loginctl enable-linger <user>`
  (otherwise units stop when you log out).
- `git`, `jq`, `flock` (util-linux), `curl`, Go ≥ 1.22 (to build), `gh` (GitHub CLI) for pull requests,
  `7z` (p7zip) for `agent archive`.
- At least one agent CLI, logged in as the agents' user:
  - [Claude Code](https://docs.anthropic.com/en/docs/claude-code) (`claude setup-token` for a subscription token),
  - [Codex CLI](https://github.com/openai/codex) (`codex login --device-auth`),
  - Antigravity CLI `agy` (interactive login once),
  - [OpenCode](https://opencode.ai) for API providers (OpenRouter, DeepInfra, …) — API keys in `secrets.env`.
- A git remote the agents may push to, and an SSH key without a passphrase (units do not see `ssh-agent`).
- Tools the agents use while working on code: `rg` (ripgrep), `grep`, `sed`, `awk`, `find`, `xargs`, `diff`, `patch`,
  `file`, `tar`, `gzip`, `unzip`, `make`, `python3`, `curl`, plus your project's toolchains. `install.sh` checks for
  them and prints the command that installs the missing ones (apt, dnf, pacman, apk, zypper).

**Installing tools: sudo or not.** If the agents' user has passwordless sudo, the prompts allow an agent to install
a missing tool with the system package manager (and only that; it must list what it installed in its report).
Without sudo, an agent reports what was missing instead. Passwordless sudo is convenient, but it also lets the
agents run anything as root — it is given at your own risk (see [Security](#security)).

## Installation

```bash
git clone <this repository> && cd team-agents
./install.sh
```

`install.sh` checks the required and recommended tools (see Requirements), builds `agentbus` and `agentlive` into
`~/.local/bin`, installs the `agent` script and the prompts
into `~/agents/prompts`, and creates `~/agents/agents.conf`, `~/agents/hardware.md` (environment notes) and
`~/agents/secrets.env` from templates if they do not exist. Re-running it updates everything except your
configuration (a new template is saved as `agents.conf.new`).

Then:

1. Log in to the agent CLIs you want to use (see Requirements).
2. Put tokens and API keys into `~/agents/secrets.env` (`chmod 600`).
3. Edit `~/agents/agents.conf`: agents, models, tiers, investigators, language, notifications.
4. Describe shared resources (devices, test hosts) in `~/agents/hardware.md`.
5. Give the agents a GitHub identity: a token of the agents' GitHub account in `secrets.env` as `GH_TOKEN`, and
   optionally `GIT_EMAIL`/`GIT_SIGNING_KEY` in `agents.conf` (see [Security](#security) and
   [Commit identity](#commit-identity)).
6. Check the accounts: `agent authenticated` — every configured provider should report `OK`.
7. Try it: `cd <your repo> && agent run claude-1 "List the toolchain versions you find. Do not change anything."`
   and watch with `agent live`.

## Configuration

`~/agents/agents.conf` is a bash file. The most important settings:

| Setting | Meaning |
|---|---|
| `AGENT_TYPE[name]` | `claude`, `codex`, `gemini` (agy) or `opencode` |
| `AGENT_MODEL[name]` | model name; for opencode `provider/model`, e.g. `openrouter/@preset/my-worker`, `deepinfra/<model>` |
| `AGENT_EXTRA_ARGS[name]` | extra CLI arguments (e.g. reasoning effort) |
| `NORMAL_AGENTS`, `WORKER_AGENTS` | tiers (an agent in neither list is normal); worker order = delegation preference |
| `INVESTIGATORS_TYPE/_MODEL/_EXTRA_ARGS[name]` | investigators (the legacy single `INVESTIGATOR_*` settings still work) |
| `INVESTIGATION_QUEUE_MAX` | waiting investigation requests (default 3) |
| `AGENT_HANG_MIN`, `AGENT_HANG_STRIKES` | watchdog (default 30 min, 3 strikes) |
| `AGENT_IDLE_RUN_SEC`, `AGENT_WAKE_LOOP_MAX`, `AGENT_WAKE_RATE_MAX`, `AGENT_WAKE_LOOP_PAUSE_MIN` | wake-loop guard (default 120 s, 4 idle runs, 30 runs / 10 min, 15 min pause) |
| `AGENT_QUOTA_PAUSE_MIN` | pause after an exhausted quota when the CLI does not say when it resets (default 30 min) |
| `AGENT_CONSULT_MIN` | how long the coordinator waits for the members' recommendations before asking you (default 5 min) |
| `QUESTIONS_EDITOR`, `VSCODE_SERVER_DIRS` | open new questions files in the connected VS Code window (`vscode`, default) or not (`off`); where VS Code servers live |
| `AGENT_LANGUAGE` | language of questions and notifications — what only you read (default English) |
| `REPO_LANGUAGE` | language of commits, PRs, reviews, reports and code comments (default English) |
| `GIT_NAME_FORMAT`, `GIT_EMAIL`, `GIT_SIGNING_KEY` | identity and signing of the agents' commits (see [Commit identity](#commit-identity)) |
| `ENV_NOTES_FILE` | environment notes shown to the agents (default `~/agents/hardware.md`) |
| `RUN_PATH` | `PATH` inside agent runs (add your toolchains) |
| `NOTIFY_CMD`, `NOTIFY_DIR`, `NOTIFY_MAX_CHARS` | notifications (see below) |
| `CLAUDE_BIN`, `CODEX_BIN`, `AGY_BIN`, `OPENCODE_BIN` | CLI paths |
| `AUTH_MODEL_*` | cheap models used by `agent authenticated` |
| `BRANCH_PREFIX`, `CLAUDE_MAX_TURNS`, `AGY_PRINT_TIMEOUT`, `OPENCODE_BASH_TIMEOUT_MS` | misc |

`~/agents/secrets.env` is loaded before every run: `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY`,
`CODEX_API_KEY`, `GEMINI_API_KEY`, `OPENROUTER_API_KEY`, `DEEPINFRA_API_KEY`, `GH_TOKEN` (the GitHub account the
agents act as) and anything your notification hook needs.

**OpenCode notes.** Each run uses `opencode run --standalone` (a private server per run, so it sees the keys and
the generated per-run configuration with the agentbus MCP server; the shared background server of OpenCode 2
would not). OpenRouter presets (`openrouter/@preset/<slug>`) are registered automatically, so you can change a
worker's model and provider routing in the OpenRouter dashboard without touching the configuration. The model
must support tool calling.

### Repository instructions (AGENTS.md)

The prompts describe how agents work *with team-agents*; what they need to know about *a repository* — what it
is, its architecture, its conventions, how to build, lint and test it, what must never be done — belongs in an
`AGENTS.md` file in that repository. team-agents makes it part of every run:

- **Every prompt** (tasks, teams, investigations, worker requests, PR reviews) tells the agent to read `AGENTS.md`
  in the repository root and every `AGENTS.md` in the directories it works in (the nearest one applies to its
  directory), and treats them as binding: the prompts decide how agents communicate, `AGENTS.md` decides how the
  code is written and checked.
- **Agents do not change `AGENTS.md`** (nor `CLAUDE.md`, `GEMINI.md`) unless the task explicitly asks for it;
  what they learn goes to the repository notebook, and suggestions to the report.
- **PR reviews** (`--verify`) check compliance with `AGENTS.md`; a violation is at least a major problem.
- **A repository without `AGENTS.md`** still works, but `agent run` prints a warning, logs `agents_md_missing`,
  and every notification of the task notes it; the agents are told to follow the conventions they see in the
  code and to say in their report that an `AGENTS.md` would help.

What to put in it: a short description of the repository and its components; the commands to set up, build,
lint and test (and how to install missing linters); coding conventions (error handling, types, comments,
logging, style); test rules (for example "never modify existing tests without the human's request"); what must
never be done (deploys, production hosts, generated files, secrets); compatibility constraints; the definition
of done. Add a `CLAUDE.md` symlink to it for interactive Claude Code sessions. This repository's own
[`AGENTS.md`](AGENTS.md) is a complete example.

### Commit identity

The runner sets the author and committer of every commit an agent makes, through git's environment variables,
so it does not depend on the agent following instructions:

| Setting | Effect |
|---|---|
| `GIT_NAME_FORMAT` (default `%a (team-agents)`) | the name, e.g. `claude-1 (team-agents)` — visible in `git log` and on GitHub |
| `GIT_EMAIL` | the e-mail. GitHub attributes a commit to the account that owns its e-mail: use the noreply address of the agents' account (*Settings → Emails*, `<id>+<login>@users.noreply.github.com`). Empty = the e-mail from your git config, so GitHub shows the commits under your account; without any e-mail the fallback is `<agent>@team-agents.invalid` (with a warning in the log) |
| `GIT_SIGNING_KEY` | an SSH private key that signs every commit. Add its public key to the agents' account as a **signing key** (*Settings → SSH and GPG keys → New SSH key → Key type: Signing key*) and GitHub marks the commits **Verified** |

Every commit message also ends with trailers, and pull requests and reviews end with a signature line:

```
Fix debounce of the button on GPIO 15

Agent: claude-1
Agent-Model: claude/sonnet
Task: 20261020-140312-claude-1
Team: 20261020-140312-team
```

`Opened by team-agents: team …, coordinator claude-1 (claude, model sonnet), members: …` at the end of a PR,
`— Review by the team-agents investigator inv-opus (claude, model opus), request #7.` at the end of a review.
Commit messages, PRs, reviews and reports are written in `REPO_LANGUAGE` (default English); questions and
notifications in `AGENT_LANGUAGE`.

## Usage

```
agent run <agent> "task" | -f task.md         start a task (inside the repository)
agent run --team a,b[,c] [--verify] "task"    start a team (optionally with a final PR review)
agent run --resume <team|task> ["addition"]   resume from the last checkpoint (same branches)
agent node                                  run this instance's outbound node daemon
agent status | agents | show <id>            overview, agents, task details
agent log <id> [--raw]                        follow the runner log (structured records rendered; --raw: JSON)
agent answer <id>                             answer the questions in $EDITOR and resume
agent resume <id> ["note"]                    resume after editing the answers yourself
agent live [--feed] [--remind MIN] [--light]  live panel (or an event stream)
agent say <agent|all|team:ID> "text"          message agents through the bus
agent inv [N|--all] | helpers [--all]         investigation queue / worker requests
agent team [id|--all] | notes [repo]          teams / repository notebooks
agent transcript <id|team> [--raw]            chronological log: bus messages + activity of every agent
agent stop <id> | --running                   stop one task / EVERYTHING that is active
agent ack <id|all> | clean <id>               acknowledge finished tasks / remove a worktree
agent archive <id|team> | --finished          pack finished work into ~/agents/tasks-archived/*.7z
agent discard <id>...                         remove tasks that never started (worktree, branch, task directory)
agent authenticated                           check every provider account
```

`<id>` can be any unambiguous fragment of a task ID. `agent run --resume` and `agent transcript` also accept a
team ID (`<timestamp>-team`) or any member's task ID.

## The live panel

With a configured hub, `agent status` and `agent live` request an authenticated aggregate overview from the
lead over the configured TLS/Tailscale connection (or the lead's private Unix socket). Every task shows its
owning instance, and task references use `task@instance` so identical local task IDs stay distinct. These
views use reported task metadata, not another instance's filesystem. The panel retains its last overview
with a visible warning during an outage and signals recovery; status fails clearly instead of reading a
second local bus. Remote task files, prompts, reports and acknowledgment remain on the owning instance;
answer/stop/message actions use the qualified reference through the normal CLI routing.

The internal `agentbus snapshot [--dir D] [--timeout SECONDS]` command emits the versioned public overview.
The timeout defaults to 3 seconds (range 1–600). The panel requests a 2-second refresh deadline and logs
`hub_view_unavailable` once when refreshing fails. In local-file mode the existing panel and status are
unchanged.

`agent live` shows all active and unacknowledged tasks (status, duration, repository, stage, last activity), a
detail view with tabs (1 Activity, 2 Prompt, 3 Questions, 4 Investigations, 5 Result, 6 Notebook, 7 Board) and an
event stream with bus messages, status changes and periodic reminders.

When the hub records instances in `bus/instances.json`, the panel shows an instance table (at most four rows) below
the task list: name, lease (`active` or `expired`), platform, free/total slots, active tasks and declared
capabilities. `agent status` prints the same table after the task list. Single-instance installations show neither.
The table reads only the registry file; it does not change any state.

| Key | Action |
|---|---|
| `↑↓` / `j k` | select a task |
| `1`–`7`, `Tab` | tab |
| `PgUp` `PgDn` `End` | scroll |
| `a` | answer the selected task's questions |
| `s` | stop the selected task |
| `m` / `M` | message the selected agent / everyone |
| `x` / `X` | acknowledge the selected / all finished tasks |
| `f` | full-screen event stream |
| `q` | quit (tasks keep running) |

## MCP tools available to agents

| Tool | Purpose |
|---|---|
| `list_agents`, `send_message`, `read_messages`, `wait_for_message`, `set_status` | communication |
| `reserve`, `release`, `list_reservations` | shared resources |
| `notebook_read`, `notebook_append`, `notebook_write_section` | the repository notebook |
| `checkpoint` | save a resumable work state |
| `request_investigation`, `investigation_status`, `record_decision` | investigators (not for investigators themselves) |
| `delegate`, `delegation_status` | worker requests (normal tier) |
| `team_status`, `claim_lead`, `handover_lead` | teams |
| `quiz_start`, `quiz_answer`, `quiz_results`, `team_exclude` | the qualification quiz |
| `board_add`, `board_list`, `board_claim`, `board_done`, `board_release` | the team board (`board_add` with `for=<member>`: only that member may claim the item; `requires=[capabilities]`: only an instance that provides all of them may claim it, and `board_claim` says which one is missing; sensitive ones also need the operator's approval in `node/approved-capabilities.json`, see `NeedsApproval`) |
| `find_capability` | live instances that offer a capability (e.g. `build:ios`), with platform, free slots and agents |
| `consult_start`, `consult_reply`, `consult_results` | ask the other normal members with a time limit (e.g. for recommendations); silence is shown as NO ANSWER |

### Instance registration and leases

The hub exposes authenticated `POST /v1/instances/register` and
`POST /v1/instances/heartbeat` endpoints. Registration carries protocol `version: 1`, a random
`registration_id` retained for one daemon session, a positive `sequence`, and the instance `manifest`.
It returns `lease_token`, `generation`, and `expires_at` (UTC). Heartbeats carry `instance_name`,
the current lease credentials, a strictly increasing `sequence`, and the replacement manifest.
Nodes should heartbeat every 60 seconds. `MANIFEST_TTL` (default 180 seconds; 1–86400) is the hub's
lease duration; `agentbus hub --manifest-ttl SECONDS` overrides the environment. A node cannot extend
its lease by advertising a larger TTL.

A live lease rejects another daemon session using the same instance name. Repeating registration
for the current session returns the same lease without replacing newer heartbeat data. After expiry,
the owning node registers again and receives a new token and generation. Old heartbeats and command
credentials are fenced. A different Tailscale node sharing an allowed tag cannot take ownership of
an existing instance name. A changed authenticated node identity requires administrative reconciliation
of its registration before reusing that name.

The new, private `bus/instances.json` stores leases and the last manifest under the existing bus lock.
Lease tokens are credentials: keep this file and its backups private. Existing installations need no
migration; an absent registry is empty and the legacy `state.json` format is unchanged. Registration
errors use operation `instance_register`; heartbeat errors use `instance_heartbeat`, with generated
cause names and the existing error classes. The later node and routing milestones connect these leases
to remote task execution and member reachability.

### Trusted run identity

Hub tool calls require a live instance registration owned by the authenticated peer, plus a task summary
matching the caller's agent, task, team, tier and role. Team members use qualified keys (`agent@instance`)
and must also match the hub's team assignment; excluded members are denied. Helper calls require their
qualified worker assignment. Remote investigator assignments must explicitly bind both the investigator
base name and its instance; older queue records still load but do not authorize remote investigators.
Normal solo runs are verified against their instance's task summary.

Task summaries now optionally include `repo`, `branch`, `worktree`, `tier`, and `role`. Earlier manifests
remain readable; summaries without identity fields cannot authorize remote tool calls. Lead-local runs
are verified against local `meta.env` files without evaluating shell content. `agentbus hub --tasks-dir DIR`
selects that metadata root (default: the `tasks` directory beside the bus directory). Qualified team
membership applies only in multi-instance mode; existing single-instance names and files keep working.
The lead's Unix socket uses the configured `INSTANCE_NAME` (or `--instance`), defaulting to `lead`.

Remote worktree paths remain display metadata. Investigation records for remote callers are stored under
`bus/remote-tasks/<instance>/<task>/investigations`, never inside those remote worktree paths. Unknown,
mismatched, excluded, or unreachable identities fail with the typed hub `Identity` cause; unreadable state
fails with `Storage`. These checks also apply when attaching to a previous request after reconnection.

### Hub backups and expired instances

The hub writes a backup of its state every `HUB_BACKUP_MINUTES` (default 30; allowed 1–1440) into
`HUB_BACKUP_DIR` (default `<bus directory>/backup`). A backup is one snapshot taken under the bus lock: `state.json`,
`messages.jsonl`, `instances.json`, `commands.json` and the `replay/` ledger, written to a `backup-YYYYMMDD-HHMMSS`
directory (UTC). The newest 10 backups are kept. The directory and its files are private because they hold
lease credentials. An unusable `HUB_BACKUP_MINUTES` stops `agentbus hub` at startup.

Every 30 seconds the hub checks the leases in `bus/instances.json`. For an instance whose lease has expired (no
heartbeat for `MANIFEST_TTL`), each member of a team whose name is qualified with that instance (`codex-1@mac`) becomes
`unreachable` (unless it was done or excluded), and each board item claimed by such a member returns to `todo` with a
note in its details. The team's board revision advances, and one `instance_unreachable` message goes to the
coordinator, or to the whole team when the coordinator is on that instance too. Unqualified names belong to the
local installation and are never reclaimed. The check is idempotent: a second scan of the same expired instance
changes nothing. Members are not restored automatically when the instance returns; reconnection does not revive a
released board claim.

### Node daemon

On a non-lead instance, `agent node` loads the local configuration and secrets, registers with the hub,
then heartbeats every minute while independently long polling for commands. Each instance keeps its own
models, credentials, worktrees and local task files. For token mode set `HUB_TOKEN` in `secrets.env` and
`HUB_CERT_SHA256` in `agents.conf`; the earlier `HUB_CERT_FINGERPRINT` name remains accepted.
`INSTANCE_CAPABILITIES` is an array from the closed vocabulary in the design. Unknown values fail startup.
`INSTANCE_ACCEPT_REPOS` limits repositories accepted by the local script, and direct daemon use can add
`agentbus node --accept-repos repo-a,repo-b` as an additional local restriction.

The node accepts only fixed internal operations against locally provisioned task directories. The hub
cannot select an executable or submit a shell command. A private readiness pipe confirms local admission
before a start, wake or answer-delivery command is acknowledged. The daemon supervises its child process
groups directly and stops those groups during graceful shutdown. Hub outages leave local runs working.
A second daemon using the same local home fails with the typed `node.AlreadyRunning` cause.

`node/state.json` records the session identity, report sequences, lease credentials and command outcomes;
`node/daemon.lock` prevents concurrent daemons and `node/executor.log` captures bridge output. These local
files do not replicate or execute logical bus state. Acceptance is synced before dispatch and the outcome
before acknowledgment; a lost acknowledgment reuses the recorded outcome. If a crash leaves an accepted
operation with no recorded outcome, the native executor reports an explicit unknown outcome and does not
blindly spawn it again. Inspect that task before submitting replacement work. Keep the journal private and
retain it across reconnects: deleting it discards the node's protection against duplicate execution.

Message and board wakeups for a qualified member are queued as `wake` commands for its owning instance,
including a member on the lead instance. The hub checks the live lease and reported task ownership while
holding its existing bus lock; it never starts another instance's wake script locally. Pending wake commands
for the same member/task coalesce. An expired lease or queue/storage failure logs `side_effect_failed` and
leaves the member sleeping with its triggering message unread. Unqualified local teams retain the existing
wake hook. The owning instance must run `agent node` to poll and acknowledge these commands. Run a node on the lead
instance too when it owns qualified team members. With `LEAD_INSTANCE` empty or equal to `INSTANCE_NAME`
and `HUB_LISTEN` or `HUB_SOCKET` configured, that node selects the owner-only private hub socket. It needs no
self-facing network address, token or certificate pin; it does not transmit an inherited network token to
that socket. An explicitly different lead origin retains the configured network transport. Missing local
sockets are outages, with no local-file fallback.

Sensitive `sign:*` and `publish:*` requirements are denied unless the local human has approved them in
`node/approved-capabilities.json` (a JSON array of typed capabilities). Requirements must also be declared
locally. The same file gates board items: a `board_add` item whose `requires` includes `sign:*` or `publish:*`
can be claimed only by an instance whose `AGENTS_HOME` lists that capability there. Otherwise `board_claim`
fails with `NeedsApproval` and the item stays on the board. Items without sensitive requirements never read the
file, and a missing file approves nothing. Claims executed by the hub (which has no local home for the claiming
instance) therefore cannot take sensitive items. Limitation: the approvals file is protected only by the operating
system account. Agents on the same account can write it, so only give that account to agents you trust with
signing or publishing. Remote code provisioning, question snapshot retrieval, full approval-command integration, and
cross-instance run creation are completed by their remaining milestones; the node daemon alone does not
create a missing task or clone a repository selected by the hub.

## Files and state

```
~/agents/
  agents.conf  secrets.env  hardware.md      configuration
  prompts/                                   prompt templates ({{PLACEHOLDERS}})
  tasks/<task id>/                           meta.env, task.md, prompt-N.md, out-N.jsonl / events-N.jsonl,
                                             activity.jsonl, activity-full.jsonl, questions-N.md, final.md,
                                             result.md (investigations), investigations/
  worktrees/<repo>/<task id>/                git worktrees
  bus/state.json  bus/messages.jsonl         agentbus state and message log
  bus/notebooks/<repo>/                      repository notebooks
  bus/checkpoints/                           checkpoints (team-<id>.md, task-<id>.md)
  tasks-archived/*.7z                        archives (agent archive)
  notifications/                             notification files
  logs/agent.jsonl                           structured log of the agent script (JSON lines)
  logs/agentbus.jsonl  logs/agentlive.jsonl  structured logs of the bus servers/commands and the live panel
```

The instance table in `agent status` compares lease expiry as UTC instants. It accepts both hub-written
RFC3339 timestamps (including fractional seconds) and the older space-separated UTC representation.
`agentbus instances` keeps its tab-separated output and exit codes; read/write failures now carry the
operation name `instances` and its own `Storage` cause instead of the unrelated `status` cause.

## Notifications

After every run (and on a few team events) the script writes a notification file — the first line is a title
(the repository), the rest is the body — and calls `NOTIFY_CMD <file>`. Ready-made hooks are in
`examples/notify/`:

```bash
# agents.conf
NOTIFY_CMD="$HOME/agents/notify/ntfy.sh"        # needs NTFY_URL (and optionally NTFY_TOKEN) in secrets.env
NOTIFY_CMD="$HOME/agents/notify/pushsafer.sh"   # needs PUSHSAFER_KEY in secrets.env
```

With `NOTIFY_CMD` empty the files are only written to `NOTIFY_DIR`. Older configurations that set
`NOTIFY_SCRIPT` (a script run inside `NOTIFY_DIR` with the file name) keep working.

`NOTIFY_CMD` must be an absolute path to an executable file. A relative name (`notify.sh`) or a file that is
not executable is a configuration error: `agent` refuses to start tasks and prints the problem (`agent status`
still runs, so it can show the problem). A failed delivery is logged as a `notification_failed` warning and kept
in `$AGENTS_HOME/notify-failure.txt`; `agent status` prints it as `last notification failure: ...` until a later
delivery succeeds.

## Logs

Every component writes structured logs as one JSON object per line, ready for Promtail, Vector, vmagent or any
other shipper to Loki or VictoriaLogs, and sends the same records to the systemd journal:

| Component | File | Journal identifier |
|---|---|---|
| `agent` (script, runners and interactive commands) | `~/agents/logs/agent.jsonl` | `agent` |
| `agentbus` (MCP servers and CLI subcommands) | `~/agents/logs/agentbus.jsonl` | `agentbus` |
| `agentlive` (the live panel) | `~/agents/logs/agentlive.jsonl` | `agentlive` |

Every record has a timestamp, a level, the component and an event name, plus the context of the event:

```json
{"ts":"2026-10-20T14:05:11Z","level":"info","component":"agent","event":"run_finished","pid":4121,
 "subcommand":"_runner","task":"20261020-140312-claude-1","run":"2","status":"done","note":""}
{"ts":"2026-10-20T14:05:09Z","level":"warn","event":"side_effect_failed","component":"agentbus","role":"serve",
 "agent":"claude-1","task":"20261020-140312-claude-1","effect":"write investigation record",
 "error":"cannot open …/inv-3.md: permission denied","class":"Storage","cause":"Storage","investigation":3}
{"ts":"2026-10-20T14:06:30Z","level":"warn","event":"tool_call_failed","component":"agentbus","tool":"quiz_start",
 "error":"quiz_start: only the coordinator (claude-1) can do this","class":"Permission","cause":"NotCoordinator"}
```

All three components use the same schema — `ts` (RFC 3339, UTC), `level` (`debug`, `info`, `warn`, `error`),
`component` and `event` — so one query covers all of them. The event names of the Go programs are typed
constants (`LogEvent` in `agentbus/logging.go` and `live/logging.go`).

Errors are never swallowed. A failure of the main operation fails it (the agent gets a tool error, a command
exits with a non-zero status). A failure of a side effect — an investigation record file, a bus notice, a
cleanup step — is logged as a warning and the operation continues. Problems found while polling (a corrupt
line in a file read every few seconds) are logged once per process. Unix socket cleanup failures are logged
as `side_effect_failed`; a socket already removed by listener shutdown is expected. A background wake timer
that cannot be removed from the shell's job table logs `wake_timer_disown_failed`; its diagnostic output is
preserved.

Useful queries:

```
# Loki
{job="team-agents"} | json | level=~"warn|error"
{job="team-agents"} | json | component="agentbus" | event="tool_call_failed"
{job="team-agents"} | json | event="run_finished" | status="failed"
# VictoriaLogs
component:agentbus AND class:Storage
event:tool_call_failed AND cause:NotCoordinator
event:notification_failed
```

`agent log <id>` follows a task's `runner.log` and renders the records readably
(`14:05:11 INFO run_finished run=2 status=done`); output of tools such as `git` passes through unchanged and
`--raw` shows the JSON as written.

**Bus messages are structured too.** Every message in `bus/messages.jsonl` has a `kind` (for example
`investigation_finished`, `board_item_added`, `coordinator_claimed`; `chat` for messages written by agents and
humans; `script` for events of the `agent` script, with a `notice` such as `member_finished` or `worker_hung`)
and the event's fields in `data`, next to the `text` the agents read:

```json
{"id":42,"ts":"2026-10-20 14:05:11","from":"agentbus","to":"all","kind":"investigation_finished",
 "data":{"id":3,"status":"done","summary":"the DMA channel is reused by the UART driver"},
 "text":"INVESTIGATION #3 finished [done]: the DMA channel is reused by the UART driver"}
```

The kinds are defined in `agentbus/events.go`.

## Example: embedded development on a Raspberry Pi

The project grew out of this setup: a Raspberry Pi 5 with two Raspberry Pi Debug Probes connected to RP2040 and
RP2350 boards, agents building Rust/Embassy firmware and flashing it with `probe-rs`.

```bash
# USB access for the agents' user (udev rules for VID 2e8a, serial ports)
sudo install -m644 examples/raspberry-pi/60-rpi-debug-probe.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
sudo usermod -aG dialout,plugdev <user>
```

In `~/agents/hardware.md` give every probe a name (e.g. `probe-rp2350`), its serial number (`probe-rs list`), the
`probe-rs` selector, the chip and the board's UART path; agents then `reserve probe-rp2350` before flashing.
Add the toolchains (`~/.cargo/bin`, `probe-rs`, `picotool`) to `RUN_PATH`. If you run an application firewall
(e.g. OpenSnitch), allow the agent CLIs and `go` (module downloads during `install.sh`).

## Troubleshooting

- **Units stop when you log out** — enable lingering: `sudo loginctl enable-linger <user>`.
- **`agent authenticated` says "needs authentication" for OpenRouter/DeepInfra** — the key is missing from
  `secrets.env` or rejected (HTTP 401). Keys must be `NAME=value` without spaces around `=`.
- **OpenCode: `provider.no-route: Model unavailable`** — the shared OpenCode 2 server did not get the keys or the
  configuration; make sure you run the current `agent` (it uses `--standalone`).
- **`cannot decode …/state.json: it contains a value this version does not know`** — the bus state holds a value
  no version of team-agents writes (a downgrade after a newer version ran, or a manual edit); every bus operation
  fails until it is fixed. Find it with `jq` (for example `jq '.. | .tier? // empty' ~/agents/bus/state.json`), fix
  the value, or stop all tasks and move `state.json` aside (queues, teams and boards are then forgotten). Values
  written by earlier versions are always accepted.
- **Tasks left with status `new`** after an interrupted `agent run` — remove them with `agent discard <id>...`.
- **A task is `paused`** — its provider's quota is exhausted or the wake-loop guard stopped it; the notification
  and `agent log <id>` say which and until when. It resumes by itself (see
  [Waking, pausing and quotas](#waking-pausing-and-quotas)).
- **A task finished as `failed` without a message** — look at `agent log <id>`, `~/agents/tasks/<id>/err-N.log` and
  the warnings in `~/agents/logs/*.jsonl` (`jq 'select(.level == "warn" or .level == "error")' ~/agents/logs/*.jsonl`).
- **Deleting `~/agents/tasks` by hand** makes tasks disappear from the lists but leaves worktrees and branches;
  prefer `agent clean <id>` or `agent archive`, then `git worktree prune`.
- **Commands that legitimately run longer than `AGENT_HANG_MIN` without output** are treated as hangs; raise the
  limit or have agents run such commands in the background.

## Development and tests

This repository is developed mostly by its own agents; [`AGENTS.md`](AGENTS.md) holds the rules for them (and
for human contributors): architecture, conventions, checks, and what must never be done.

```bash
make tools         # installs the pinned linters (golangci-lint, staticcheck, shellcheck) into ~/.local/bin
make lint          # shellcheck, gofmt, golangci-lint, staticcheck, doccheck, generated-code check
make test          # Go unit tests (agentbus, agentlive) + bats tests of the agent script
make integration   # real agent CLIs and accounts (costs usage)
```

The `agentbus` Go suite initializes a temporary `HOME`, `AGENTS_HOME`, bus directory and file logger before
running tests. Default wake and helper-dispatch hooks record requests without spawning the installed agent;
only tests that assert wake details install `captureWakes`. Node subprocess tests use temporary executable
stubs or repository-script paths that reject work before starting an agent. Toolchain detection runs read-only
version probes. A view integration test builds the stdlib-only agentbus helper into a temporary directory
with a temporary Go cache, then runs only `agent status` against an in-process TLS hub in a separate home.
Panel-source tests use a temporary snapshot stub. The suite logger does not send to the host journal. On Linux, a final audit fails the suite
if it leaves a new open file handle under the original agents home; inherited runner descriptors are allowed.
This audit detects leaked logger/storage handles, not arbitrary open-write-close access, so explicit file paths
and subprocess fixtures must still use temporary directories. No existing test assertion is weakened.

- **Linters.** `tools/install-linters.sh` downloads pinned release binaries of `golangci-lint`, `staticcheck` and
  `shellcheck` from GitHub, verifies their SHA-256 checksums and installs them without sudo. `golangci-lint`
  runs the standard linters plus `errorlint`, `gosec`, `misspell`, `revive` and `unparam` (`.golangci.yml`);
  every Go module and every Bash script (bats files included) must pass with zero findings. CI runs the same.

- **Unit tests** need no accounts: `agentbus` is tested in-process (queue capacity, quiz privacy, board rules,
  provider balancing, checkpoints, event normalization), and the `agent` script is tested with
  [bats](https://github.com/bats-core/bats-core) against stub CLIs, a stub `systemd-run`/`systemctl` and a stub
  `gh` (`tests/unit/stubs/`): status handling, questions, the watchdog, teams, `--resume`, `stop --running`,
  transcripts, archives, the structured logs, typed bus notices, logged failures, commit identity and signing,
  and `AGENTS.md` handling. They run in CI
  (`.github/workflows/test.yml`).
  The multi-instance hub transport is also tested with `httptest`: request identity checks, concurrent
  retries, durable response replay, long-poll continuation, cursor delivery, and the crash window where an
  accepted operation has an unknown outcome. Its private protocol is documented in
  [`docs/multi-instance.md`](docs/multi-instance.md). Hub replay records live in `bus/replay/`; completed
  response payloads are retained for 24 hours, then reduced to request-ID tombstones. Unresolved accepted
  records are retained for reconciliation and are never automatically executed again.
- **Errors.** Every operation of `agentbus` — each MCP tool and each CLI subcommand — returns its own error type,
  so the signature says what can go wrong:

  ```go
  func (s *Server) quizStart(question string) (string, *QuizStartError)
  ```

  `QuizStartError` is `OpError[QuizStartCause]`, and `QuizStartCause` is an enum of exactly the failures of
  `quiz_start` (`QuizStartMissingQuestion`, `QuizStartNotInTeam`, `QuizStartNotCoordinator`,
  `QuizStartAlreadyHeld`, `QuizStartResumedTeam`, `QuizStartStorage`); the compiler rejects any other cause.
  The enums are generated from `agentbus/errors.json` by `tools/errgen` (`make generate`; `make lint` fails when
  the generated file is stale). Each cause has a class (`InvalidArgument`, `NotFound`, `NotInTeam`,
  `Permission`, `Conflict`, `InvalidState`, `Storage`, `UnknownTool`, `Process`, `Unavailable`) that gives the
  sentinel for `errors.Is` and the exit status of CLI subcommands (2 … 11), so the `agent` script can tell "no such
  team" (3) from a storage failure (8). `Unavailable` (11) is for the multi-instance hub: it is not reached yet, and
  a forwarded operation whose reply was lost reports `OutcomeUnknown` (`InvalidState`), which is never retried
  automatically. Lower layers return concrete types too: `*StorageError` (state and log files,
  stdout), `*ValueError` (invalid enum values, timestamps) and `*ProcessError` (helper processes); operations
  convert them into their own causes explicitly. The plain `error` interface appears only where an interface
  requires it (`slog.Handler`, `json.Unmarshaler`, `flag.Value`) and at the MCP dispatcher. Tests check exact
  causes (`failsWith(t, QuizStartNotCoordinator)(m.quizStart("q"))`), plus `errors.Is` / `errors.As`.
- **Types.** Statuses, states, tiers, levels and message kinds are typed constants (`agentbus/types.go`,
  `agentbus/events.go`); unknown values are rejected when `state.json` is read. Timestamps are a `Timestamp`
  whose zero value means "has not happened yet". Every bus message is built from an event type that renders
  its own text.
- **Documentation rule.** `make lint` runs `tools/doccheck`, which fails on any constant, variable, type or struct
  field without a comment.
- **Integration tests** (`tests/integration/run.sh`, enabled with `AGENT_IT=1`) run one trivial task per
  configured CLI type (or `IT_AGENTS=...`, optionally a team with `IT_TEAM=a,b`) with the real CLIs in a
  separate `AGENTS_HOME` and a throw-away repository, and check that the agent used the bus, committed the
  expected file and finished with `STATUS: DONE`.

## License

MIT — see [LICENSE](LICENSE).
