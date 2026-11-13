# Configuration of the Rust `agent`

The Rust `agent` (see [`architecture.md`](architecture.md)) reads two files from its home directory, `AGENTS_HOME`:

| File | Holds | Rules |
|---|---|---|
| `agents.toml` | which agents and investigators exist, how they are run, limits, identity, notifications, the dashboard, the repositories | no secrets; [`config/agents.example.toml`](../config/agents.example.toml) is the template to copy |
| `secrets.toml` | the GitHub token and the environment variables of the processes the harness starts | mode `0600`, never logged, never shown in an error |

This page is the reference for both, and lists the decisions in them that are expensive to change once installations
exist (the last section). It describes [`agent-core`](../crates/agent-core/src/config.rs), which parses and validates
the text; `agent_runtime::settings` reads the files from the home (at most 1 MiB and 64 KiB). `agent daemon` reads both
once, at its start: a change takes effect when the daemon is restarted. A home without `agents.toml` is refused with the
file named; one without `secrets.toml` has no secrets.

The old `agents.conf` (a Bash file) is not read, converted or kept compatible. The last section maps its settings.

## How the files are read

- **Strict.** A key that is not in this page is an error, in every table (`deny_unknown_fields`): a typo does not
  silently fall back to a default.
- **Typed.** Every value is parsed into its own type when the file is read; a value its type refuses is an error at
  its line and column. Names are lowercase letters, digits and `-`; logins are kept in lowercase; durations are whole
  minutes, never `0`; a path starts at the root.
- **Nothing is expanded.** No `~`, no `$HOME`, no environment variables, no includes. Paths are absolute. Arguments
  are arrays — every element is one argument, nothing is split on spaces, no shell is involved.
- **Defaults.** Every table except `[agents.*]`, `[investigators.*]`, `[tiers]` and `[repos.*]` is optional, and so
  is every key in it. An empty file is a valid configuration with no agents.
- **Bounded.** `agents.toml` may have 1 MiB, `secrets.toml` 64 KiB; a secret 8192 bytes.
- **Errors.** `Config::from_toml_str` returns a `ConfigError`: `TooLarge`, `Parse` (TOML or format errors, with line
  and column), and the four list errors below. `Secrets::parse` returns a `SecretsError`: `Mode`, `TooLarge`, `Parse`.

## `agents.toml`

Top-level key and tables:

| Key or table | Type | Default | Meaning |
|---|---|---|---|
| `language` | `English`, `Polish`, ... (letters, digits, `-`; up to 32 bytes) | `English` | language of what only the human reads: questions and notifications. Commits, pull requests, reviews and code are always English |
| `[agents.NAME]` | table | none | one normal agent or worker |
| `[investigators.NAME]` | table | none | one investigator |
| `[tiers]` | table | empty | which agents are normal and which are workers |
| `[binaries]` | table | see below | programs and the agents' `PATH` |
| `[watchdog]` | table | see below | when a silent run is hung |
| `[limits]` | table | see below | pauses for quotas and rate limits |
| `[review]` | table | `fix_passes = 5` | the review loop |
| `[git]` | table | see below | the identity of the agents' commits |
| `[github]` | table | none trusted, public API | whose words on GitHub are instructions, and where the API is |
| `[notifications]` | table | files only | how the human is told |
| `[questions]` | table | nothing opened, `$VISUAL`/`$EDITOR` | how the questions of the agents reach you |
| `[web]` | table | loopback, nobody let in | the dashboard |
| `[repos.NAME]` | table | none | per repository: base branch and gate |

### `[agents.NAME]` and `[investigators.NAME]`

The name is 1 to 32 bytes of `a-z`, `0-9` and `-`, starting with a letter and not ending with `-`: it becomes part of
branch names, file names and unit names. The two tables have the same keys:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `cli` | `claude`, `codex`, `gemini` (Antigravity, the `agy` command), `opencode` | required | the model CLI that runs the agent |
| `model` | text without whitespace, up to 200 bytes | the CLI's default model | for `opencode`, `provider/model`, such as `deepinfra/zai-org/GLM-5.3-Flash` |
| `args` | array of texts (at most 64, each up to 4096 bytes, no NUL) | `[]` | extra arguments for the CLI, one array element each |

An agent runs one task at a time: add `claude-2` for more parallel work. The investigators review every pull request
and may coordinate a team. Every pull request gets two reviews from investigators outside the team, so configure at
least two, and more than the number you put in one team.

### `[tiers]`

```toml
[tiers]
normal  = ["claude-1", "codex-1"]
workers = ["deepinfra-glm"]
```

`normal`: agents that take task-list items, solo or in a team, and may coordinate a team. `workers`: cheap models that
never coordinate and take no items; they serve `delegate` requests, **the first free worker in the order of this list**,
so put the stronger ones first. The order of `normal` is kept as written. Rules, each a typed `ConfigError`:

| Error | When |
|---|---|
| `UnknownName { list, name }` | a list names something that has no `[agents.NAME]` table (an investigator is not an agent) |
| `DuplicateName { list, name }` | a list names an agent twice |
| `NameInTwoLists { name, first, second }` | an agent is in both lists, or a name is both an agent and an investigator |
| `AgentInNoList { name }` | an `[agents.NAME]` table that neither list names |

