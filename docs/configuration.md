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
| `[workers]` | table | `tries = 2`, `seconds = 0` | what the harness does with the result of a worker that served a request |
| `[build]` | table | `incremental = false` | Cargo cache policy for managed agent CLIs and gates |
| `[binaries]` | table | see below | programs and the agents' `PATH` |
| `[watchdog]` | table | see below | when a silent run is hung |
| `[limits]` | table | see below | pauses for quotas and rate limits |
| `[review]` | table | `fix_passes = 5`, `check_passes = 2`, `replace_after_minutes = 15` | the review loop, the checks before it and the reviewers that cannot work |
| `[split]` | table | `part_limit = 1500`, `corrections = 2`, `unit = "lines"` | automatic coordinator proposal policy |
| `[plan]` | table | `roles = ["solo", "member", "coordinator"]`, `bounces = 3` | whose runs keep a plan of their task, and how often one goes back for it |
| `[replacement]` | table | `new_sessions = 1`, `replacements = 1`, `mode = "auto"`, `agents = []` | how a task whose runs went bad is kept going: in a new session, then by another agent, by the team's lead, or by the human |
| `[git]` | table | see below | the identity of the agents' commits |
| `[github]` | table | none trusted, public API | whose words on GitHub are instructions, and where the API is |
| `[notifications]` | table | files only | how the human is told |
| `[questions]` | table | nothing opened, `$VISUAL`/`$EDITOR` | how the questions of the agents reach you |
| `[history]` | table | `enabled = true`, `max_text_bytes = 1048576`, `keep_days = 0` | the history database (`history.db`) |
| `[log]` | table | `level = "info"`, `keep_days = 0` | the daemon's own log (`stderr` and `history.db`) |
| `[snapshot]` | table | `warn_mb = 200` | the snapshots of worktrees |
| `[merge]` | table | `conflict_passes = 3` | what happens when GitHub refuses an approved pull request for a conflict |
| `[publish]` | table | `tries = 3` | what happens when a branch cannot be published to GitHub |
| `[web]` | table | loopback, nobody let in | the dashboard |
| `[repos.NAME]` | table | none | per repository: base branch, gate and the checks before a review |

### `[agents.NAME]` and `[investigators.NAME]`

The name is 1 to 32 bytes of `a-z`, `0-9` and `-`, starting with a letter and not ending with `-`: it becomes part of
branch names, file names and unit names. The two tables have the same keys:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `cli` | `claude`, `codex`, `gemini` (Antigravity, the `agy` command), `opencode` | required | the model CLI that runs the agent |
| `model` | text without whitespace, up to 200 bytes | the CLI's default model | for `opencode`, `provider/model`, such as `deepinfra/zai-org/GLM-5.3-Flash` |
| `args` | array of texts (at most 64, each up to 4096 bytes, no NUL) | `[]` | extra arguments for the CLI, one array element each |

An agent runs one task at a time: add `claude-2` for more parallel work. The investigators review every pull request
and may coordinate a team. Every pull request gets `[review] min_first_full_reviews` reviews (default two) from investigators outside the team.
Configure that many outside each team.

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

### `[build]`

