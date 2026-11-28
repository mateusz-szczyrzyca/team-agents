# team-agents

**Run a team of AI coding agents on your own Linux machine. Mix models from Anthropic, OpenAI, Google and open-model
providers, let them check each other's work, and get pull requests that pass your tests.**

> **Work in progress.** team-agents changes every day and has not reached version 1.0.0. Commands, configuration and the
> files it keeps can change from one version to the next. Try it on a test repository and a spare machine before you
> trust it with a project that matters.

## Why it exists

AI agents can write good code. The hard part is to keep them working on a real project for weeks. team-agents
exists for one goal: **build your other software projects with many models, pay less per token, and stop tuning the
setup by hand every day.**

It solves these problems:

- **One vendor means one limit and one bill.** A single subscription hits its limit in the middle of a task, and a
  single API key charges for every token. team-agents puts the subscriptions you pay for (Claude, ChatGPT, Gemini)
  and pay-per-token APIs (OpenRouter, DeepInfra) in one pool. The daemon hands each job to a free agent of the right
  rank, so no single account carries the whole project.
- **Strong models doing cheap work.** Top models cost the most, and most coding steps don't need them. team-agents
  gives each job to the cheapest model that can do it. Workers make the mechanical edits, normal agents plan and write
  the code, and the strongest models review it. You pay top prices where judgment counts.
- **Agents that stall while nobody looks.** An agent hangs, waits on a prompt, loses its session or dies with a
  restart. The daemon watches every run. It resumes stalled work in a fresh session with a briefing on the work so far
  (a checkpoint), or hands it to another agent. The work lives in git, so nothing starts from zero.
- **Code that nobody checked.** Every round must pass your repository's tests and lints before it becomes a pull
  request. Reviewers from other vendors read it, the team fixes what they find, and in autopilot the daemon merges
  what they approve.
- **Agents in each other's way.** The agents share a message bus. They reserve files before they edit them, record
  what they learn in a notebook of the repository, and hand parts of the work to each other.
- **Endless tuning.** Many setups make you pick a model, rewrite prompts and retry runs for every task. With
  team-agents you describe your agents in one file and hand over a task list. The daemon picks the agent, plans the
  rounds and keeps the work moving. You answer the questions the agents can't settle.
- **Losing control.** An agent with a question writes it to a file and waits for your answer. Notifications can reach
  your phone, and `agent status` and `agent log` show every step.

## Built by its own agents

team-agents develops itself. Since October 2026 its own agents opened more than 220 of its 240+ merged pull
requests. They plan the rounds, write the code and the tests, review each other and fix what the reviewers find. An
AI assistant on duty watches the teams, answers routine questions and files the bugs it sees as issues.

The human, Mateusz Szczyrzyca, answers the questions that need a human decision and keeps the roadmap, which sets
the direction of the project. The [pull requests](https://github.com/mateusz-szczyrzyca/team-agents/pulls?q=is%3Apr)
and the [issues](https://github.com/mateusz-szczyrzyca/team-agents/issues) show the work as it happens.

## How it works

- One daemon (`agent daemon`) runs under `systemd --user`. It starts the agent CLIs, gives them their instructions
  and a shared bus (an MCP server), and records every run.
- `agent run` starts one agent on one task, or a team on a task list. Each task gets its own git worktree and
  branch, and each item of the list ends as one pull request.
- Agents come in three ranks: **workers** (cheap models for mechanical steps), **normal agents** (plan, code,
  coordinate) and **investigators** (the strongest models: reviews, second opinions, splitting a task that has
  grown past its size).
- `agent swap` lets you choose another configured agent for an unfinished task while keeping its worktree and branch.
- The orchestrator uses Rust. Its strict compiler and type system act as a harness for the code the models write.

Supported agent CLIs:

| CLI | Vendor |
|---|---|
| Claude Code | Anthropic |
| Codex CLI | OpenAI |
| Antigravity (`agy`) | Google (Gemini) |
| OpenCode | any API provider, such as OpenRouter or DeepInfra |

> **Security warning.** team-agents turns off the permission prompts and sandboxes of the agent CLIs. The agents
> can run any command as their user, push to your repositories and use every credential that user can read. Run
> them on a dedicated machine or VM, as a dedicated unprivileged user, and read
> [Security](README.long.md#security) before you install.

## Quick start

You need Linux, `git`, `bash` 5 or newer, `make`, Rust (through `rustup`), Go, and the agent CLIs you want to use,
signed in as the agents' user. As that user, in a checkout of this repository:

```sh
cargo build --release --locked -p agent
(cd agentbus && go build -o ~/.local/bin/agentbus .)
install -m 755 target/release/agent ~/.local/bin/agent

export AGENTS_HOME=~/agents
mkdir -p -m 700 "$AGENTS_HOME"
cp config/agents.example.toml "$AGENTS_HOME/agents.toml"   # list your agents in this file
cp -r prompts "$AGENTS_HOME/prompts"
```

Write `secrets.toml` (mode `0600`, with the GitHub token for the agents) as
[`docs/configuration.md`](docs/configuration.md#secretstoml) says. Start the daemon and give it work:

```sh
agent daemon &                                   # or as a user service: config/systemd/agent-daemon.service
cd ~/src/my-repo
agent run claude-1 "Fix the failing date parser test"            # one agent, one task
agent run codex-1 claude-1 --coordinator codex-1 -f tasks.md     # a team working through a task list
agent status                                     # what each agent is doing
agent answer <task>                              # answer an agent's questions
agent log --team <team>                          # what a team did, step by step
```

[`docs/task-list.md`](docs/task-list.md) describes the form of a task list.

## Roadmap

- Version 1.0.0: stable commands, configuration and files.
- Agent accounts: the daemon reads the limits of each subscription and plans around them (in progress).
- A web dashboard.
- Stronger isolation of the agents (Landlock and more).
- Agents spread across several machines.
- macOS support.

## Learn more

- [README.long.md](README.long.md) is the full guide: security, every command and option, and what the daemon does.
- [`docs/daemon-overview.md`](docs/daemon-overview.md) tells what the daemon watches and when it acts.
- [`docs/checkpoints.md`](docs/checkpoints.md) tells how the work survives a lost session.
- [`docs/configuration.md`](docs/configuration.md) is the reference of `agents.toml` and `secrets.toml`.
- [`docs/architecture.md`](docs/architecture.md) shows how the parts fit together.
- [`WhatsNew.md`](WhatsNew.md) says what each version brought.
- [`AGENTS.md`](AGENTS.md) holds the rules the agents follow when they work on this project.

## License

MIT — see [LICENSE](LICENSE).
