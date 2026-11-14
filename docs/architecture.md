# The Rust `agent` — architecture

`bin/agent` (Bash) is being rewritten in Rust, as a Cargo workspace in this repository, item by item of the plan
"Rewrite `bin/agent` in Rust". This file describes the workspace's layout, its process model and the rules every
item of the plan follows. Read it before you change anything under `crates/`; [`AGENTS.md`](../AGENTS.md) ("Rust")
points here.

Until the cutover (feature 10 of the plan) the old implementation runs the installation and the new one runs
nowhere but in tests and trials.

## Layout

```text
Cargo.toml                  the workspace: members, shared package fields, dependencies, lints
Cargo.lock                  committed; every cargo command of the gate runs --locked
rust-toolchain.toml         the pinned stable toolchain with rustfmt and clippy (rustup installs it)
rustfmt.toml, clippy.toml   formatting and the clippy settings of the lint baseline
crates/agent-core/          pure: identifiers, configuration, state, task list, decisions, protocol types
crates/agent-runtime/       effects: state store, processes, git, GitHub API, agentbus, prompts, notifications
crates/agent/               the `agent` binary: CLI commands and `agent daemon`
crates/agent-web/           the `agent-web` binary: dashboard (axum, htmx, server-sent events)
prompts/clarification.md
prompts/lang/<language>/...             human-facing texts in another language (English is the fallback)
prompts/agent/rust-developer.md
prompts/worker-agent/rust-developer.md
prompts/investigator/rust-developer.md
prompts/investigator/code-review.md
prompts/harness/<module>/<name>.md     texts the harness composes (run prompts, a team's life, questions files, notifications)
config/agents.example.toml, config/systemd/agent-daemon.service
docs/architecture.md, docs/configuration.md, docs/state.md, docs/task-list.md
```

The workspace exists with its four crates; the other files of this list arrive with the items that need them
(`docs/state.md` came with the state, the protocol and the questions files).

- **`agent-core`** decides and does no I/O: no `std::fs`, `std::process`, `std::net`, no `tokio`. What it needs from
  the world comes in as arguments — the clock ([`Clock`](../crates/agent-core/src/ids.rs)) and the random source
  (`rand::Rng`) included — so every decision is a pure function, tested without a file system.
- **`agent-runtime`** carries the decisions out: files, processes, git, the GitHub API, agentbus, prompts and
  notifications. It depends on `agent-core`, never the other way round.
- **`agent`** and **`agent-web`** are thin binaries on top: command lines, the daemon, the dashboard.

### The vocabulary (`agent-core`)

| Module | Types |
|---|---|
| `ids` | `AgentName`, `InvestigatorName` (normal and worker agents, investigators), `TeamId`, `TaskId`, `RunId` (`team-`/`task-`/`run-YYYYMMDD-HHMMSS-xxxx`, UTC, generated from an injected `Clock` and `Rng`), `ItemId` (`2.3`), `BranchName` (literal git branch names, with the bounds below) |
| `kinds` | `CliKind` (`claude`, `codex`, `gemini`, `opencode`), `Tier` (`normal`, `worker`, `investigator`), `Role` (`solo`, `coordinator`, `member`, `worker`, `reviewer`) |
| `status` | `RunOutcome` (`DONE`, `QUESTIONS`, `FAILED`, `SLEEPING`, `EXCLUDED`), `ReviewVerdict` (`APPROVE`, `REQUEST_CHANGES`), `TaskStatus` and `ItemStatus` with their transition tables |

Each is made only through its check (`FromStr`, `TryFrom<String>`, serde), its documentation lists what it refuses,
and its text form parses back to an equal value. Branch names follow Git's literal ref rules, with a
255-byte limit and an explicit refusal of `@` to avoid its revision shorthand; checkout-history expressions are
not expanded. A status changes only through `StatusMachine::transition_to`, which
refuses a change its table does not allow with a typed `InvalidTransition`:

```text
task:  queued -> running | paused | stopped
       running -> queued | waiting | questions | paused | stopped | failed | done
       waiting -> queued | paused | stopped | done
       questions, paused, failed -> queued | stopped
       stopped -> queued
       done (final)

item:  pending -> in_progress -> in_review -> approved -> done (final)
       in_progress -> pending (its member left)    in_review -> in_progress (a fix pass)
```

### The configuration and the task list (`agent-core`)

| Module | Types |
|---|---|
| `config` | `Config` (read from `agents.toml`) with its `AgentConfig`, `InvestigatorConfig`, `RepoConfig` and sections; `Secrets` (from `secrets.toml`) with `Secret`; the typed values (`Language`, `Program`, `AbsolutePath`, `CommandLine`, `Minutes`, `LoopbackAddr`, ...); `ConfigError`, `SecretsError`, `InvalidValue`. See [`configuration.md`](configuration.md) |
| `task_list` | `TaskList`, `Feature`, `Item` (title, description, expected result, checks) and `TaskListError`, whose every variant but two carries its line. See [`task-list.md`](task-list.md) |
| `repo_path` | `RepoPath` (a path inside a repository as git prints it), `PathPattern` (a glob over such paths: `*`, `?`, `**`, `dir/`, a bare name at any depth) and `PathPatterns` (at most 256 of them): what the `[repos.NAME.checks]` table lists. See [`configuration.md`](configuration.md) |

Both `Config` and `TaskList` take text, not paths: reading `agents.toml`, `secrets.toml` (and the file mode `Secrets::parse` needs) and a
`-f` task list is the runtime's job, so the core stays free of I/O.

### State, protocol and questions files (`agent-core`)