`incremental` is an optional boolean, default `false`, including in configurations without `[build]`.
The daemon sets `CARGO_INCREMENTAL=0` for every agent CLI and gate to avoid large worktree caches;
`true` sets `CARGO_INCREMENTAL=1`. This setting wins over inherited or secret environment values and does
not affect builds started by hand. Copy [`agents.example.toml`](../config/agents.example.toml) for an example.

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
| `watchdog.asleep_minutes` | `5` | every task of a team sleeps this long with an item unfinished: the lead is woken and told that nobody works; after two such wake-ups the human is told |
| `limits.max_turns` | `300` | positive per-run turn budget, passed as `--max-turns` to Claude Code; CLIs without a turn limit ignore it |
| `limits.turn_limit_resumes` | `3` | maximum consecutive automatic resumes after a turn limit; `0` fails on the first exhaustion |
| `limits.quota_pause_minutes` | `30` | the pause of an exhausted quota when the CLI gives no reset time |
| `limits.rate_limit_pause_minutes` | `5` | the pause of a rate limit (HTTP 429) |
| `limits.idle_team_minutes` | `120` | minutes without a run or new branch commit before automatic team removal; `0` disables it |
| `limits.idle_team_restarts` | `1` | reserved replacement budget; unused in this removal-only release |
| `limits.tail_lines` | `30` | how many of a run's last lines of error output are read to recognise a limit |
| `limits.wake_ups_per_message` | `2` | runs a task is woken for one unread message; after that many that end without the message read, the task is not woken for it again, a `bus_down` warning names the task, the message and the count, and the message waits for the task's next run; `0` never wakes a task for a message |
| `limits.runs_per_agent` | `1` | runs an agent may have going at once over all its teams and solo tasks, for normal agents and investigators; a task queued for an agent at its limit waits, queued, until the run that holds the agent ends (`agent status` says whom it waits for and where); `0` is no limit, as before this key |
| `limits.runs_per_worker` | `1` | the same for the agents of `[tiers] workers`; `0` is no limit |

A waiting task is never failed and is not handed to another agent by this limit: it starts when its agent is free. Waits
of a task that is running, of a task that asks the human and of a review are not runs and do not count. A reviewer's run
counts as a run of its investigator. Existing installations get the limit of `1` when the key is absent: an agent that
ran two tasks at once before now runs them one after the other. Set `runs_per_agent = 0` to keep the old behaviour.