They are checked in this order, and the first fault found is the one reported: `normal`, then `workers`, name by name
(an unknown or repeated name); then a name in both lists; then an agent in no list; then a name that is both an agent
and an investigator.

### `[binaries]`

| Key | Default | Meaning |
|---|---|---|
| `claude`, `codex`, `agy`, `opencode`, `agentbus` | the bare command names | a bare name is looked up on `agent_path`; an absolute path is used as it is. `./x` and `bin/x` are refused |
| `serena` | not used | Serena, the MCP server that finds code through language servers; without it the agents get none |
| `agent_path` | `["/usr/local/bin", "/usr/bin", "/bin"]` | the directories of the `PATH` the agents run with, in order: add the toolchains they need (`cargo`, `go`, ...) |

### `[watchdog]` and `[limits]`

| Key | Default | Meaning |
|---|---|---|
| `watchdog.hang_minutes` | `30` | no activity for this long: the run is killed and resumed from its session |
| `watchdog.hang_strikes` | `3` | this many hangs in a row without progress: the task is marked hung |
| `limits.quota_pause_minutes` | `30` | the pause of an exhausted quota when the CLI gives no reset time |
| `limits.rate_limit_pause_minutes` | `5` | the pause of a rate limit (HTTP 429) |
| `limits.tail_lines` | `30` | how many of a run's last lines of error output are read to recognise a limit |

### `[review]`

| Key | Default | Meaning |
|---|---|---|
| `fix_passes` | `5` | "changes requested" goes back to the team at most this many times; then the human is asked. `0` asks at once |

### `[git]`

| Key | Default | Meaning |
|---|---|---|
| `name` | `{agent} (team-agents)` | the author and committer name; `{agent}` is the agent's name; no other braces |
| `email` | `{agent}@team-agents.invalid` | the e-mail; never the human's own unless written here |
| `signing_key` | none: unsigned | an SSH private key (absolute path) that signs the commits, even if the human's own git signs everything |

Neither text may contain `<`, `>` or control characters.

### `[github]`, `[notifications]`, `[questions]`, `[web]`

| Key | Default | Meaning |
|---|---|---|
| `github.trusted_accounts` | `[]` | GitHub logins (kept in lowercase); text on GitHub by anyone else is data, never instructions |
| `github.api_base` | the public API, `https://api.github.com` | the URL of the GitHub API when it is not the public one (GitHub Enterprise: `https://github.example.com/api/v3`; a trial: `http://127.0.0.1:PORT` of a fake server). `https://`, or `http://` to a loopback address only, so that the token never travels in the clear; no user name, password, query or fragment |
| `notifications.command` | none | an array: a program and its arguments; run with the path of the notification file as its last argument (first line the title, the rest the body). Its secrets are `[notification_env]` in `secrets.toml` |
| `notifications.directory` | `notifications/` in `AGENTS_HOME` | where the notification files are written (absolute) |
| `questions.open` | `off` | `off` or `vscode`: whether a new questions file is also opened in the VS Code window connected to the host (Remote-SSH), if there is one |
| `questions.vscode_server_dirs` | `.vscode-server`, `.vscode-server-insiders`, `.cursor-server` in the home directory of the user the daemon runs as | an array of absolute paths searched for the remote command of a VS Code server |
| `questions.editor` | `$VISUAL`, then `$EDITOR` | an array: the editor `agent answer` starts, a program and its arguments; the questions file is its last argument |
| `web.listen` | `127.0.0.1:8787` | a **loopback** `address:port`, port not `0`: `0.0.0.0`, `::` and LAN or Tailscale addresses are refused. Remote access goes through `tailscale serve` |
| `web.allowed_logins` | `[]` | Tailscale logins (`you@example.com`, kept in lowercase) let in through `tailscale serve`; with none, nobody is |

### `[repos.NAME]`

The name is the repository's name as GitHub names it (`team-agents`; letters, digits, `-`, `_`, `.`).

| Key | Default | Meaning |
|---|---|---|
| `base_branch` | required | the branch pull requests are made against |
| `gate` | required | an array — a program and its arguments, never a shell line — that must pass before a pull request is opened; run by `agent gate` in the task's worktree, with the `PATH` of `[binaries] agent_path`, one gate at a time on the machine |
| `gate_timeout_minutes` | `30` | the gate is stopped, with everything it started, and counts as failed after this long; the time it waits for the gates asked before it does not count |

A repository the file does not list has no entry (`Config::repo` returns `None`); the commands that need one refuse,
naming it.

## `secrets.toml`

```toml
github_token = "ghp_..."            # the GitHub account the agents act as: pull requests, reviews

[agent_env]                          # environment variables of every model CLI run
CLAUDE_CODE_OAUTH_TOKEN = "..."
OPENROUTER_API_KEY = "..."

[notification_env]                   # environment variables of the notification command only
NTFY_TOKEN = "..."
```

