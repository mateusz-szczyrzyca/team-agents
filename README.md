# team-agents

An orchestrator that runs many AI coding agents for you, alone or as teams, on your own machine. It works on
Linux today; support for macOS and for several machines is in progress.

Let agents of different vendors work side by side. Combine the paid subscriptions and API accounts you already have
to spread the token usage and to finish the work faster.

## What it does

- **Teams of mixed agents.** A team can mix agents of different vendors. A coordinator hands the items of a task list
  to its members, and every item comes back as one pull request.
- **Shared work.** The agents talk over a shared message bus. They reserve files, share findings in a notebook of the
  repository, and hand parts of their work to each other.
- **The right model for each job.** Agents come in tiers:
  - **Workers** are cheap models. They do the simple, mechanical steps that use up many tokens.
  - **Normal agents** plan a task, write the code and coordinate the team.
  - **Investigators** are the strongest models. They review the pull requests, give second opinions and split a
    task that is too big.
- **It keeps an eye on the work.** The daemon checks every round, runs the repository's tests, takes the reviews
  through their fixes, and can merge the result.
- **It recovers.** A run that hangs, fails or is cut by a restart goes on in a new session, briefed on what is done,
  or is handed to another agent. The work done so far is kept in git.
- **It spreads the load.** An agent runs one task at a time. Workers take turns across providers, and a task whose
  agent is busy is handed to a free agent of the same rank. You can configure the same model several times, under
  different accounts.
- **You stay in charge.** An agent with a question asks you in a file and waits. You answer with `agent answer` and
  it goes on. `agent status`, `agent log` and the notifications show what is going on.

Supported agent CLIs and vendors:

| CLI | Vendor |
|---|---|
| Claude Code | Anthropic |
| Codex CLI | OpenAI |
| Antigravity (`agy`) | Google (Gemini) |
| OpenCode | any API provider, such as OpenRouter or DeepInfra |

The orchestrator is written in Rust. Its strict compiler and type system act as a harness for the code that the
models write.

> **Security warning.** The agents run with the permission prompts and sandboxes of their CLIs **turned off**. They
> can run any command as their user, push to your repositories and use every credential that user can read. Run
> them on a dedicated machine or VM, as a dedicated unprivileged user, and read
> [Security](README.long.md#security) before you install.

## Quick start

You need Linux, `git`, `bash` 5 or newer, `make`, Rust (through `rustup`), Go, and the agent CLIs you want to use,
logged in for the agents' user. As that user, in a checkout of this repository:

```sh
cargo build --release --locked -p agent
(cd agentbus && go build -o ~/.local/bin/agentbus .)
install -m 755 target/release/agent ~/.local/bin/agent

export AGENTS_HOME=~/agents
mkdir -p -m 700 "$AGENTS_HOME"
cp config/agents.example.toml "$AGENTS_HOME/agents.toml"   # describe your agents here
cp -r prompts "$AGENTS_HOME/prompts"
```

Write `secrets.toml` (mode `0600`, with the GitHub token for the agents) as
[`docs/configuration.md`](docs/configuration.md#secretstoml) says. Then start the daemon and give it work:

```sh
agent daemon &                                   # or as a user service: config/systemd/agent-daemon.service
cd ~/src/my-repo
agent run claude-1 "Fix the failing date parser test"            # one agent, one task
agent run codex-1 claude-1 --coordinator codex-1 -f tasks.md     # a team working through a task list
agent status                                     # what is going on
agent answer <task>                              # answer an agent's questions
agent log --team <team>                          # what a team did, step by step
```

[`docs/task-list.md`](docs/task-list.md) describes the form of a task list.

## Roadmap

- A web dashboard.
- Stronger isolation of the agents (Landlock and more).
- Agents spread across several machines.
- macOS support.
- Agent accounts: the daemon reads each subscription's limits and plans around them.

## Learn more

- [README.long.md](README.long.md) is the full guide: security, every command and option, and what the daemon does.
- [`docs/configuration.md`](docs/configuration.md) is the reference of `agents.toml` and `secrets.toml`.
- [`docs/architecture.md`](docs/architecture.md) shows how the parts fit together.
- [`WhatsNew.md`](WhatsNew.md) says what each version brought.
- [`AGENTS.md`](AGENTS.md) holds the rules for developing the project. Most of it is written by its own agents.

## License

MIT — see [LICENSE](LICENSE).