| Module | Types |
|---|---|
| `state` | `State` (with its schema version, version 1 migrated as it is read), `Team`, `Crew`, `Member`, `Participant`, `Run`, `RunEnd`, `Question`, `Round`, `RoundLife`, `RoundEnd`, `ItemProgress`, `ItemPhase`, `PullRequest`, `Review`, `FixPass`, `GateResult`, `GateRecord`, `GateTree`, `ChecksRecord` (the checks' record of a lead, a file of its task directory), `Timestamp`, `SessionId`, `CommitId`, `PrNumber`. See [`state.md`](state.md) |
| `protocol` | `Request`, `Command`, `Response`, `Reply`, `Failure`, `Event`, `EventKind`, `Frame`, with the protocol version and `ProtocolError`. JSON lines over the daemon's socket. See [`state.md`](state.md) |
| `questions` | `QuestionsFile`, `Entry`, `Verdict`: parsing and rendering of the questions file, the `ok` verdict, the `**ANSWERED**` marker. See [`state.md`](state.md) |

They take and give text too (`State::from_json`, `Request::from_line`, `QuestionsFile::parse`): the state store, the
socket and the files are the runtime's. `serde_json` is a dependency of `agent-core` for that; it does no I/O.
`strum` (derives only) gives the keyword enums their text, as `McpServerKind` does.

### The prompt catalog and the CLI contract (`agent-core`)

| Module | Types |
|---|---|
| `prompt` | `PromptId` (every prompt, with its `PromptSpec`: path under `prompts/`, `Audience`, required variables), `HarnessPrompt` (the texts of `harness/run/`) and `TeamPrompt` (the texts of `harness/team/`, a catalog of its own because a test pins `HarnessPrompt` to the files of `harness/run/`), `ChecksPrompt` (the texts of `harness/checks/`: the reasons of the checks before a review, the message that sends a round back, the questions for the human), `AutopilotPrompt` (the texts of `harness/pr/` and `harness/review/`: a pull request's title, description and notices, the reviews and the fix pass; what goes on GitHub is English only), `Template` (the strict `{{name}}` renderer), `Variables`, `VariableName` and their errors (`TemplateError`, `RenderError`, `PromptError`) |
| `cli` | `CliAdapter` (build a run, make a parser), `OutputParser` (lines to `ActivityEvent`s, then `RunOutput`), `RunRequest`, `RunSpec`, `RunFile`, `ActivityEvent`, `RunOutput`, `SessionId`, `ErrorTail`, `McpServerKind` |

A template renders `{{name}}`; `\{{` is a literal `{{`. A variable without a value, a value without a variable, an
unclosed placeholder and an invalid name are errors, and the catalog adds two: a text that uses a variable the
catalog does not list, and one that leaves a listed variable out. The text of a prompt is read by the runtime;
`PromptSpec::candidates` gives the files to try. Only a text for the human (`Audience::Human`) has a version per
language, `prompts/lang/<language, lowercase>/<path>`, with English as the fallback; what a model reads is English
only. The integration test `tests/prompt_catalog.rs` checks every catalog entry against the files of `prompts/`, and
its failures name the file.

An adapter is pure: `CliAdapter::build_run` turns a `RunRequest` into a `RunSpec` (program, arguments, environment,
the files to write first, what to keep out of git), the runtime starts it, feeds the CLI's stdout line by line into
an `OutputParser` and its stderr at the end, and gets the final message, the session ID, how the stream says the run
ended and the error tail.

### The adapters (`agent-runtime`)

`agent_runtime::cli` has one `CliAdapter` per CLI — `claude::ClaudeAdapter`, `codex::CodexAdapter`,
`agy::AgyAdapter` (`CliKind::Gemini`), `opencode::OpencodeAdapter` — and `adapter_for(CliKind)`, which the run engine
uses to pick one from the configuration. The module's documentation lists the rules all four follow; the ones the run
engine relies on:

| Per CLI | New run | Resumed run | MCP servers | Final message |
|---|---|---|---|---|
| Claude Code | `claude -p --output-format stream-json --verbose … --session-id ID` | `--resume ID` | `mcp.json` and `claude-settings.json` in the run's directory | the `result` event |
| Codex | `codex exec --json … -C DIR -c mcp_servers.…` | `… resume ID PROMPT` | `-c` settings (TOML) | the last message of a completed turn |
| Antigravity | `agy --output-format stream-json … --print-timeout 8h -p PROMPT` | `--conversation ID` | `.agents/mcp_config.json` in the worktree, kept out of git | the `result` event |
| opencode | `opencode run --standalone --format json --auto … -- PROMPT` | `--session ID` | `OPENCODE_CONFIG_CONTENT` (and a copy in the run's directory) | the last `text` |

- The request carries each MCP server's whole command line; the adapter adds only Serena's `--context <client>`
  (`claude-code`, `codex`, `antigravity`, `ide`). A server that is not in the request is not configured.
- The error tail is the CLI's stderr followed by the lines of the stream that report an error: the classifier of run
  outcomes reads only the tail, and opencode reports a provider's refusal on stdout.
- The quirk settings are constants of the adapters: Claude's turn limit (300) and Bash timeout (20 minutes), `agy`'s
  print timeout (8 hours), opencode's Bash timeout (10 minutes). Other flags, such as `--effort high`, are the
  agent's `args`.
- Not in the adapters, because they are not pure: laying down a `RunFile` (creating its directories), keeping
  `worktree_excludes` out of git, a prompt too long for one command-line argument, and opencode's `session export`
  for a resumed session whose result it did not print. They come with the run engine (6.1).
- Recorded output of each CLI is in `crates/agent-runtime/tests/fixtures/cli/`; its README says which files were
  recorded and which were written by hand.

### The decisions (`agent-core`)

| Module | What it decides |
|---|---|
| `decide::team` | `plan_team`: from the agents named, the configuration, the coordinator asked for (`--coordinator`, then the task list's `Coordinator:` line) and a random source, a `TeamPlan` — solo, or a team with its coordinator, members and workers — or a `TeamPlanError`: unknown agent, a worker as coordinator, a coordinator that was not named, only workers, no member besides the coordinator, and too many (or too few) investigators for two outside reviewers |
| `decide::assign` | `next_assignments`: which item goes to which member next — one item per member, the items of a feature in order (the next starts when the previous is merged, with `--no-merge` approved), features in parallel once the features they depend on are merged; `first_free_worker` for `delegate` |
| `decide::checks` | `assess`: from the facts of a round's diff (its files, the files whose mode changed, its commits with their parents, the lines it added, the description of its pull request) and the repository's `checks` lists, the `Issue`s — an existing test file that loses lines or is removed or renamed and a protected file that changes (only the human may allow them); a test of a `RED:` commit edited by a later commit unless a named `TEST-FIX:`; production code with no added test line and no `Test-First: not possible — <reason>`; an added line that skips a test, suppresses a lint or asks whether the code runs under test, in a file the description does not name (the built-in `markers` table per extension). See "Round checks before the review" |
| `decide::round` | `RoundState`, `RoundEvent` and `RoundState::transition`: member work, integration, gate green at the head, pull request published for that head, two reviews, merge or hand-over to the human, item done; "changes requested" starts a fix pass until `[review] fix_passes`, then the human is asked, and the answer grants one more pass. Each transition says what the daemon does next (`RoundAction`); `Checking` is the phase before the reviews are asked: what the issues of the checks come to — the reviews start, the round goes back to the team (at most `[review] check_passes` times) or the human is asked |
| `decide::round::rounds_of_run` | `rounds_of_run`: from the role of a task, why its run starts (`RunStart`: its own work, a request for a worker, a review, a continuation) and what the state says of its team's rounds (`TaskRounds`), the rounds the run works for. See "Rounds" |
| `decide::outcome` | `classify_outcome`: from the final message, the exit status, the error tail and who ended the run, the `RunEnd` to record — reported status, no status, quota exhausted (with the reset time when the CLI gives one), rate limited, hung, stopped — and the `Pause` it implies; `after_hang` for the watchdog's strikes. The CLIs' outputs are fixtures in `crates/agent-core/tests/fixtures/cli-outputs/` |

The daemon feeds these functions what it observed and carries out what they return; none of them touches a file, a process or
the network. The order of the checks and the rules each follows are in the documentation of its module.

### The run engine and the daemon (`agent-runtime`, `agent`)

`agent daemon` is the process model below, working: one daemon per home, the Unix socket, the run engine, recovery.

| Where | What |
|---|---|
| `agent_runtime::run` | `RunEngine::run`: one run of a model CLI, from its files to its classified end; touches no state (`run_held`: the same with a `WatchdogHold`, see "The gate"). The run's environment has `AGENTS_HOME`. `run::compose` makes its prompt: context, role, the rules of the tier, brief, item, checkpoint — or, for a resumed session, what interrupted it |
| `agent_runtime::{settings, harness_prompts, autopilot_prompts, checks_prompts, text_file, clock}` | `agents.toml` and `secrets.toml` read from the home; the second prompt catalog (`HarnessPrompt`, `prompts/harness/run/`: the run prompts and the notifications) and the fourth and fifth (`AutopilotPrompts`, `ChecksPrompts`); a bounded text file; the system clock |
| `agent::daemon` | `owner` (the state), `server` (the socket), `notifier`, `opener`, `settle` (what a run's end does), `recovery` (on start), `team` (the team's life as decisions), `owner/teams` (what carries them out), `questions` (the questions file) and `control` (what a control does to a task), with `owner/answers` and `owner/controls`, and the autopilot: `pr`, `review`, `merge` and `checks` (its decisions) with `owner/rounds` (what carries them out) |
| `agent::{client, commands}` | the socket client, `agent status`, `agent daemon`, `agent run`, `agent answer`, `stop`, `pause`, `unpause`, `resume`, `say` |

**Tasks** (CON-3, CON-4): the *owner* alone reads and changes the state; the *server* passes requests to it through a
bounded channel; the *notifier* delivers notifications off its path. Cancelling the shutdown token (SIGTERM, SIGINT)
stops them in order: the server stops accepting, the owner has its runs stopped and recorded, the notifier delivers what
is queued, the socket is removed; every task is joined. A state that cannot be saved ends the daemon with an error —
models must not work on with a state that is not kept — and `Restart=on-failure` of the unit starts it again.

**Start.** `agent daemon` needs `AGENTS_HOME` (no default before the cutover, so a trial cannot touch the live home),
`agents.toml` there and the prompt files (`prompts/` in the home, or `--prompts-dir DIR`); the files of both catalogs are
checked at start, every problem reported at once. It takes the lock of the home (a second daemon exits with "already
running"), recovers the state, makes the socket (in a private directory, mode `0600`, renamed into place) and serves. A trial:

```sh
export AGENTS_HOME=$(mktemp -d)
cp config/agents.example.toml "$AGENTS_HOME/agents.toml"
agent daemon --prompts-dir "$PWD/prompts" &    # logs on stderr
agent status                                   # no teams
agent daemon --prompts-dir "$PWD/prompts"      # agent: the agent daemon is already running: ...
kill -TERM %1                                  # ends the daemon and its runs
```

**Tasks and runs.** A task whose status is `queued` is started: the owner makes the run ready (`start.rs`: the CLI and
model from the configuration, the worktree `worktrees/<repo>/<task>`, the prompt, the MCP servers — `agentbus serve` for
the task, Serena when configured), records the run, sets the task `running` and gives the job to the engine. Creating the
worktree and queueing the task is `agent run` (the next item); until then a test or a trial writes them. What a run's
end does (`settle.rs`):

| The run ended | The task goes to | And |
|---|---|---|
| `STATUS: DONE` | `done` for a solo task and a review, `waiting` for a coordinator, member or worker | |
| `STATUS: QUESTIONS` | `questions`, when `tasks/<task>/questions-<number>.md` is there and parses | the questions are recorded and the human told; a missing or unreadable file fails the task and says why |
| `STATUS: FAILED`, no `STATUS:` line | `failed` | the human is told |
| `STATUS: SLEEPING` / `EXCLUDED` | `waiting` / `done` | |
| quota exhausted, rate limited | `paused` | woken at the time the CLI gave (plus a minute), else after the `[limits]` minutes; the human is told |
| hung | `queued`: resumed from its session | after `[watchdog] hang_strikes` hangs in a row without progress, `failed` and the human is told; a run that did something before it went silent starts the count again |
| the daemon shut down | `queued` | the run is recorded as ended with no status and no exit code |

**The watchdog** is the engine's: a run whose output files show no change for `[watchdog] hang_minutes` is stopped (its
whole process group, `SIGTERM` then `SIGKILL`) and classified as a hang unless the provider refused it. **A pause** is
kept in no field of the state: its wake-up is worked out again from the last run's end, so it survives a restart; the
daemon looks at the clock every 5 seconds, so a suspended machine does not stretch it. **Recovery**: a run that was going
when the daemon died is ended as "no status, no exit code" and its task queued, to resume from its session; paused tasks get
their wake-ups again, hung ones their strikes (progress is not saved: every hang in a row counts); a clock set back never
ends a run before it began. A run is a process group of its own: a daemon killed without `SIGTERM` (`SIGKILL`, the OOM
killer) leaves it going, and only the unit's `KillMode=mixed` ends it — outside systemd, end such runs before a restart,
or a second CLI resumes the same session.

**The socket** serves `status`, `show`, `subscribe`, `run`, `answer`, `stop`, `pause`, `unpause`, `resume` and `say` of
[`state.md`](state.md), and `gate` (below). At most 64 connections and 16 runs go at once; a line over 2 MiB ends its
connection; a subscriber that falls 1024 events behind is let go (it subscribes again and starts from the state).

**The home**: `agents.toml`, `secrets.toml` and `prompts/` (put there by the human); `state.json`, `daemon.lock`,
`agent.sock`; `tasks/<task>/activity.jsonl`, `tasks/<task>/runs/<run>/` (the output of a run and the files its CLI is
configured with, `prompt.md` for a prompt too long for a command line) and `tasks/<task>/questions-<number>.md`;
`tasks/<task>/gate-<n>.log` and `tasks/<task>/gate.json` (below); `tasks/<reviewer task>/review-pr<N>-pass<K>.md` (the reports of the reviews, below); `worktrees/<repo>/<task>/`; `notifications/`.

### Teams (`agent run` and the team's life)

`agent run <agents...> "tasks" | -f file [--coordinator NAME] [--no-merge]` names the repository by the `origin` of the
checkout it is run in (`[repos.NAME]` of the configuration), reads and parses the task list, and asks the daemon to
`run` it; then it asks for the team (`show`) and prints each task with its role, agent, worktree and item. Everything the
command can find wrong by itself (the words, the file, the parse, the repository) is refused before it looks for the daemon.
`run` is added to the clap command when the command line is parsed, not a variant of `Command`: the tests of the command
line match that enum exhaustively.

**Starting** (`team::form`, `owner/teams.rs`): the daemon plans the team (`plan_team`: the coordinator, the members, the
workers in the configured order; a worker as coordinator, only workers and too many investigators are refused before
anything is made), draws the IDs, gives the first items (one to each member, `next_assignments`), makes one worktree for
every task from the checkout's base branch — `worktrees/<repo>/<task>`, on the branch of its item
`agents/<agent>/<task>-<item>` or else `agents/<agent>/<task>` —, creates the team on the agentbus (a solo task has none)
and saves the state. A worktree or a bus that fails takes back what was made, and the request is refused.

**Every task starts at once**, the coordinator's first. The Go bus posts a message of the team's creation that every member
has unread, and sets a member to `waking` whenever it goes to sleep with something unread, so a member that has not read
it could not be kept asleep: the old harness started everybody too. A member with no item reads its messages and sleeps
(`STATUS: SLEEPING`); a list without items is one piece of work for the coordinator, who splits it by message or on the
board.

**The daemon looks at a team** after something happened to it (a run ended, a team started) and on its clock, every 5
seconds, because the Go bus wakes a sleeper by marking it `waking` and spawns nothing — the
`agentbus` wrapper configures no wake command — so only the daemon can tell, and because an item whose branch could not be started is tried again. Every look:

1. does what waits for the bus, oldest first (the *outbox*);
2. reconciles the bus with the tasks (`team::reconcile`): each member's state on the bus is derived from its task's status
   (`queued`/`running` → `active`, `waiting` → `sleeping`, `questions`/`paused` → `paused`, `stopped` → `stopped`,
   `done`/`failed` → `done`, an excluded member `excluded`); a waiting member the bus says is `waking` is queued with the
   wake text, and so is a member that goes to sleep while something waits for it (`team-set` says `waking`);
3. gives the pending items to the members that are free (`team::assignable`): the member's worktree gets
   a new branch from the base branch (`git switch -c`), the item becomes `in_progress`, a waiting member is queued with the
   item (the first run of a member has it in its prompt). An item whose branch cannot be started waits, with a warning in the
   log, and is tried again at the next look;
4. serves the requests for workers: the first free worker in the configured order (`first_free_worker`) takes the oldest
   queued request of its team (`helper-next`; a request of another team is held and put back before the next team is looked
   at) and runs it with the delegation text; when its run ends the request is closed with the
   worker's report (`helper-set`, without the status line) and the requester gets a `helper_result` message, which wakes it;
5. closes the team when its work is over (`team::is_finished`: every item done — for a list without items, the lead said
   `DONE` — and no task going, asking, paused or about to start): waiting tasks become `done`, the bus team is marked done
   and the human is notified, once.

When a member ends, the coordinator gets a message on the bus (`member_finished` with the item and the branch,
`member_failed`, `member_no_status`), which wakes it to integrate. A member's item stays `in_progress` until its pull
request is open ("Pull requests, reviews and the merge", below): the next item of a feature starts when the previous one is
merged, so a member that is done waits. A solo task whose item is under way waits as well, instead of ending (`done` is
final).

A bus that is down is not an error of the daemon: what could not be sent stays in the outbox and is tried again at the next
look, a team that cannot be read from the bus is skipped until the next look, and what the bus refuses for good (a request
that does not exist) is logged and dropped.

**Texts.** What a team's run is told beyond the working context lives in `prompts/harness/team/` (the catalog
`TeamPrompt`, loaded by `agent_runtime::team_prompts::TeamPrompts` and checked at start): the team and its branch for the
first run of a member, the new item of a member with a session, the wake-up, the request a worker serves, the messages for
the coordinator and for a requester (with the sentence of how a worker that could not do a request ended,
`worker-*.md`), and the notification that a team is finished.

**Known limits.** An agent that never reads its messages is woken again and again (the Bash wake-loop guard is not
ported). A request that is being served when the daemon stops is not returned. The bus gives the oldest request of every
team to whichever team's worker asks: a look at a team holds the requests of other teams while it reads the queue on, and
puts them back before the next team is looked at. The notices still in the outbox when the daemon stops are lost, and so is why a queued run starts (a wake-up, an item,
a request): it then starts as the plain continuation of its task. The owner's loop waits for each `agentbus` and `git`
call of a look at a team (an `agentbus` call for at most `15` s): a bus that hangs leaves the socket unanswered that
long.

Not done here, by item: the dashboard (9.1). A resumed agent's checkpoint is not read from the agentbus yet. opencode's `session export` for a
resumed session whose result it did not print is not done.

### Pull requests, reviews and the merge (the autopilot)

The default `--autopilot` loop, in the owner (`owner/rounds.rs`), is: the lead's branch is pushed and opened as a pull
request, two investigators from outside the team review it, and their verdicts lead to the merge and the next item, to a
fix pass, or to a question for the human. Without `github_token` in `secrets.toml` the autopilot is off (the daemon says so
at its start) and the items stay where they are; `[github] api_base` points it at another API ([`configuration.md`](configuration.md)).

**Nothing of a round lives in memory only.** A round is a thing of the state, with an ID (see "Rounds" below). Where it is
in its life is worked out at every look at a team from the items' phases and reviews, the runs and questions of the tasks and
two files of the task directories (`daemon/merge.rs`), so a daemon that restarts goes on with the same round. What is in memory
is only when to try again, what the human was told once, how often a report was sent back and when an approved pull request was
last read.

**Publishing** (`daemon/pr.rs`, `plan_publications`). A pull request is made of the branch of the lead: the coordinator's, which
integrates the members' branches and gets the gate green on it, or a solo agent's item branch. A branch is published when the lead
waits, its last gate record (`tasks/<lead>/gate.json`) passed on a clean tree at the tip of the branch, and the tip has the branch
of the finished item in its history (`git merge-base --is-ancestor`). The branch is pushed (git's own credentials, not the
token), the open pull request of the branch is found or opened as a draft with a title and a description from the item (its text,
expected result and checks) and the gate's summary, and marked ready; the round machine checks that GitHub says the pull request
carries the commit the gate passed on, and the items are `in_review` (`pull_request_opened`). A branch has one pull request at a
time: items integrated into the same head share it and move together; a finished item waits while another is in review on the
same branch (a solo agent's items each have a branch). A branch that cannot be published is tried again after a minute, and the
human is told once.

**Reviewers** (`daemon/review.rs`). The review pass of an item in review starts empty and is filled once (`ItemMut::ask_reviews`) with
two investigators drawn at random among the free ones: configured, not in the crew, with no task that is going, asking or paused,
no review to give and no team to lead that is not finished; the pull request waits for them. A reviewer is a task of the team
with a worktree at the head (a new branch of its own), one per investigator: it ends `waiting`, not `done`, is given the next
pull request, and the same one again after a fix pass, in the same session, and is closed with the team. Its prompt is the
working context, the review rules, the brief, the request (pull request, branch, base, the commit under review) and each item of
the pull request with its expected result and checks. Its final message is the report: the first `VERDICT:` line says
`APPROVE` or `REQUEST_CHANGES` (markup and case aside), or the report is asked for again, twice, and then the human is told. The
report is kept in `tasks/<reviewer>/review-pr<N>-pass<K>.md` (`K` fix passes the pull request had), posted on the pull request
as a **comment review** with the verdict in its body (the harness opens the pull request and posts with one account, which
GitHub does not let approve it) and recorded in every item of the pull request (`review_submitted`). A report is matched with
the review it answers by the time its run started, so one that comes after the review pass was decided is kept and posted, not
taken for the next review pass.

**What a verdict leads to** (`daemon/merge.rs`, the round machine). The first approval waits for the second; two approvals mean
`approved`; a request for changes starts the next fix pass, until `[review] fix_passes`, and then asks the human.

- **Merge.** In autopilot the pull request is read from GitHub and merged with a merge commit at the head it carries (a merge
  made elsewhere or by the human completes the items as well); the items are `done`, the base branch is fetched, and the next
  item starts from the remote's base when it is ahead of the checkout's, so that it builds on what was merged. A merge GitHub
  refuses for a reason waiting does not change is told once with GitHub's reason and not tried again; a pull request closed
  without a merge is told once. An approved pull request that waits is read again every 30 seconds.
- **`--no-merge`.** Nothing is merged: the human is told once that the pull request is approved. The next item of the feature
  starts at the approval; what depends on the pull request waits until it is merged, which the daemon finds when it reads it.
- **Fix pass.** It goes to the lead, which has the integrated branch: it is woken with the reports of the reviewers that asked
  for changes, the pass out of the configured ones and the branch (a solo agent switches to it first). When the gate has passed at
  the new tip the same pull request is updated (a comment, and its description when a finished item joins) and the same reviewers
  review it again.
- **The question.** When a reviewer still asks for changes after the configured passes, the lead is woken with the reports and the
  questions file to write; its question reaches the human like any other (`agent answer`). The answer is one more fix pass. A lead
  that ran and put no question leaves the human told directly, once.
- When every item is merged the team ends: its tasks, the reviewers among them, are `done` and the human is notified.

**Known limits.** The owner waits for GitHub and git calls (GitHub's own timeouts, at most 30 s an attempt; a push at most 10
minutes): a call that hangs leaves the socket unanswered that long. The merge is at the head GitHub carries, so a commit someone
else pushed after the reviews is merged as well. A review that cannot be posted counts and is only logged. A reviewer whose run
fails or is stopped is not replaced: the human resumes it. A solo agent that takes the next item while its first is in review
switches to its fix pass's branch by what the prompt says, not by the daemon. The reports are cut at 20,000 characters.

### Rounds

A **round** is the work of one team on one item of its task list, from the item's start to its merge (with `--no-merge`: to its
approval) — or the whole work of a team whose list has no items. One round, one branch of the lead, one pull request. It has an ID
(`round-YYYYMMDD-HHMMSS-xxxx`, `RoundId`) and is a record of the state: `{id, item?, opened_at, state}`, the state `open`, or
`closed` with a time and how it ended — `merged`, `approved` (`--no-merge`) or `abandoned`. A team's ID and a round's ID are the two
keys that everything else (logs, tasks, agents, worktrees, prompts) is found by. The word "round" means this and nothing else; the
questions of a task are numbered (`questions-<number>.md`) and the reviews asked for one head of a pull request are a review pass.

| When | What the owner does (`owner/round_state.rs`) | Event |
|---|---|---|
| a team starts | opens the round of every item given to a member at the start; for a list without items, the one round of the whole work | `round_opened` |
| an item is given to a member for the first time (`assign_items`) | opens its round, then moves the item on | `round_opened`, then `item_status` |
| an item is given again (its member left) | nothing: an item has one round, which stays | |
| the pull request of an item is merged — by the daemon or by the human | closes the round `merged`, after the item is `done` as merged | `round_closed` |
| both reviews approve and the team does not merge by itself (`--no-merge`) | closes the round `approved`; the human's merge later completes the item and leaves the round as it is | `round_closed` |
| a `stop` leaves every task of a team stopped, failed or done | closes the team's open rounds `abandoned` | `round_closed` |
| a team ends (all its items done; for a list without items, its lead said `DONE`) | closes the rounds still open `abandoned`: nothing merged the round of a whole list | `round_closed` |

A closed round is never opened again. A task that is resumed after a stop goes on with the item it had, in the round it had, which is
closed (`abandoned`): the runs that go on name it. A round is kept for ever, so the state shows what happened.

**The rounds of a run** are decided when the run starts, by a pure function of the task's role and of why the run starts
(`decide::round::rounds_of_run`), and recorded with the run (`Run::rounds`, `Team::begin_run`):

| The task | The run works for |
|---|---|
| a member, or a solo agent | the round of its item; none while it has no item. A solo agent on a list without items: the one round of the list |
| the coordinator | every round of its team that is open |
| a reviewer | the round of the pull request it reviews |
| a worker | the rounds of the run that asked for it |
| any task, for a fix pass, the answers of the human, or a resume | the rounds its last run worked for |

A run names only rounds of its own team, none twice. The daemon that restarts reads them from the state and goes on with the same rounds.

**What the state checks**, by its operations and again when it is read: an item has at most one round and has one as soon as it
leaves `pending` (`Team::advance_item`; an item the list marks `[x]` has none); a round belongs to an item of its team, or is the one
round of a list without items; a round closed as `merged` belongs to an item done as merged, one closed as `approved` to an item that is
approved or done as merged; a round is not closed before it was opened; a run names only the rounds of its own team. The full list is
in [`state.md`](state.md).

**The CLI.** `agent run` prints the round of each item it started, after the tasks; `agent status` lists the rounds of every team under
its line; `agent status <round>` (the protocol's `show` of a round) shows its item, its state, the tasks that worked for it with their
role, agent, worktree and branch and their runs, its pull request and its review passes (the reviews of the pass that was approved are
not kept once the item is done: the history of a later item has them).

### Round checks before the review

Before the head of a round goes to the reviewers — the first time and after every fix pass — the daemon checks the round's own
diff and its commits through git, with no model, as the repository's `checks` table says ([`configuration.md`](configuration.md)).
A repository without the table goes straight to review. Nothing in the code names a repository or its layout: what is a test,
what is production code and what judges a round are the three lists of path patterns.

**When.** In the autopilot's publishing (`owner/rounds/publish.rs`), once the lead waits and its gate passed at the tip: for a
first round after the pull request is opened as a draft and before it is marked ready; for a fix pass before the branch is
pushed, so that a head the checks hold back is not pushed and the reviewers keep the head they reviewed. The facts are the
files, the commits and the added lines from the merge base with the fetched base branch to the tip (`agent_runtime::git`:
`round_facts`) and the description of the pull request as GitHub has it now.

**The rules** (`agent_core::decide::checks`, each with the case that raises it and the nearest that does not):

| Rule | Who may allow it |
|---|---|
| an existing test file is changed other than by added lines, removed or renamed — a change of its mode (an executable bit) is a change too, with lines added or none (the facts read it with `git diff --raw`) | only the human |
| a protected file is changed in any way | only the human |
| a test file a `RED:` commit of the round touched has lines removed or replaced by a later commit (a descendant of it in the history), unless that commit touches test files only, its subject starts with `TEST-FIX:` and the description names it, by its ID or at least seven digits, under `## Tests changed after RED` (TDD-14). A later merge counts with what it changed against each parent that has the `RED:` commit, so a resolution that drops a line of the test is an edit; against a parent without it, the merge's lines are those of the other side, whose own commits answer for them | the team, or the human |
| the round changes production code and adds no line to a test file, and none of its commits has a line `Test-First: not possible — <reason>` (TDD-1) | the team, or the human |
| an added line skips a test, suppresses a lint or asks whether the code runs under test, in a file the description does not name (TDD-11, LINT-3, TDD-12). A suppression in Rust is `#[allow(…)]` and `#[expect(…)]` alike: the allowed spelling has to be named too. An attribute that `#[cfg_attr(…)]` applies counts as if it stood alone (`#[cfg_attr(not(test), expect(…))]`, `#[cfg_attr(miri, ignore)]`), also when rustfmt wraps it over several lines, and also when the round changes only one line of an attribute that was wrapped already: the daemon reads all the unchanged lines of a Rust file that has added ones, and an attribute with an added line is read as a whole, whatever its length. A file whose diff is too long to be read whole (over `8` MiB) cannot be checked: the round is not published and the human is told, as for any other failure to read the facts. `cfg!(test)` and `cfg!(not(test))` both ask whether the code runs under test. The description names a file with its whole path — `crates/other/src/a.rs` does not name `src/a.rs`, nor does `src/a.rs~` or `src/a.rs@backup`: a path is named as a whole token between delimiters (white space, quotes, brackets, `,;:#=*|`), and the dots and marks of a sentence may follow it | the team, or the human |

A file that matches `test_paths` is a test even when `code_paths` matches it too; a protected file is reported as protected
only. A test module inside a source file is not a test file: it stays with the reviewers.

**What the issues come to** (`decide::round::Checking`, the phase before the reviews are asked; a type of its own because the
tests of `RoundState` list its phases and events):

- no issue — or every issue allowed — the reviews start;
- an issue the team can fix — the round goes back to the lead with the reasons, at most `[review] check_passes` times; then
  the human is asked;
- only issues that the human alone may allow — the human is asked at once;
- both kinds — the team goes first.

**The message to the team.** The lead is woken (`Why::Checks`) and told the reasons — one for each issue, from
`prompts/harness/checks/` — with the pull request, the pass and what to do: settle every finding, get the gate green, end with
`STATUS: DONE`. It is made when the round is held and kept in the record, so a run that starts after a restart of the daemon is told
the same.

**The question to the human.** The daemon cannot put a question for a task that waits (the status table has no `waiting →
questions`), so — as for the reviewers' used-up passes — the lead is the messenger: the daemon writes the questions file of
the lead's next round itself, one question for each subject (a file, or "production code without a test"), each with the
reasons, why the human is asked, and the recommendation to refuse; it wakes the lead with the path and has it end its turn
with `STATUS: QUESTIONS` and change nothing. The human answers like any other question (`agent answer`): **only the word
`allow`, in any case, allows**; `ok` (the recommendation), any other text, no verdict and a question the lead removed refuse. Each subject's answer is the verdict of the question with its number, wherever it stands in the file: a removed
question passes its permission to no other subject.

- *Allowed* — the subjects are recorded for the items, which start the review (it is not asked again for the same file), and
  the count of passes starts again. The lead, resumed with the answers, is told to change nothing and end with `STATUS: DONE`.
- *Refused* — back to the team, with what the human refused named, as the next pass; the lead's resumed run is told to undo
  what was refused.
- A lead that ends without putting the question is woken again, twice; then the human is told directly (`notice-not-asked`).

**The record** is `tasks/<lead>/checks.json` ([`state.md`](state.md)): the passes and what was allowed for each item, the
question the round waits on, what the lead is to be told. Nothing of the checks lives in memory only; the count of reviews and
fix passes of the state is not touched.

**Known limits.** The description is read from GitHub when the check runs, without the lines the harness itself wrote for the
items (their text names files in passing): the harness writes the description of the pull request it opens, so a section such as
`## Tests changed after RED` or the name of a file with a suppression comes from the team or the human editing it afterwards —
the first check of such a round sends it back and the next reads the edit. A lead has one question at a time: while one waits, no
other round of the same lead is checked. A path with a
control character, or a range of more than 500 commits, cannot be checked and the branch is not published (the human is told once,
as for any publication that fails). Lines that are not UTF-8 text have their bad bytes replaced for the markers. Two items that share
a pull request move together: their passes are the larger of the two and what was allowed for either applies to both.

### The gate (`agent gate`)

`agent gate [TASK]`, run by an agent in its worktree, runs the repository's gate (`[repos.NAME] gate` of the
configuration) for the task and says what it said. The task is the one named, or the task of the worktree the command runs
in (`<home>/worktrees/<repository>/<task>`, also in a subdirectory, and also when `AGENTS_HOME` names the home through a
link); outside a worktree, and with no task named, the command fails before it asks anything. Every run the daemon starts
has `AGENTS_HOME` in its environment, so that the commands an agent runs find the daemon.

**One gate at a time on the machine.** The owner looks up the task, the repository's command and timeout and the worktree,
and puts a job on the queue of the *gate service* (`daemon/gate.rs`), a task of the daemon that takes the jobs one by one, in
the order they were put there. The owner never waits for a gate: the client's reply goes with the job and the service
answers it. At most 16 gates wait; the next is refused (`refused`, "the gate queue is full"). The client waits for its answer
as long as the daemon takes (`Connection::request_until_answered`: the one request without a time limit; the daemon bounds
it by the queue and the timeout, and a daemon that ends closes the connection).

**A gate.** When its turn comes the service reads the worktree with git: the commit `HEAD` names and whether the tree is
clean — a changed or untracked file, or a merge that waits to be committed, makes it **not clean**; a gate on such a tree is
run all the same, because a coordinator checks the tree it merged before it commits the merge, and it is recorded as not
clean. A worktree git cannot read is refused (`refused`: "the gate cannot run in ...") and nothing is run. The command runs
in the worktree in a process group of its own, with the `PATH` of `[binaries] agent_path`, its output (standard output and
error) in `tasks/<task>/gate-<n>.log` (`n` counts from 1; no log is overwritten), and no input. It is stopped, with
everything it started (`SIGTERM`, then `SIGKILL`), when it runs past `gate_timeout_minutes` — recorded `timed_out`, a failure —
and when its task is stopped or paused or the daemon shuts down while it runs — then nothing is recorded and the client is
answered `conflict`. A gate that waits when its task is stopped keeps its place in the queue and is answered `conflict`,
without running, when its turn comes; one that waits when the daemon shuts down is answered `internal` ("the daemon is
shutting down"). If git cannot read the worktree after the command ran, nothing is recorded and the client is answered
`internal` with the log's name. The owner keeps one token per task for its gates, whether the run that asked for the gate is still going or
not (a gate outlives its run: the agent may finish while the gate waits), and cancels it when the human stops or pauses the
task. An exit code of 0 is `passed`; any other code `failed` with that code, and a signal `failed` with 128 plus its
number. The worktree is read again when the command and its group have ended: a gate that changed a tracked file, left
an untracked one or moved `HEAD` (a commit, a reset) is recorded **not clean** too, at the commit it started from — the
commit does not hold what the gate passed. The files a gate leaves must therefore be ignored by git (`target/`, `build/`).

**The record.** `GateRecord` is `{tree, result}`: whether the tree was `clean` or `not_clean`, and the `GateResult`
(`head`, `finished_at`, `outcome`, `log`). The last record of a task is saved in `tasks/<task>/gate.json` (a temporary file
and a rename: a reader sees a whole record or none), told to the subscribers (`gate_finished`) and written to the task's
activity log. **Only a pass on a clean tree is a pass of the commit** (`GateRecord::counts_as_pass`): the gate on a
tree that is not clean says nothing about `head`. The record is the result the autopilot reads ("Pull requests, reviews and the merge"); an agent's word for
it counts for nothing.

**The command.** It prints first where the result is recorded (`tasks/<task>/gate.json`: a model CLI cuts a shell command
that runs past its limit (the Bash tool of Claude Code has one, in minutes, and a gate may take 30) while the gate goes on in the daemon, so an agent reads the
record there), then what it waits for, the last 60 lines of the output (at most the last 64 KiB of the log are
read), the verdict (`gate: passed at 26b65de; recorded. Full output: <log>`) and, for a pass on a tree that is not clean, that
it is not a pass of the commit. It succeeds for `passed` and fails (status 1) for a failure, a timeout and every refusal of
the daemon.

**A run that waits on its gate is not cut by the watchdog.** The model CLI of an agent that runs `agent gate` prints nothing
until the gate is over, which may be the wait in the queue and the gate's timeout. The owner keeps a `WatchdogHold`
(`agent_runtime::run`) for every run that is going and gives the engine a clone (`RunEngine::run_held`); a job takes a
`HoldGuard` of it when it is queued and drops it when it is answered, whatever way it ended. While a guard is alive the run
is never silent, and once the last is dropped its silence is counted from that moment, not from its last output. The gate
is stopped by the task's own gate token, not by the token of the run, which goes when the run ends.

**Known limits.** A gate whose client went away (the agent was killed) is not noticed while it waits or runs: it is stopped
only with its task or the daemon, or by its timeout. A gate on a tree that is not clean is not refused (the old script
refused it for the agent's own task); it is recorded as not clean. `agent gate --wait` (always waits here) and `agent
gate-status` of the old script do not exist. The queue and the records of the gates that were waiting when the daemon
stopped are lost; the last record of each task is kept.

### Questions, answers and controls (`agent answer`, `stop`, `pause`, `unpause`, `resume`, `say`)

The human's two ways into a running team: answering and steering. Both go through the daemon's socket; the daemon
alone changes the state.

**Questions.** A run that ends with `STATUS: QUESTIONS` and has written `questions-<number>.md` (the owner
reads it first: a missing or unreadable file fails the task, as in the table above) gets:

1. the instructions of `prompts/harness/questions/instructions.md` under the title of its file (rewritten in place,
   through a temporary file and a rename, with the file's mode), so the human reads what to do where the questions are;
2. a watch (`notify`, inotify, one non-recursive watch on the task's directory while questions are open) on the file;
3. the notification of `harness/run/notice-questions.md` (a file in `notifications/` and the configured command);
4. when `[questions] open = "vscode"`, the file opened in the VS Code window connected to the host: the remote command of
   the newest VS Code server with the socket of the newest window of the session (`agent_runtime::editor`); a window
   that was closed or does not answer is skipped, and nothing found is logged and not an error. A task of its own
   (`daemon/opener.rs`) does this, so a slow command never holds up the state.

**Three ways to answer**, all ending in the same code (`owner/answers.rs`): the answers are checked against the file
(`questions::apply`: a question the file does not have, a question left without a verdict, `ok` where there is no
recommendation — every problem listed at once, `invalid`), put into the file, the questions are recorded as answered with their
channel (`command`, `saved_file`, `dashboard`), the task goes `questions` → `queued` and a `question_answered` event
is published.

| Channel | How the answers arrive |
|---|---|
| `agent answer <task> [--file FILE]` | the command asks `show` which questions are open and where their file is, takes the verdicts of `FILE` — or of the questions file after the editor (`[questions] editor`, else `$VISUAL`, else `$EDITOR`) ends — and sends `answer`. A file saved with the marker while the editor was open is taken by the daemon first: the command says so and sends nothing |
| the file saved with `**ANSWERED**` | the watch tells the owner the file was written; it reads it. Not marked: nothing. Marked and complete: answered. Marked and not complete: the human gets the notification `harness/questions/notice-incomplete.md` listing the problems, once for the same problems. A file saved while the daemon was down is looked at on the start |
| the dashboard | `answer` with the client `dashboard`: the same as the command |

**What the task is told.** The prompt of the next run is made when the run is made ready, from the state and the file, so
it survives a restart: when the last questions are answered and no run since the one that asked has said anything to the end
(a reported `STATUS:`, or an exit with status `0`), the run gets the answered file (`harness/questions/answered.md`:
without the instructions and the marker, an `ok` replaced by the words of `accepted.md` and the recommendation) — instead
of the whole task for a session that resumes, after it for one that starts. The answers are given again to every run
until one has had them: a run that crashed, was stopped, hung or hit a limit did not read them. A run that ended with an
unusable questions file leaves a state this cannot tell from that, so a later resume repeats the old answers; that is
harmless.

**Controls** (`owner/controls.rs`, the table in `control.rs`) reach each task of the target by itself, so a team with one
task that cannot be controlled is still controlled, and the answer lists the tasks it reached; when it reached none, the
refusal (`conflict`) says why for each task.

| Control | queued | running | waiting | questions | paused | stopped | failed | done |
|---|---|---|---|---|---|---|---|---|
| `stop` | stopped | run stopped, then stopped | stopped | — | stopped | — | stopped | — |
| `pause` | paused | run stopped, then paused | paused | — | — | — | — | — |
| `unpause` | — | — | — | — | queued | — | — | — |
| `resume` | — | — | queued | — | — | queued | queued | — |

A task with a run going has the run's process group stopped (the engine's `SIGTERM`, then `SIGKILL`), the run is recorded
as ended `stopped`, and the owner answers only when that is done, so a client that has its answer sees the status it
asked for. A run that ends by itself while the stop is being asked leaves the task stopped (or paused) where the status
table allows; a task that asks, failed or is done keeps what the run made of it. A task that waits for answers is not stopped, paused or resumed: it runs nothing, and its open questions have to be answered before it can ask more. A pause of the human has no wake-up time:
it lasts until `unpause`, and survives a restart. `resume` and `unpause` start the task again in its session; the prompt is
the one the end of its last run gives (a stop gets the whole task again, as before). A team's control also reaches its
reviewers. Each control is published (`controlled`, with the task, or none for a team, and the client) and written to the
activity log of each task it reached: `{"at":…,"kind":"control","action":"stop","by":{"kind":"cli"}}`.

`say <target> words…` puts the words on the bus, from `human`: for a task to its agent, for a team to the team (a team
on the bus) or to each of its agents (a solo task has no team on the bus), and wakes a member that sleeps through the
look at its team. A task that is done hears nothing; the bus being down refuses the command, with the reason.

**The watch is bounded.** The saves that wait for the owner are at most 256, each file once; a save that finds the queue
full, and an error of the system's watch, make the owner read every open questions file (the look it takes on a start),
so no save is lost for good.

**Known limits.** A save of a questions file that the system's watch limit would not let the daemon watch is not noticed
(`agent answer` and the dashboard still work); the daemon says so in its log. A save in the middle of a write can be read
half-way: the next event of the save has the whole file. The words of a `say` to a solo task that is `waiting` are read
when it next runs; nothing wakes it. A human's stop of a worker that serves a request answers the request as failed.

## Process model

`agent daemon`, a systemd user service, is the only owner of the state and of every model run. Every other command
— `agent run`, `answer`, `status`, `stop`, `gate`, ... — and the dashboard `agent-web` are clients of its Unix
socket. One owner, no file races (CON-3).

- `agent run <agents...> "tasks" | -f tasks.md [--coordinator NAME] [--no-merge]`: one agent is a solo task, more
  than one is a team. `--autopilot` is the default: PR → two reviews → merge → next item; `--no-merge` stops before
  the merge and hands the PR to the human.
- The coordinator: an explicit `Coordinator: NAME` line at the top of the task list or `--coordinator` wins;
  otherwise an investigator in the team (randomly among two or more); otherwise a random normal agent. Workers never
  coordinate. An investigator coordinating weaker models leads them: explains, shows the way, and writes or fixes
  code itself when needed.
- The daemon assigns items, one to each normal member at a time. Workers do not take items: they serve `delegate`
  requests, the first free worker in the configured order.
- The coordinator integrates a member's branch, gets the gate green and opens the PR. The next item of a feature
  starts when the previous one is merged (with `--no-merge`: approved by both reviews).
- Every PR gets two reviews from investigators outside the team. A team that leaves fewer than two configured
  investigators outside it is refused at start: "too many investigators in the team". "Changes requested" goes back
  to the team at most 5 times; after that the human is asked.

## Rules of the plan

These hold for every item of the plan, on top of the rules each role is given — the developer rules to write Rust, the
review rules to review it — whose rule IDs (ERR-2, PR-1, ...) they cite. Both are in the prompt catalog (`PromptId`);
the developer rules also come in a variant for workers (a delegated task instead of a pull request).

- **Size, relaxed for this plan:** PR-1's limit is 5000 changed lines per PR, tests not counted, and PR-2's threshold
  for planning a split is about 4000. Everything else in PR-1 and PR-2 applies as written: how lines are counted, a
  ceiling not a target, never fewer tests because of it. An item that would exceed the limit is split before coding
  and the split is recorded in the PR.
- **One item, one PR.** The PR description quotes the item in full under `## Task`; the reviewers check the PR
  against it and run its checks. A file outside the item's expected list, an expected part left out, or a check that
  cannot pass yet needs its reason in the PR description. Each round starts from its own file,
  `rounds/round-N.M.md` ([`rounds/round-1.1.md`](../rounds/round-1.1.md), [`rounds/round-2.1.md`](../rounds/round-2.1.md)).
- **Every item stands alone:** it compiles and passes the whole gate on `main` without the other items of its
  feature, and uses only code from the features its feature's "Depends on" line names, all of them merged. Items of
  one feature are taken in order; features whose dependencies are merged run in parallel.
- **The old implementation is a reference only.** `bin/agent` (Bash), `agent/` (the unfinished Go version) and
  `live/` (the old dashboard) show how it worked. Nothing is ported line by line, linked, called or kept compatible:
  no shared files, no shared state format, no shared commands. `agentbus/` (Go) stays and is used as it is.
- **No prompt text in `.rs` files, ever.** Everything a model reads, and everything written into a file a human
  answers (questions-file instructions, notification texts), lives in `prompts/` as a template with `{{name}}`
  variables. Error messages of the CLI and log events stay in code.
- **Async with `tokio`.** No external process except the model CLIs (`claude`, `codex`, `agy`, `opencode`), `git`,
  `agentbus`, the repository's own gate command, the configured notification command and the configured editor.
  Everything the Bash version did with `sed`, `awk`, `jq`, `grep`, `flock`, `gh` or `tailscale` is done natively.
- **Approved dependencies:** `tokio`, `tokio-util`, `clap`, `serde`, `serde_json`, `toml`, `thiserror`, `tracing`,
  `tracing-subscriber`, `tempfile`, `rustix`, `reqwest` (rustls), `axum`, `notify`, `rand`, `time`, `strum` (`derive`). Any other needs
  a DEP-1 justification. Each is declared once, in `[workspace.dependencies]`, with its minimal features.
- **A separate `AGENTS_HOME`.** New code is developed and tested against its own `AGENTS_HOME`; nothing touches the
  live `~/agents` of the running Bash harness until feature 10.
- English everywhere: code, comments, docs, commit messages, PR descriptions.

Out of scope for the plan: remote nodes and the lead, investigation teams with phases and standalone investigation
requests, `agent live`, archiving, the cost of tokens in money, moving the state file into the database, choosing
agents by the quota their accounts have left (load balancing), automatic PR size limits (reviewers check PR-1), a
proof by the harness that a RED commit fails (a reviewer runs one, TDD-1), an `agentbus` socket API (it is called
as a process).

## The gate

```sh
make rust     # the workspace's checks, with their full output
make gate     # the gate: the same checks with short output (GATE_STEPS, by default the rust step alone)
```

`make rust` runs, all `--locked`:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked          # doctests included, under the machine's test lock
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked
```

`make gate` runs it alone: the Bash and Go suites of the old implementation are steps of `make gate-full`, which
only the human runs. It is also a step of `make test` and `make lint`. The existing CI workflow
([`.github/workflows/test.yml`](../.github/workflows/test.yml)) runs `make lint`, so it also runs the complete Rust
gate. The Ubuntu runner supplies rustup; the first cargo command installs the pinned toolchain and its components.
No workflow edit is needed. This also avoids requiring the investigation token to have the `workflow` scope.

The lints are the baseline of the developer rules' appendix, in `[workspace.lints]` of `Cargo.toml`; every crate has
`[lints] workspace = true`. They are `warn` there and fatal through `-D warnings` in the gate. A lint is suppressed
only with `#[expect(lint, reason = "...")]` on the smallest item (LINT-3), and every suppression is listed in the PR.

## Tests first, in Rust

The cycle of [`docs/tdd.md`](tdd.md) holds: the RED commit has the tests and fails only on them, the GREEN commit
the simplest code that passes. In Rust the RED commit's bodies are compiling stubs that return a wrong value (accept
everything, refuse everything, an empty table) — never `todo!()`, which aborts the test instead of failing it and is
never committed (TDD-1). A guarantee a type claims — a private constructor, two types that do not mix — gets a
`compile_fail` doctest; stable rustdoc does not check the error code named in it, so the snippet is checked by hand
to fail for that reason.

The commits say which step they are: the subject of a RED commit starts with `RED:`, of the code that passes it with
`GREEN:`, and a GREEN commit does not touch the tests. A test is frozen once its RED commit exists (TDD-14): later
commits add tests; a test that was itself wrong is corrected by a commit of its own that touches tests only, its
subject starting with `TEST-FIX:`, listed in the PR under `## Tests changed after RED`. The reviewers compare each
RED commit with the head and run one of them (the review rules, steps 4 and 6). The unit tests of a new module live
in its `tests.rs` (`#[cfg(test)] mod tests;`), so a change of the tests is a change of test files (TDD-10); an inline
`mod tests { }` that is already there stays.