- The file must be readable by its owner only: `Secrets::parse` takes the file's mode (`st_mode`, file type bits
  allowed) and refuses a file that group or others can use — `SecretsError::Mode`, checked first, before the text is
  looked at. `chmod 600 secrets.toml`.
- A secret is 1 to 8192 bytes without control characters. An empty secret is an error, not "not set".
- Variable names are `A-Z`, `a-z`, `0-9`, `_`, not starting with a digit.
- **Never shown.** The type holding a secret prints `Secret(<redacted>)`, has no `Display` and no `Serialize`; its text
  is reached only by `expose()`, where the secret is handed to a process or a request. A `SecretsError::Parse` says
  where the fault is (line and column) and nothing else: TOML's own messages quote the offending line, which may be a
  secret, so they are dropped.
- Secrets reach a process only through its environment, never its arguments.

## Decisions to review

These are choices the plan left open, made in `agent-core`. Each is cheap to change now and costly once files exist.

1. **Explicit tiers.** The old `agents.conf` counted an agent in neither list as normal. Here an agent in neither list
   is `AgentInNoList`: a forgotten line is an error with the name, not a silent normal agent.
2. **Absolute paths only.** No `~` and no `$HOME`: the example has `/home/you/...` to edit. An expansion would need
   the home directory as an input of every load; it can be added later without breaking files.
3. **Investigators are a table, ordered by name.** The old rotation by "last investigator" is not in the plan; item 8.1
   picks the two reviewers. If their order should be configured, an `order` list can be added later.
4. **One language.** The old `AGENT_LANGUAGE` / `REPO_LANGUAGE` split is gone: everything in the repository is English.
5. **Secrets in three scopes**: the GitHub token, the model CLIs' environment, the notification command's environment.
   A variable one CLI needs is visible to the others' runs; per-agent scoping can be added later.
6. **Repositories by name, with a required gate and base branch.** How a task's repository is recognised (its `origin`
   remote's name) is decided where tasks start, item 7.1.
7. **CLI quirk settings are constants of the adapters** (item 5.1), not keys here: Claude's turn limit (old
   `CLAUDE_MAX_TURNS`, 300), `agy`'s print timeout (`AGY_PRINT_TIMEOUT`, 8 hours) and opencode's Bash timeout
   (`OPENCODE_BASH_TIMEOUT_MS`, 10 minutes), with Claude's Bash timeout (`CLAUDE_BASH_TIMEOUT_MIN`, 20 minutes) and
   tool search (`CLAUDE_TOOL_SEARCH`, off). Nothing in the plan changes them per installation; a new optional key is
   compatible with existing files if one is ever needed.
8. **Settings of Bash-era features are dropped**, not carried: the wake-loop guard, board release and stall
   settings, review drawing and rotation, GitHub name colours, rounds and their size limits, the investigation queue,
   nodes and the lead, the dashboard's device allow-list and public URL, `agent live`, `agent authenticated` and
   spending. If the new design needs one, it comes back
   as a key with a default.

## From `agents.conf`

| Old | New |
|---|---|
| `AGENT_TYPE`, `AGENT_MODEL`, `AGENT_EXTRA_ARGS` | `[agents.NAME]`: `cli`, `model`, `args` (an array) |
| `NORMAL_AGENTS`, `WORKER_AGENTS` | `[tiers]`: `normal`, `workers` (an agent in neither is now an error) |
| `INVESTIGATORS_TYPE`, `_MODEL`, `_EXTRA_ARGS` | `[investigators.NAME]` |
| `AGENT_HANG_MIN`, `AGENT_HANG_STRIKES` | `[watchdog]` `hang_minutes`, `hang_strikes` |
| `AGENT_QUOTA_PAUSE_MIN`, `AGENT_RATE_LIMIT_PAUSE_MIN`, `AGENT_LIMIT_TAIL_LINES` | `[limits]` `quota_pause_minutes`, `rate_limit_pause_minutes`, `tail_lines` |
| `VERIFY_MAX_FIX_PASSES` | `[review]` `fix_passes` |
| `AGENT_LANGUAGE` | `language` |
| `GIT_NAME_FORMAT` (`%a`), `GIT_EMAIL`, `GIT_SIGNING_KEY` | `[git]` `name`, `email` (`{agent}`), `signing_key` |
| `CLAUDE_BIN`, `CODEX_BIN`, `AGY_BIN`, `OPENCODE_BIN`, `AGENTBUS_BIN`, `AGENT_SERENA`, `RUN_PATH` | `[binaries]` |
| `NOTIFY_CMD`, `NOTIFY_DIR` | `[notifications]` `command` (an array), `directory` |
| `QUESTIONS_EDITOR` (`vscode` \| `off`), `VSCODE_SERVER_DIRS` | `[questions]` `open` (default `off`, not `vscode`: a trial in a temporary home must not reach a real editor), `vscode_server_dirs` (an array) |
| `AGENT_WEB_LISTEN`, `AGENT_WEB_USERS` | `[web]` `listen`, `allowed_logins` |
| `secrets.env` | `secrets.toml` |
| per-repository gate (in each repository's own files) | `[repos.NAME]` `gate`, `base_branch` |