A turn-limit ending queues the task and resumes the same session with a short continuation prompt. With the default
`turn_limit_resumes = 3`, the fourth consecutive exhaustion is the end of the retries of that session: the task is started
in a new session when [`[replacement] new_sessions`](#replacement) allows it, and fails with one notification otherwise.
Every other run ending resets the count (resuming a failed task with `agent resume` continues in the same session without
resetting the count until a run ends in another way). The count is derived from saved runs and survives daemon restarts.
Provider quota exhaustion takes precedence over a turn limit and uses the quota pause instead.

Idle teams are checked at startup and once per minute. Activity comes from saved run starts and ends, and local
branch/worktree commits. The interval starts at the later of recorded activity and current daemon startup:
time while the daemon was stopped does not count, and old teams get a full interval after restart. Runs and active checks,
human pause/stop, provider quota/rate-limit reset waits and accepted delivery claims awaiting another team's
pull-request merge prevent removal. Tasks queued for an agent and rounds awaiting a free investigator also
prevent removal, including queued provider retries after a daemon restart. An unanswered human question does not. A removed team is **not restarted
in this release**; automatic replacement follows in a separate part.

Automatic removal uses the safeguards of `agent remove`: unmerged work is bundled before deletion, and the
notification names the reason, bundle and freed bytes. A refusal keeps the team and notifies once with its reason
and branches. While the team remains idle, removal is retried once a minute without repeating that notice.
Repairing the cause allows a later scan to remove the team without new task activity.
An enabled but unavailable history blocks automatic deletion; `[history] enabled = false` explicitly opts out
of the history requirement. Set `idle_team_minutes = 0` before upgrading to retain teams indefinitely.

### `[review]`

| Key | Default | Meaning |
|---|---|---|
| `min_first_full_reviews` | `2` | reviews per pass; explicit values below two or above the configured investigator count are refused; team creation requires this many outside investigators |
| `allow_second_opinion_on_blocker` | `true` | permission for one second opinion on an open BLOCKER; omission means true. Stored for the later daemon activation; requests are still refused in this release |
| `fix_passes` | `5` | "changes requested" goes back to the team at most this many times; then the human is asked. `0` asks at once |
| `check_passes` | `2` | the checks before a review ([`[repos.NAME.checks]`](#reposnamechecks)) send a round back to the team at most this many times; then the human is asked. `0` asks at once |
| `replace_after_minutes` | `15` | a reviewer that is paused this long with its review still to come is replaced by another free investigator; one whose task failed or was stopped is replaced at once. With no investigator free the human is told |

### `[plan]`

A plan is the task's own record of what it is for and what is left, written by the agent at `.agent/plan.md` in its
worktree ([`docs/plan.md`](plan.md)). The daemon reads it at the end of every run and keeps every new version in the
history; a run of a role here that ends `STATUS: DONE` against its plan is sent back with the reason.

| Key | Default | Meaning |
|---|---|---|
| `roles` | `["solo", "member", "coordinator"]` | the roles whose runs are sent back for their plan. Only these three: a worker or a reviewer keeps no plan, and naming one is refused (`PlanRole`), as is a role named twice (`PlanRoleTwice`). A file without `[plan]` has these roles |
| `bounces` | `3` | how many times a task's runs go back for its plan before the human is asked once, in a questions file. `0` never sends a run back; its plan is still recorded |
| `check_seconds` | `120` | timeout in seconds for executing a done step's command check; `0` disables command checks, keeping them declared |
| `check_refuse` | `["agent gate", "make gate"]` | command prefixes refused as step checks so that recursive gate runs are never executed; refused steps remain declared |

A task created before the daemon first ran with plans keeps none of this: its plan, if it writes one, is recorded, and
its runs are never sent back. Done steps are nevertheless judged for all roles and outcomes, even with
`bounces = 0`: existence checks and command checks run, while no-check and reviewer steps remain declared in the daemon. The daemon runs the command checks of done steps (`check_seconds` is their time limit); a step check that fails is recorded as a `mismatch` and sends a `STATUS: DONE` run of a role in `roles` back within `bounces` (or asks the human once after the limit), while a step check matching `check_refuse` stays `declared` and is told once in the task's next prompt.
Existing configurations need no changes.

### `[workers]`

What the harness does with the result of a worker — a model of `[tiers] workers` — that served a `delegate` request.
A request is given to one worker first. Its result is wrong when the request's check fails, the worker ends otherwise
than `STATUS: DONE`, it says it cannot, it changed a file outside the request's `files`, or its run passed `seconds`;
the request then goes once more to a free worker of another CLI (the same CLI only when no other is free), and a second
wrong result returns the step to the requester, who does it itself. A request that names no path to write asks for
facts only: its result goes to the requester as before, and its check, when it has one, is run all the same. Both keys
apply to a result of a run that ended `STATUS: DONE`; a run that ends otherwise is answered to the requester as failed at
once, as before (its second try comes with a later version). The second worker is told who served the request before it and
why that result did not serve; the requester is told the request went again, and, after a second wrong result, gets the
reasons of both. A restart of the daemon while the request waits for the second worker does not lose it.

| Key | Default | Meaning |
|---|---|---|
| `tries` | `2` | how many tries a request gets, its first try included. `1` never gives it to a second worker; `0` is refused (`WorkersTriesZero`). A request gets one second try at most: a value above `2` behaves as `2` |
| `seconds` | `0` | how long a worker's run may last, in seconds; a run past it is a wrong result. `0` has no limit |

A file without `[workers]` gets these defaults, so existing configurations need no change.

The keys are read and checked at load. `seconds` is applied: a worker's run that lasted longer is a wrong result, and the
step is returned to the requester. `tries` is applied with the second try of a request, in the next version: until then a
wrong result is returned to the requester at once, whatever `tries` says.

### `[split]`

These keys configure automatic coordinator proposal decisions. The daemon records the complete proposal before
deciding and retains its corrections and human question across restarts. The
[explicit human verdict](task-list.md#approved-split-plans-core-format) remains accepted.

| Key | Default | Meaning |
|---|---|---|
| `part_limit` | `1500` | maximum estimate when its unit matches the repository unit — case, surrounding spaces and a plural `s` are ignored (`Lines`, `line`); `0` disables this check. Other units are retained without conversion |
| `corrections` | `2` | send-backs allowed before a faulty proposal needs a human decision; `0` asks on the first fault. A valid proposal remains eligible after the budget is exhausted |

| `unit` | `"lines"` | nonblank repository unit used for the size comparison, such as `pages` or `images`; no conversion between units |

`part_limit` and `corrections` are nonnegative integers through `4294967295`. Missing tables or keys use the defaults; unknown keys,
negative values, fractional values and strings are rejected. Scope changes always require human approval once
the proposal satisfies the structural checks. Acceptance criteria are required for every kind of deliverable.

### `[replacement]`

A run may end without a `STATUS:` line or hang. The task is asked again in the same session a few times — twice for a run
without a status, `watchdog.hang_strikes` times for a hang, `limits.turn_limit_resumes` times for an
exhausted turn budget. After that the session is itself the trouble: the model may keep writing its tool calls as text, or
hang on the work it carries. The task is then started in a new session of the same agent, with a briefing built from facts:
the task's own prompt, the commits on its branch since the base, the files it left uncommitted (a snapshot of the worktree
is taken first), its plan of steps when it has one, the messages of its team it had not read, and why it is started anew.
The briefing tells it to go on from that state, not to start over and not to undo the commits. A solo agent, a coordinator
and a member are started anew; a worker and a reviewer are not.

| Key | Default | Meaning |
|---|---|---|
| `new_sessions` | `1` | how many new sessions of its agent a task is started in after its runs went bad; `0` turns them off; at most `10`. The count is kept in the state, so it survives a restart of the daemon |
| `agents` | `[]` | the agents that may take a task over when its new session went bad too, in the order to try them: names from `[agents.NAME]` or `[investigators.NAME]`. A name that is not configured, or given twice, is refused. With an empty list, `mode = "auto"` gives a member's work to the team's lead, as `lead` does |
| `mode` | `"auto"` | how far the harness goes without you: `auto` — another agent of `agents`, then, for a member, the team's lead, then you; `lead` — a member's work goes to the team's lead at once, other tasks fail; `off` — the task fails at once, as before `agents` existed |
| `replacements` | `1` | how many times a task is taken over by another agent; `0` takes none over; at most `10`. The count is in the state |
| `rejections` | `5` | the limit of times the lead of a team may return the work of an item to the member, which `agent status` shows against the count (`returned 3 of 5`); `0` is no limit; at most `10`. The count is kept with the item in the state and survives a restart |
| `busy_after` | `300` | seconds a queued task waits for its agent, which runs another task at its `[limits] runs_per_agent` limit, before a free agent of `agents` (of the same or a higher tier) takes it over; `0` never hands a task over for this reason. Only the agents listed in `agents` are candidates: an empty `agents` list means no hand-over for a busy agent, and the task waits on. The moment the wait began is kept in the state, so a restart of the daemon keeps the time waited |

A busy hand-over uses only the candidates listed in `agents`, the tier rules and the `replacements` count as above: the agent taken must be of the same or a higher
place than the one it replaces, not in the task's team, not in a run, not with a queued task of its own and not paused for a
quota. With an empty `agents` list, or when no agent of `agents` fits, no busy hand-over takes place and the task keeps waiting for its own agent. Of the agents that fit, the one whose CLI runs the fewest runs now is taken, and among equals the first of `agents`.
The task keeps its worktree and branch, its new holder's first run is briefed with the reason `busy`, the bus tells the team who holds the item now, and the first holder
is not woken for it. Only a queued task waiting for its busy agent counts: a task that is running, a task that waits for an answer of the human or for a review, a coordinator's task while one of its members runs, a reviewer's task, and a worker's task are never handed over this way.

The same `mode` and ordered `agents` list apply when the next part of a split needs a new holder because
its previous member stopped, failed or handed over the task. `auto` tries qualified configured replacements,
then the lead; `lead` tries the lead; `off` asks you. If no eligible holder can take the part, the harness asks
you once rather than leaving the part waiting silently. A replacement must meet both the part's required tier
and the previous holder's tier; a busy eligible agent waits until it is available.

When the new session goes bad too — after its own retries — the task is given to the first agent of `agents` that is not in
the task's team, not in the middle of a run, and of the same or a higher place than the agent it replaces: an investigator
above a normal agent, which is above a worker. It works in the same worktree, on the same branch and on the same task, and
its first run is briefed like a new session (see above), with retries of its own; a takeover is not counted among
`new_sessions`. On the team's bus the task keeps the name of the agent it was first given to: the new agent's runs act
under that name, so the messages to it, its wake-ups and a lead's rights stay with the task, and its branch keeps that
name too. The state keeps who held the task before (`past_holders`), and the daemon's log the choice
(`decision_taken` with `decision` `takeover`).

`rejections` counts another way a member's work comes back: the lead of a team, in its final message, sends an item back
instead of taking the branch. A rejection is a line `RETURNED: <item>` of that message, beside its `STATUS:` line — a
lead of an older prompt that writes no such line never counts one. It counts only when the harness sees that the lead
read the member's work and sent it back: the item is held by a member's task whose last run before the lead's run ended
`DONE`, the member is started again for it (by the lead's message, or at its next run), and the item did not move on
(it is still in progress). The reason of a rejection is what the lead told the member directly during the run that returned the work, read from the bus (the newest message of the bus when the lead's run began marks where the run's messages start).
A lead returns one part of a split item with `RETURNED: 1.1a`; every part has its own count.
A line repeated before the member ran again counts once. An answer to the member's question, a forwarded answer of
the human, a bounce of the task's plan, a refused gate and the checks before a review are not rejections: they have
counters of their own. The count is kept with the item (`rejections`, see [`state.md`](state.md)) and survives a restart;
`agent status` shows it on the task's line as `returned 3 of 5`, and on the round's line when it is one rejection from
the limit. `0` is no limit. What `rejections` does at the limit — the item goes to an equivalent agent, told the reasons — belongs to
the version that gives the item to that agent; this version counts and shows.

With no agent to take it, a member's task is stopped and the team's lead is told on the bus that the work is its own now:
whose it was, its branch and its worktree, with the briefing of what the member's runs left — the snapshot of its
worktree, the commits, the uncommitted files, its plan and the messages it had not read. The lead integrates the branch,
finishes the work and ends `DONE`, and the item is published as the lead's. The message goes out before the daemon ends,
also when it is stopped during the member's last retry. A solo task and a coordinator's have no lead: they fail and you
are told, as before.

After the last step — when the agent that took a task over goes bad too, after its own retries, or a lead given a
member's work goes bad too — you are asked once, in the task's next questions file (`harness/run/notice-last-step.md`):
`a` starts the task once more in a new session of its agent, briefed; `b`, `ok` for the recommendation, and any other
answer, free text included, stop it, and a member's work then goes to its lead. A task that goes bad again after its question fails as before. Mode `off`
asks nothing. Each hand-over is logged as `task_handed_over` (see [log.md](log.md)).

The new session works in the same worktree, on the same branch and on the same task. The state keeps the restart and its
reason, the history the snapshot, the daemon's log the decision (`decision_taken` with `decision` `ladder`), and
`agent status` says `in a new session` on the task's line, and `, for AGENT` when another agent took the task over. A task that works for no open round (a state of an older
version) is started in a new session without the briefing.

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

### `[history]`

| Key | Default | Meaning |
|---|---|---|
| `history.enabled` | `true` | whether the daemon writes `history.db` in `AGENTS_HOME` ([`history.md`](history.md)); `false` records nothing and loses nothing |
| `history.max_text_bytes` | `1048576` (1 MiB) | the longest text one field of the history keeps, in bytes, `1` to `268435456`: a longer text (a prompt, a final message) is cut at a character boundary and the record says which field was cut |
| `history.keep_days` | `0` | days to retain closed rounds; `0` keeps all; an open round is never deleted |

A history that cannot be written — the file is a directory, the disk is full, the database is of a newer version — never stops
or waits for storage: the daemon says so once on its standard error for each cause, counts the records it could not keep, and
`agent status` shows the count.

Git history capture has a fixed overall `2`-second budget at each run boundary, with at most `1` additional second for
process cleanup. The capture exception and configured-secret redaction are documented in [`history.md`](history.md).

### `[log]`

| Key | Default | Meaning |
|---|---|---|
| `log.level` | `info` | `error`, `warn`, `info`, or `debug`: the lowest severity of the daemon's own log events written to standard error and to `history.db` ([`log.md`](log.md)) |
| `log.keep_days` | `0` | days to retain log rows of no round; `0` keeps all; scoped rows follow their round's retention |

Both retention values are non-negative integers no greater than `4294967295`. The daemon runs retention at startup
and once per `24` hours of its clock. Cutoffs are exclusive: a round closed exactly at the cutoff stays until a later
pass. Retention removes history and gate output, not worktrees or the authoritative state. See [`log.md`](log.md#retention).

### `[repos.NAME]`

The name is the repository's name as GitHub names it (`team-agents`; letters, digits, `-`, `_`, `.`).

| Key | Default | Meaning |
|---|---|---|
| `base_branch` | required | the branch pull requests are made against |
| `gate` | required | an array — a program and its arguments, never a shell line — that must pass before a pull request is opened; run by `agent gate` in the task's worktree, with the `PATH` of `[binaries] agent_path`, one gate at a time on the machine |
| `gate_timeout_minutes` | `30` | the gate is stopped, with everything it started, and counts as failed after this long; the time it waits for the gates asked before it does not count |

A repository the file does not list has no entry (`Config::repo` returns `None`); the commands that need one refuse,
naming it.

### `[repos.NAME.checks]`

What the checks before a review need to know about a repository. Without the table a round goes straight to review; with
it, the daemon checks the round's own diff — from the merge base with `base_branch` to the head — and its commits through
git, with no model, before the head goes to the reviewers: the first time and after every fix pass. Nothing in the harness
names a repository or its layout: these three lists are all it has.

| Key | Default | Meaning |
|---|---|---|
| `test_paths` | `[]` | the test files. A round may add lines to one; changing one that exists other than by added lines (its mode too), removing it or renaming it is for the human to allow |
| `code_paths` | `[]` | the production code. A round that changes it has to add a line to a test file (or say in a commit `Test-First: not possible — <reason>`) |
| `protected_paths` | `[]` | the files that judge a round: the gate's scripts, lint and CI configuration, the repository's rules. Changing one is for the human to allow |
| `commit_guard` | `true` | `false` turns off the early refusal of a commit that changes a frozen test (below); the checks before a review stay on |

Each list is an array of at most 256 path patterns. A **path pattern** is a glob over paths relative to the root of the
repository, written with `/`:

- `*` matches any characters of one name and `?` one character of it; neither crosses a `/`;
- a segment `**` matches any number of directories (at the end of a pattern, at least one); at most four to a pattern;
- a pattern that ends in `/` is a directory and matches everything below it: `tests/` is `tests/**`;
- a pattern without a `/` is a file name and matches it at any depth: `AGENTS.md`, `*_test.go`;
- any other pattern is anchored at the root: `crates/*/src/**`.

A pattern is not empty, not absolute, has no `..` segment, no control character and none of `\ [ ] { }`, and uses `**`
only as a whole segment. A file that matches `test_paths` is a test even when `code_paths` matches it too; a test module
inside a source file is not a test file (the reviewers look at those). How the rules use the lists is in
[`architecture.md`](architecture.md#round-checks-before-the-review).

The same lists drive the **early refusal of a commit** (`agent guard-commit`): the `git` of every run of an agent is given a
`commit-msg` hook through its environment (`core.hooksPath` in `GIT_CONFIG_*`; nothing is written into the repository), which
refuses a commit that takes lines out of a test its `RED:` commit froze unless it is a `TEST-FIX:` commit of test files only,
and tells the agent to make that commit. The hook runs the repository's own hook of the same name afterwards and keeps its
other hooks, and acts for the repository it was made for only. It judges the files of `test_paths` only: without the table,
with `test_paths = []` or with `commit_guard = false`, every commit is allowed. Its refusal has the exit status 86 of
`agent guard-commit`; any other status lets the commit through.

### `[disk]`

Thresholds for filesystem free space in `AGENTS_HOME`.

| Key | Default | Meaning |
|---|---|---|
| `low_free_gb` | `100` | below this, a notification is sent once with largest worktrees, and `agent status` displays free space |
| `min_free_gb` | `20` | below this, `agent run` is refused and open rounds wait for disk space |

### `[snapshot]`

The snapshots of worktrees ([`architecture.md`](architecture.md), "Snapshots"). A snapshot is never refused, and nothing
prunes the snapshots yet: their commits stay under `refs/agents/snapshots/` in the repository until someone removes them.

| Key | Default | Meaning |
|---|---|---|
| `warn_mb` | `200` | a snapshot bigger than this megabytes is taken and logged with a warning that says its size |

### `[merge]`

What happens when GitHub refuses to merge an approved pull request because it conflicts with its base (a 405 that says the
pull request has merge conflicts, is not mergeable, or whose base branch was modified). The lead of the round is sent to
merge the base into its branch, the result is gated and pushed, and the merge is tried again. A change to files the task
does not allow (`Allow:`) is reviewed again before the merge. Every other refusal is told to you as before.

| Key | Default | Meaning |
|---|---|---|
| `conflict_passes` | `3` | lead passes per round to merge the base after a conflict; when they are used up, or the lead gives no result, the lead asks you in its questions file. `0` tells you at once, as before |

### `[publish]`

What happens when a branch cannot be published to GitHub (push failure, pull request creation or update error). The harness retries with backoff below `tries`, tells the lead once when the limit is reached so the lead can fix it, and asks the human once if publication still fails.

| Key | Default | Meaning |
|---|---|---|
| `tries` | `3` | how many times publication is tried before the lead is told once, and after that you are asked in a questions file. `0` asks you at once |

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
3. **Investigators are a table, ordered by name.** The old rotation by "last investigator" is not in the plan: the two reviewers
   of a pull request are drawn at random among the free investigators outside the team (item 8.1). If their order should be
   configured, an `order` list can be added later.
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
| `AGENT_PRECHECK_MAX` | `[review]` `check_passes` |
| the precheck's test, code and judging paths (in the script) | `[repos.NAME.checks]` `test_paths`, `code_paths`, `protected_paths` |
| `AGENT_LANGUAGE` | `language` |
| `GIT_NAME_FORMAT` (`%a`), `GIT_EMAIL`, `GIT_SIGNING_KEY` | `[git]` `name`, `email` (`{agent}`), `signing_key` |
| `CLAUDE_BIN`, `CODEX_BIN`, `AGY_BIN`, `OPENCODE_BIN`, `AGENTBUS_BIN`, `AGENT_SERENA`, `RUN_PATH` | `[binaries]` |
| `NOTIFY_CMD`, `NOTIFY_DIR` | `[notifications]` `command` (an array), `directory` |
| `QUESTIONS_EDITOR` (`vscode` \| `off`), `VSCODE_SERVER_DIRS` | `[questions]` `open` (default `off`, not `vscode`: a trial in a temporary home must not reach a real editor), `vscode_server_dirs` (an array) |
| `AGENT_WEB_LISTEN`, `AGENT_WEB_USERS` | `[web]` `listen`, `allowed_logins` |
| `secrets.env` | `secrets.toml` |
| per-repository gate (in each repository's own files) | `[repos.NAME]` `gate`, `base_branch` |
