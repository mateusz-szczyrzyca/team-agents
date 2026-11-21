# The Rust `agent` — architecture

`agent` is a Cargo workspace in this repository, built item by item of the plan "Rewrite `bin/agent` in Rust"
(`rust-agent-plan.md`, `rounds/`). This file describes the workspace's layout, its process model and the rules every
item of the plan follows. Read it before you change anything under `crates/`; [`AGENTS.md`](../AGENTS.md) ("Rust")
points here.

The implementation it replaced — `bin/agent` (Bash), `agent/` (an unfinished Go version) and `live/` (the terminal
panel and the old dashboard) — was removed from the repository in 0.25.0; git's history has it.

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
docs/architecture.md, docs/configuration.md, docs/state.md, docs/task-list.md, docs/plan.md, docs/history.md,
docs/history.schema.sql
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
| `notice_status` | `NoticeStatus` (`Questions`, `Decision`, `Failed`, `Hung`, `Paused`, `Approved`, `NotMerged`, `Closed`, `NotPublished`, `NoVerdict`, `Merged`, `Asleep`, `Done`) with its icon and label (`strum`) |

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
| `plan` | `Plan`, `Stage`, `Step`, `StepState` (`[ ]`, `[>]`, `[x]`, `[-]`), `Check` (a command, `exists <path>` or `reviewer`) and `PlanError`, whose every variant but two carries its line; the comparison of two versions (`Plan::compare` → `PlanChanges`: added, state changed, disappeared, retitled, by step ID only) and the text back (`Plan::to_text`). See [`plan.md`](plan.md) |

`Config`, `TaskList` and `Plan` take text, not paths: reading `agents.toml`, `secrets.toml` (and the file mode `Secrets::parse` needs) and a
`-f` task list is the runtime's job, so the core stays free of I/O. A plan of steps is text too: `parse_plan` reads `.agent/plan.md`
in the task list's format read in a plan mode and shares the task list's number, indentation and size helpers; it keeps nothing on
disk, and the daemon that reads and writes the file of a task is a later item.

### State, protocol and questions files (`agent-core`)

| Module | Types |
|---|---|
| `state` | `State` (with its schema version 7; versions 2 through 6 are migrated without changing active review rules and version 1 is migrated as it is read; the `findings` ledger of an item), `Team`, `Crew`, `Member`, `Participant`, `Run`, `RunEnd`, `Question`, `Round`, `RoundLife`, `RoundEnd`, `ItemProgress`, `ItemPhase`, `PullRequest`, `Review`, `FixPass`, `GateResult`, `GateRecord`, `GateTree`, `ChecksRecord` (the checks' record of a lead, a file of its task directory), `Timestamp`, `SessionId`, `CommitId`, `PrNumber`. See [`state.md`](state.md) |
| `protocol` | `Request`, `Command`, `Response`, `Reply`, `Failure`, `Event`, `EventKind`, `Frame`, with the protocol version and `ProtocolError`. JSON lines over the daemon's socket. See [`state.md`](state.md) |
| `review_findings` | `Severity` (`BLOCKER`, `MAJOR`, `MINOR`), `Finding`, `Answer` (`FINDING 3: RESOLVED`, `OPEN — why`), `parse_report`: a review report read into its findings and its answers for the open ones, or the line that is wrong. Pure: the harness sends the report back to its reviewer when it fails |
| `questions` | `QuestionsFile`, `Entry`, `Verdict`: parsing and rendering of the questions file, the `ok` verdict, the `**ANSWERED**` marker. See [`state.md`](state.md) |

They take and give text too (`State::from_json`, `Request::from_line`, `QuestionsFile::parse`): the state store, the
socket and the files are the runtime's. `serde_json` is a dependency of `agent-core` for that; it does no I/O.
`strum` (derives only) gives the keyword enums their text, as `McpServerKind` does.

Below internally tagged enums (`Outcome`, `Reply`, `EventKind`), `serde` buffers enum variant contents into an intermediate tree where JSON object keys are preserved as strings. When a map or set is keyed by finding numbers (`NonZeroU32`), standard integer deserializers reject string keys with `invalid type: string, expected a nonzero u32`. All maps and sets keyed by finding numbers (`RoundDetail.opinion_attempts`, `RoundDetail.opinions`, `FindingsWire.opinions`, `FindingsWire.opinion_attempts`, `FindingsWire.fixed`, `Disputes.accepted_heads`, `Disputes.explanations`, `FindingsWire.opinion_notices`, `Disputes.changed_findings`) use `#[serde(deserialize_with = "crate::protocol::finding_map::deserialize_map")]` (or `deserialize_set`). Any new map or set keyed by finding numbers in protocol replies, events or persisted state must use these deserializers so it reads back correctly both from direct JSON and below tagged enums.

Investigator activity history is a per-team `activities` list in state schema 11. Typed requests classify crew
membership, first reviews, fix-pass verifications, second opinions and future investigation or allocation work.
The daemon retains assignments before launching them, accepted reports before item phases discard their review
lists, and terminal task outcomes before saving. Migration from schemas 1–10 reconstructs the same records from
crew, item review/fix-pass history and opinion attempts, with deterministic timestamp fallbacks where older
schemas kept no exact time. The pure `decide::investigator::choose` consumes configured names, free names,
activity facts, the target team, permanent exclusions and a nonzero request size. It performs no I/O and uses
neither a clock nor a random generator. Selection adds no persisted or protocol fields. What the record shows — the
reviewers' lines of `agent status --running` and the view of `agent status --investigators` — is described below
("What the investigators did").

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
- Claude's turn budget comes from `[limits] max_turns` (default `300`). Other quirk settings remain adapter constants: Claude's Bash timeout (20 minutes), `agy`'s
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
| `decide::team` | `plan_team`: from the agents named, the configuration, the coordinator asked for (`--coordinator`, then the task list's `Coordinator:` line) and a random source, a `TeamPlan` — solo, or a team with its coordinator, members and workers — or a `TeamPlanError`: unknown agent, a worker as coordinator, a coordinator that was not named, only workers, no member besides the coordinator, and too many (or too few) investigators for the configured outside reviewers |
| `decide::assign` | `next_assignments`: which item goes to which member next — one item per member, the items of a feature in order (the next starts when the previous is merged, with `--no-merge` approved), features in parallel once the features they depend on are merged; `first_free_worker` for `delegate` |
| `decide::checks` | `assess`: from the facts of a round's diff (its files, the files whose mode changed, its commits with their parents, the lines it added, the description of its pull request) and the repository's `checks` lists, the `Issue`s — an existing test file that loses lines or is removed or renamed and a protected file that changes (only the human may allow them); a test of a `RED:` commit edited by a later commit unless a named `TEST-FIX:`; production code with no added test line and no `Test-First: not possible — <reason>`; an added line that skips a test, suppresses a lint or asks whether the code runs under test, in a file the description does not name (the built-in `markers` table per extension). See "Round checks before the review" |
| `decide::plan` | `decide_plan`: from the role, how the run ended, the plan read at its end (`PlanRead`: missing, unreadable with its error, or the plan), the version kept from an earlier run, what the task keeps of its plan in the state and `[plan]`, the `PlanVerdict` — accept, send the run back with a `PlanReason` (missing, unreadable, a lost step, an unfinished step, in this order), or ask the human once the task was sent back `bounces` times (plus what the human granted). A role outside `[plan] roles`, another end than `DONE`, a task created before plans and a plan the human released are accepted. See [`plan.md`](plan.md) |
| `decide::round` | `RoundState`, `RoundEvent` and `RoundState::transition`: member work, integration, gate green at the head, pull request published for that head, the configured reviews, merge or hand-over to the human, item done; "changes requested" starts a fix pass until `[review] fix_passes`, then the human is asked, and the answer grants one more pass. Each transition says what the daemon does next (`RoundAction`); `Checking` is the phase before the reviews are asked: what the issues of the checks come to — the reviews start, the round goes back to the team (at most `[review] check_passes` times) or the human is asked |
| `decide::round::rounds_of_run` | `rounds_of_run`: from the role of a task, why its run starts (`RunStart`: its own work, a request for a worker, a review, a continuation) and what the state says of its team's rounds (`TaskRounds`), the rounds the run works for. See "Rounds" |
| `decide::outcome` | `classify_outcome`: from the final message, the exit status, the error tail and who ended the run, the `RunEnd` to record — reported status, no status, quota exhausted (with the reset time when the CLI gives one), rate limited, turn limit, hung, stopped — and the `Pause` it implies; `after_hang` for the watchdog's strikes; `after_turn_limit` for bounded same-session continuation. The CLIs' outputs are fixtures in `crates/agent-core/tests/fixtures/cli-outputs/` |

The daemon feeds these functions what it observed and carries out what they return; none of them touches a file, a process or
the network. The order of the checks and the rules each follows are in the documentation of its module.

### The run engine and the daemon (`agent-runtime`, `agent`)

`agent daemon` is the process model below, working: one daemon per home, the Unix socket, the run engine, recovery.

| Where | What |
|---|---|
| `agent_runtime::run` | `RunEngine::run`: one run of a model CLI, from its files to its classified end; touches no state (`run_held`: the same with a `WatchdogHold`, see "The gate"). The run's environment has `AGENTS_HOME`. `run::compose` makes its prompt: context, role, the rules of the tier, brief, item, checkpoint — or, for a resumed session, what interrupted it |
| `agent_runtime::{settings, harness_prompts, autopilot_prompts, checks_prompts, text_file, clock}` | `agents.toml` and `secrets.toml` read from the home; the second prompt catalog (`HarnessPrompt`, `prompts/harness/run/`: the run prompts and the notifications) and the fourth and fifth (`AutopilotPrompts`, `ChecksPrompts`); a bounded text file; the system clock |
| `agent::daemon` | `owner` (the state), `owner/plan` (the plan of a task at a run's end and start, the question about it and its answer, the plan kept out of the delivery), `server` (the socket), `notifier`, `opener`, `settle` (what a run's end does), `recovery` (on start), `team` (the team's life as decisions), `owner/teams` (what carries them out), `questions` (the questions file) and `control` (what a control does to a task), with `owner/answers` and `owner/controls`, and the autopilot: `pr`, `review`, `merge` and `checks` (its decisions) with `owner/rounds` (what carries them out) |
| `agent_runtime::history`, `agent_core::history`, `agent::daemon::{history, owner/history}` | the history: a database of what happened and what every run did; see "The history" |
| `agent::{client, commands}` | the socket client, `agent status`, `agent daemon`, `agent run`, `agent answer`, `stop`, `pause`, `unpause`, `resume`, `say` |

**Tasks** (CON-3, CON-4): the *owner* alone reads and changes the state; the *server* passes requests to it through a
bounded channel; the *notifier* delivers notifications off its path. Cancelling the shutdown token (SIGTERM, SIGINT)
stops them in order: the server stops accepting, the owner has its runs stopped and recorded, the notifier delivers what
is queued, the socket is removed; every task is joined. A state that cannot be saved ends the daemon with an error —
models must not work on with a state that is not kept — and `Restart=on-failure` of the unit starts it again.

**Agent text.** A reviewer's malformed report is a protocol refusal, not a failure of the state owner. Before recording
anything, the owner parses it against every item whose ledger will receive it; recording consumes those parsed reports
without parsing the text again. A refusal returns the report to the reviewer with its source line, finding number and
the rejecting item's allowed answers, or asks the human after two corrections. It cannot stop the daemon or record
only part of the review. Failures of the storage and other infrastructure needed to keep state can still stop the owner.

**GitHub and git.** A failure of GitHub or of git in a step of a round is not a failure of the state owner either. The
steps that could end the owner wait instead (`owner/rounds/waiting/github.rs`): refreshing the description of a team's
pull requests at each look; making or moving the worktree of the investigator of a second opinion; and the three reads
of a worktree's commit — the lead's before an approved pull request whose item is in its fix pass is merged (the team's
publication waits with it, so that the unread commit is not taken for a new fix), the lead's to check its fix
response, and a verifier's before its verdict is taken (the verdict is held and taken again at a later look; nothing
of it is recorded before the commit is read). A failure that may pass (no connection, a timeout, a 5xx answer, a rate
limit; a git fetch or push) is tried again at the first look after a pause of 30 seconds from the end of the failed
call, doubled after every further failure up to 10 minutes (`agent_core::decide::retry::pause`), without end. A failure
that will not pass (401, 403 that is not a rate limit, 404, 422, git's own refusals) waits for the human: one notice
(`harness/pr/notice-github-refused.md`) names the step and the answer, again only after the step went through and was
refused anew, and `agent resume <team>` (or a task or round of it) tries every waiting step of the team at the next
look; such a resume starts no task of the team. The quick retries inside one call (`[github]` attempts and delays)
stay as they were. The waits live in the owner's memory, not in `state.json`: a daemon that starts has none, so every
step is tried at its first look; a verdict held for its commit is not kept over a start. While a step waits the rest
of the look, the other rounds and the other teams go on; `agent status` says what the round waits for ("GitHub:
refresh the description of the pull requests, tried 4 times, next try 01:07"). The log has one `github_step_waiting`
warning when a step starts to wait and one `github_step_resumed` when it goes through. Publishing, merging, posting
comments and reviews, asking for and replacing reviewers and the fetches log a failure of GitHub or git and try again
at a later look, as before; none of them ends the owner. Two failures of the home, not of GitHub or git, still end
it: a worktree path that is not UTF-8 text, and a file of an investigator's run that cannot be inspected when a silent
investigator is replaced (`replace_silent_opinions`).

**Items of one pull request.** The items reviewed on one pull request share its findings ledger: each item keeps a
copy, and every fix response, verification and finding is recorded in all of them, counted by the pull request's fix
pass — the most fix passes of any of its items, since an item that joined a fix pass has none of its own for it. A
verifier's brief and the reading of its report use one list of allowed answers per finding
(`Findings::verifier_answers`). A state whose copies disagree (written before this rule held) is repaired when the
daemon starts (`recovery::reconcile_findings`): for each finding every copy has, the most advanced record — a decided
explanation over a pending one over none, a rejected over an accepted one, then a closed finding over an open one,
then the higher severity — gives its explanation, status and severity to all copies, and each repaired finding is
logged as `restart_corrected`. Reviewers that wait with a review pending which they already ran for are asked again.

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
| `STATUS: DONE` against its plan | `queued`, sent back with the reason; after `[plan] bounces` send-backs `questions`, in a questions file the daemon writes | only for the roles of `[plan] roles` and tasks this version made; see [`plan.md`](plan.md#the-daemon-and-the-plan) |
| `STATUS: QUESTIONS` | `questions`, when `tasks/<task>/questions-<number>.md` is there and parses | the questions are recorded and the human told; a missing or unreadable file fails the task and says why |
| `STATUS: FAILED` | `failed` | the coordinator (or human) is told |
| no `STATUS:` line | `queued`: resumed in the same session with a short prompt (`harness/run/resume-no-status.md`) | at most 2 consecutive automatic resumes for resumable tasks with a session (solo, coordinator, member; excluding workers serving requests, reviewers, tasks without a session, or failures during startup/execution); the 3rd goes up the ladder (below): once a new session with a briefing, then it fails and notifies. Runs ending without a status line upon daemon shutdown/interruption (exit code `None`) count toward the streak so restarts remain bounded. Any other end resets the count. Settle records `Settled::NoStatus` so human pause or stop overrules retry |
| `STATUS: SLEEPING` / `EXCLUDED` | `waiting` / `done` | |
| quota exhausted, rate limited | `paused` | woken at the time the CLI gave (plus a minute), else after the `[limits]` minutes; the human is told |
| turn limit | `queued`: resumed in the same session with a short continuation prompt | at most `[limits] turn_limit_resumes` consecutive automatic resumes (default `3`); the next exhaustion goes up the ladder (below), then fails and notifies with the count; any other end resets the count |
| hung | `queued`: resumed from its session | after `[watchdog] hang_strikes` hangs in a row without progress, the ladder (below), then `failed` and the human is told; a run that did something before it went silent starts the count again |
| the daemon shut down | `queued`, or `failed` when the no-status budget is exhausted and neither the ladder nor the takeover (below) has a step | the run is recorded as ended with no status and no exit code; on the third consecutive no-status end of a session, a resumable solo, coordinator or member task goes up the ladder (below), and fails when it has no step left |

**The ladder** (`decide::ladder::next_step`, `[replacement] new_sessions`): after the retries of the same session a task of
a solo agent, a coordinator or a member is started once in a new session of the same agent. The renewal (`renewals` of the
member, with its reason) is saved with the decision, so a restart before the new run keeps it; the run after it resumes
no session. When that run starts, the daemon takes a snapshot of the worktree (recorded in the history) and the prompt is
the briefing (`harness/run/briefing.md`): why the task is started anew, its whole prompt, the commits of its branch since
the base, the uncommitted files, its plan of steps and the unread messages of its team (read from the last 4 MiB of
the bus's log and its cursors in at most 64 MiB of its state, `agentbus/unread.rs`; when only the newest are shown the
briefing says so). Every word of it is in the catalog: the values are facts (the reason as the state records it, the
plan's marks). `State::renew_task` records the renewal and refuses a worker and a reviewer. Each decision is logged as `decision_taken` with `decision` `ladder`. A worker and a
reviewer are not started anew; when the new session goes bad too, the task fails as before.

**The takeover** (`daemon/owner/takeover.rs`, `decide::replacement::choose`, `[replacement] agents`, `mode`,
`replacements`): when the ladder has no step left, the daemon offers the task to the agents of `[replacement] agents`, in
their order, leaving out the agent that holds it and every agent of its team (crew and reviewers: a place in the team is
one task). `choose` takes the first that is not in a run and of the same or a higher tier; `State::hand_over_task` gives
it the task and keeps the agent before it in the member's `past_holders`, the items the task works on are reassigned to
it on their branches, and the new holder's session begins at the same moment (`State::begin_holder_session`, the
boundary in the member's `renewals` without raising their count), so its first run is briefed like a new session and
its retries are its own. A task keeps the name of its first agent (`Member::bus_agent`) on its team's bus and in its own
branch: the agentbus server of its runs gets `--agent` with that name, the bus's states are reconciled under it, the
team's context and the coordinator's notices name it, and the briefing reads the messages to it — the bus itself never
learns of a new holder, and needs no change. With nobody fitting — or at once with `mode = "lead"`, and in mode `auto`
with no agents listed — a member's task is stopped and its lead is told on the bus that the work is its own
(`harness/team/bus-lead-takes-work.md`, notice `member_failed`), followed by the briefing of the member's work without
its prompt (`harness/run/lead-briefing.md`), gathered with a snapshot at the next turn of the loop and before the daemon
ends. The work is owed to the lead in the state (`State::owe_lead`, `lead_owed` in the member's `renewals`), saved with
the stop, and the mark is cleared (`State::lead_told`) only once the bus took the notice, so a restart or a bus that
fails before the lead is told tells it at the next turn after all; the item of the stopped member is published when the lead ends `DONE`, as for a member the human stopped
(`pr::joins`). A solo task and a coordinator's have no lead: they fail as before, and so does every task with
`mode = "off"`, or once it was taken over `replacements` times. Each choice is logged as `decision_taken` with
`decision` `takeover`, and each hand-over as `task_handed_over` (history migration 16 admits it).

**After the last step** (`daemon/owner/last_step.rs`): when a run would fail a task that was handed over before, or a
coordinator holding the item of a stopped member, the daemon writes the task's next questions file from
`harness/run/notice-last-step.md` instead, records its number with `State::ask_after_last_step` (in the member's
`renewals`, with the boundary of a new session) and settles the run as one that asked questions; a file the state does
not record is removed again and the task fails as before. Whether to ask is decided by `decide::last_step::after_last_step`
(mode, role, asked before, held before, a stopped member's work), the answer by `decide::last_step::last_step_answer`:
`a` queues the task, which starts briefed in that new session; any other answer — `b`, `ok`, free text — stops it, and
a member's work is owed to its lead as above. A task is asked so only once, and never in mode `off`. The open question
is in the state, so a restart while it waits keeps it, and its answer still does what it says.

**The watchdog** is the engine's: a run whose output files show no change for `[watchdog] hang_minutes` is stopped (its
whole process group, `SIGTERM` then `SIGKILL`) and classified as a hang unless the provider refused it. **A pause** is
kept in no field of the state: its wake-up is worked out again from the last run's end, so it survives a restart; the
daemon looks at the clock every 5 seconds, so a suspended machine does not stretch it. **Recovery**: a run that was going
when the daemon died is ended as "no status, no exit code" and its task queued, to resume from its session, unless that
end exhausts its no-status budget: the task then fails and its existing failure notices are sent. Paused tasks get
their wake-ups again, hung ones their strikes (progress is not saved: every hang in a row counts); a clock set back never
ends a run before it began. A run is a process group of its own: a daemon killed without `SIGTERM` (`SIGKILL`, the OOM
killer) leaves it going, and only the unit's `KillMode=mixed` ends it — outside systemd, end such runs before a restart,
or a second CLI resumes the same session.

**The socket** serves `status`, `show`, `subscribe`, `run`, `answer`, `stop`, `pause`, `unpause`, `resume` and `say` of
[`state.md`](state.md), and `gate` (below). At most 64 connections and 16 runs go at once; a line over 2 MiB ends its
connection; a subscriber that falls 1024 events behind is let go (it subscribes again and starts from the state).

**The home**: `agents.toml`, `secrets.toml` and `prompts/` (put there by the human); `state.json`, `history.db` (the history), `daemon.lock`,
`agent.sock`; `tasks/<task>/activity.jsonl`, `tasks/<task>/runs/<run>/` (the output of a run and the files its CLI is
configured with, `prompt.md` for a prompt too long for a command line) and `tasks/<task>/questions-<number>.md`;
`tasks/<task>/gate-<n>.log` and `tasks/<task>/gate.json` (below); `tasks/<reviewer task>/review-pr<N>-pass<K>.md` (the reports of the reviews, below); `worktrees/<repo>/<task>/`; `notifications/`.

### Teams (`agent run` and the team's life)

`agent run <agents...> "tasks" | -f file [--coordinator NAME] [--no-merge] [--allow-unpushed-base]` names the repository by the `origin` of the
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

Before creating resources, the owner fetches the configured base branch from `origin` and compares its reachable
commits with the local base. Missing origin preserves the existing start path; an equal or behind local base starts
from the local branch as before. Fetch failures refuse the request with git's error. An ahead or diverged base is
refused with the count, first unpublished commit and push/reset remedies unless the optional protocol field
`allow_unpushed_base` is true (absent means false). The CLI exposes this as `--allow-unpushed-base`.

State schema 12 adds optional per-team `unpushed_base` evidence: branch, positive commit count, first commit ID and
subject. Schema 11 migrates without changing activities or granting permission; older migrations retain their
existing reconstruction. The evidence is printed by `agent run` and rendered by the harness's
`prompts/harness/pr/unpushed-base.md` in every PR header, including updates and later items. The lead contribution
cannot replace the harness-owned `## Unpushed base` section.

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
`member_failed`, `member_no_status`), which wakes it to integrate. If the member failed or ended without status while holding an item, the message names the item and the branch (`prompts/harness/team/bus-member-failed-item.md`, `bus-member-no-status-item.md`), advising the coordinator that if it takes over the work, it should still report `STATUS: DONE`: the harness will tell the human, the human will resume the member, and the member must end with a status. The harness publishes an item while its assignee task waits, or, when the assignee task is stopped or no longer in the crew, once the coordinator's last run ended `STATUS: DONE`: the item follows its task, not the agent. A coordinator taking over the item of a failed member finishes its own work and reports `STATUS: DONE`, after which the human is alerted to resume the member with status. A member's item stays `in_progress` until its pull
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

**Known limits.** An agent that never reads its messages is woken at most `limits.wake_ups_per_message` times for the same message (`owner/wakes.rs`): then the daemon stops, logs a `bus_down` warning and leaves the message unread on the bus. The budget is kept in the state (`wake_deliveries`, see `docs/state.md`): the count is saved before each wake-up, a `delivered` marker is saved at the limit, and both are restored after a restart of the daemon, so that a restart neither grants more wake-ups nor wakes the task again for a message it already stopped for; the record is cleared when the task goes to sleep with nothing waiting for it. A request that is being served when the daemon stops is not returned. The bus gives the oldest request of every
team to whichever team's worker asks: a look at a team holds the requests of other teams while it reads the queue on, and
puts them back before the next team is looked at. The notices still in the outbox when the daemon stops are lost, and so is why a queued run starts (a wake-up, an item,
a request): it then starts as the plain continuation of its task. The owner's loop waits for each `agentbus` and `git`
call of a look at a team (an `agentbus` call for at most `15` s): a bus that hangs leaves the socket unanswered that
long.

Not done here, by item: the dashboard (9.1). A resumed agent's checkpoint is not read from the agentbus yet. opencode's `session export` for a
resumed session whose result it did not print is not done.

### Pull requests, reviews and the merge (the autopilot)

The default `--autopilot` loop, in the owner (`owner/rounds.rs`), is: the lead's branch is pushed and opened as a pull
request, the configured number of investigators from outside the team review it, and their verdicts lead to the merge and the next item, to a
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

**Description ownership** (`daemon/pr/description.rs`, `owner/rounds/publish.rs`). The lead writes
`tasks/<lead>/description.md`, a bounded UTF-8 file (at most 1 MiB), outside the worktree. Its initial and resumed
prompts name this file. `pull_text_of` combines the harness header, the lead contribution with its Size and Gate
sections removed, `Git::pull_request_size` at the recorded head, and `measured_sections` from the gate record.
The existing TEST-FIX disclosure block follows. A missing or empty contribution uses `harness/pr/no-lead.md`
and does not stop publication. A read error is reported with the file path. Manual GitHub body edits are
replaced by this assembled description. `refresh_descriptions` runs independently of the publication plan for
open rounds in review, fixing or approved, so a file-only change is published too; it only uses a gate record
matching the remote PR head. It patches GitHub only when the rendered body differs.

Size uses the PR-1 diff against the fetched base at the explicit commit, with test files, inline trailing test
modules, snapshots, lock files and pure moves excluded. Gate records retain their existing JSON format:
`Store::save_gate_record` first saves the preceding record in optional `gate-previous.json`, then atomically
replaces `gate.json`. A successful clean record preceded by a failure at the same commit renders the second-run
notice. Older homes without the sidecar load as before; no state-schema migration is needed.

**The gate of a lead that is done** (`agent_core::decide::publish::gate_step`, `owner/rounds/publish.rs`). A lead need not have
run the gate. When its last run ended with `STATUS: DONE` and no record passes at the tip of its branch, the harness acts instead
of waiting: with no record of the tip it runs the gate itself, once for a tip, through the gate service (the record and the
history are those of any gate); with a record of the tip of a gate that failed or timed out — the lead's own or the harness's — it runs the gate once more,
once for a tip (`GateStep::Retry`, logged as `gate_retried`): a check that fails for a reason outside the commit and then
passes costs no run of the lead, which may not hide it by running the gate again itself; with a record of the tip that is still
no pass, or of a pass on a tree that was not clean, it sends the lead back with a run whose prompt
(`harness/pr/gate-not-passed.md`) says what the gate said and where its output is; a branch with no commit of its own, and a gate that ran on another commit than the tip (the worktree has another
branch out), send it back as well. A lead that is done with a gate that passed and a branch that contains the branch of none of the
finished items is sent back as well, with the branches to merge (`harness/pr/branch-not-joined.md`): without that nothing would
be published and nobody told. After three times in a row (`GATE_SEND_BACKS`, counted together) the human is told, once for a
tip, and the lead waits. A lead that ended with `STATUS: SLEEPING` waits for its team and is not done: its branch is neither gated nor sent back,
and is published only when the gate it ran itself passed at the tip. What the harness remembers of its own gates and of the
times it sent a lead back is not in the state: a daemon that restarts starts over.

**A reviewer that cannot work** (`agent_core::decide::reviewer::replace_now`, `owner/rounds/replace.rs`). A review pass waits
for every review that was asked. At every look the harness finds the reviews that are still to come and whose reviewer's task
failed, was stopped, or has been paused for `[review] replace_after_minutes`: each goes to another free investigator that is not
of the team and not of the pass (`ItemMut::replace_review` puts the new review in the place of the old one, so the reviews that
came stay), with a worktree at the head of the pull request, and the history records it as a review asked. The task of the
reviewer that was replaced is left as it is: a report it still makes answers no review, is logged (`review_report_late`) and is
not taken. When no investigator is
free the human is told once for the review (`harness/review/notice-no-verdict.md`, which also tells of a report without a
verdict). The reviewers of a team whose lead was stopped or failed are not replaced: stopping a team starts nobody.

**A team that sleeps** (`agent_core::decide::asleep::asleep_step`, `owner/rounds/asleep.rs`). A coordinator that gives work to a
member and sleeps, and a member that wakes, does nothing and sleeps again, leave a team in which each waits for another. At every
look the harness asks: is an item in progress or being fixed, does every task of the crew wait (none queued, running, asking the
human or paused), did the lead's last run end otherwise than with `STATUS: DONE` (a lead that is done handed its branch to the
harness), and did the last run of the crew end `[watchdog] asleep_minutes` ago or more. Then the lead is woken with a run whose
prompt (`harness/pr/team-asleep.md`) says that nobody works and how every task ended; after two such wake-ups in a row
(`ASLEEP_WAKES`) the human is told instead (`harness/pr/notice-team-asleep.md`), once for every time the team falls asleep again.
The count starts over when the lead ends with `STATUS: DONE`, and with the daemon.
When the lead has reported `STATUS: DONE` and waits, but an item in progress or being fixed stands behind a failed member, the lead cannot fix it and waking the lead would do nothing. The harness tells the human immediately, once per item (`harness/pr/notice-standing-item.md`), with the member's last-run detail line, showing how to speak to the member and resume it (`agent say <task>`, `agent resume <task>`) or stop the team (`agent stop <team>`). Resuming the failed task clears the deduplication for items assigned to that task.

The first pass of a newly opened PR is independent: reviewer runs have no Agentbus MCP server, including replacement
and report-correction runs. The harness keeps reports locally until all verdicts arrive, then posts them in request
order and updates the findings comment. The request forbids reading peer reviews or contacting other reviewers. Later
passes retain the bus and post each report when received. Schema-three ledgers have no frozen `review_count` and retain
two reviews, immediate publication and bus access until their PR ends; new PRs store the configured count.

Review and verification requests use `review_gate` to attach the matching lead gate's commit, finish time,
result and log path. They forbid repeating that gate and allow single tests to investigate a finding. A stale
or missing record instead renders the request to run `agent gate`; a red gate report names the step and log line.

**Reviewers** (`daemon/review.rs`). The review pass of an item in review starts empty and is filled once (`ItemMut::ask_reviews`) with
`[review] min_first_full_reviews` investigators (default two). All investigator requests, including verification,
second opinions and replacements, use `decide::investigator::choose`. The daemon projects the activity history
of every team into the pure selector and maps its chosen names back to team-specific worktree slots.

Only running or queued work reserves an investigator across teams. Accepted pending reviews and opinions count
as queued work before their reusable task is scheduled. Sleeping coordinators and members may serve another
team, but never their own. Within the same team, an investigator whose reviewer task is `Stopped`, `Failed`, `Paused`, `Questions` or `Done` is
not offered again (`slot_in`): the state keeps one task per investigator in a team, `release` queues only a waiting
task, a stopped or held task is not started against the human, and `Done` is final. Such an investigator is free for
every other team. Crew and
caller exclusions (including the reporter, answered pass and prior opinion attempts) are permanent.

Candidates with no team activity precede those with activity; then the oldest latest team assignment wins,
followed by the oldest latest assignment anywhere (never assigned first), then name. All activity kinds count,
regardless of outcome. Everyone asked at the team's latest assignment time forms the latest request: both
reviewers of a simultaneous pair are deferred. If too few others remain, these candidates are appended in the
same order. If still insufficient, the whole request waits and its round names a free independent investigator.
No random draw is used. Old states continue through the activity migration described above.
A reviewer is a task of the team with its own worktree at the reviewed head. It ends `waiting` and can be reused
in the same session. A verification request replaces the full-review request after fixes: open numbered findings
and their checks, `git diff <previous reviewed head>..HEAD`, the lead's last fix message, and the gate. If history
was rewritten and the previous head is no longer an ancestor, it gives `git diff <base>...HEAD` and the reason.
Verifiers answer every open finding, judge the changes and may add missed findings of any severity. They do not
repeat the full review. The dedicated request in `harness/review/verification.md` carries these instructions.
New ledgers persist this context and the additional fix allowance granted by the human; migrated PRs retain their
previous draw, full-review requests and one-pass renewal until they end. The first-pass prompt remains the
working context, the review rules, the brief, the request (pull request, branch, base, the commit under review) and each item of
the pull request with its expected result and checks. Its final message is the report: the first `VERDICT:` line says
`APPROVE` or `REQUEST_CHANGES` (markup and case aside), and its findings are read (below), or the report is asked for again, twice, and then the human is told. The
report is kept in `tasks/<reviewer>/review-pr<N>-pass<K>.md` (`K` fix passes the pull request had), posted on the pull request
as a **comment review** with the verdict in its body (the harness opens the pull request and posts with one account, which
GitHub does not let approve it) and recorded in every item of the pull request (`review_submitted`). A report is matched with
the review it answers by the time its run started: the latest review asked of its task at or before that time that is still
pending. A report that answers none (a reviewer that was replaced, or woken with nothing to review) is logged and not taken.

**Findings** (`agent_core::review_findings`, `owner/rounds/findings.rs`). A report lists its findings as
`N. [SEVERITY] RULE place — problem → fix`, with a `CHECK:` line under each, and the severity is one of `[BLOCKER]`,
`[MAJOR]`, `[MINOR]` (what each means is in the review rules, `prompts/investigator/code-review.md`, §6). A report goes back
to its reviewer, with the budget of a report without a verdict, when a finding has another severity or none or no check, when
the verdict is `APPROVE` and a finding is a `[BLOCKER]` or `[MAJOR]`, when it is `REQUEST_CHANGES` and none is, or, in a pass
after the first, when it leaves an open finding without its `FINDING n: RESOLVED` / `FINDING n: OPEN — why` line
(`harness/review/findings-invalid.md` says which line is wrong); a line that looks like a finding outside the numbered list under `FINDINGS` is refused rather than dropped, and fenced code blocks are not read. The findings of a pull request opened by this build are kept
with its items (`findings`, [`state.md`](state.md)): numbers that never change, counted over the pull request in the order the
reviews were asked and then the order of the report, and a status, `open` until every reviewer of a later pass says
`RESOLVED`. A pass is numbered when all its reports are in. The text of a fix pass lists the open findings with number,
severity and place, and says which block. BLOCKER findings require code fixes; a MAJOR may instead receive one explanation, decided unanimously by the verifiers. A review pass after a fix
pass is given the open findings with their numbers and checks. The harness keeps **one comment** of its own on the pull
request, made when the first findings are in and edited in place after every pass (`agent_runtime::github`, a `PATCH`): one row
per finding with number, severity, rule, place, the text cut to a line, status and reporter, the severity in bold and colour
(`[BLOCKER]` dark red, `[MAJOR]` light red, `[MINOR]` orange; the reports and the state carry the plain words). A comment
that cannot be posted or edited is logged and holds nothing. `[MINOR]` findings that are open stay in it when the pull request
is merged. A round opened before the update has no ledger and ends by the verdict-only rules it began with. `agent status
<round>` shows how many findings are open of each severity.

**Finding decisions** (`review_response`, `state/findings/disputes`, `owner/rounds/responses.rs`). New PR ledgers
carry optional `disputes` state; its absence preserves the protocol of an active migrated PR. A fix response must
answer every open blocking finding with `FIXED`, or an unused MAJOR with `EXPLAINED — why`. Validation is atomic:
an invalid complete response changes no explanations. The checked run and correction count survive restart; two
correction runs are allowed, without consuming a fix pass, and exhaustion holds publication and notifies the human.
The dedicated verification request (`harness/review/disputes.md`) supplies the explanation and requires ACCEPTED
or REJECTED with evidence. Unanimous acceptance makes the finding MINOR; any rejection keeps it blocking and
requires a fix. The explanation and individual votes remain in state and the table. RAISED changes severity upwards
only, with reviewer and evidence retained in history. For an accepted explanation, a diff against its accepted head
must show a change to the affected file (including a rename) before raising is permitted. No second-opinion path
is offered in this slice. State schema 6 migrates schema 5; history migration 008 preserves records, foreign-key
links and the search index while admitting the four finding-decision event kinds.

**Second-opinion foundation (part 1).** `Review::allow_second_opinion_on_blocker` defaults to true.
`review_response::opinion::{parse_lead, parse_report}` validates whole lead messages and investigator evidence
without changing state. `Evidence` prevents empty reasons; `Permission`, `OpenFinding`, `LeadAnswer`,
`OpinionReport` and `Outcome` carry the checked protocol. The daemon applies the checked opinion protocol to active fix passes.
`Findings::{request_opinion, assign_opinion, record_opinion, opinions}` persists waiting, task ownership,
replacement and completed outcomes through `OpinionRequest`, `SecondOpinion` and `OpinionStage`.
State schema 7 migrates schema 6 with an empty opinion map. History migration 009 adds the run reason and
three outcome events while retaining earlier records and search.

`decide::opinion::after_opinions` returns `Next`: wait for every requested opinion; return confirmed blockers
to the lead in the same fix pass until answered FIXED; then verify the changed tip. If every blocking finding
was not confirmed and the code did not change, `Next::DecideUnchanged` lets the later integration decide the
pass without a new tip or redundant reviews. Otherwise an unchanged tip waits. Decisions filter by fix-pass
number and never increment the pass count. The owner assigns a free investigator outside the team, excluding the reporter, the reviewers of the answered
pass and all earlier investigators for this request. Its task runs at the requested head without a bus.

**Second-opinion recovery.** Failed runs, missing final statuses and exhausted report corrections record their
cause with the assignment and return the request to waiting. The same deterministic activity order supplies at most
two replacements. A lack of candidates leaves the request waiting rather than dropping it or notifying the
human prematurely. Opinion inactivity uses the review replacement interval and resets when output changes;
a silent run is cancelled before its request is reassigned. Replacement stops the old queued, waiting or
quota-paused task and removes its scheduled wake, including during startup recovery. The third failure retains the assigned stage and
sends the catalogued human notice once, listing all assignments and causes. Late reports from replaced or
exhausted assignments are rejected.

At startup the owner resumes an unanswered task queued for release with a surviving worktree.
A task that cannot be queued, including one awaiting human answers, or a missing task/worktree records an
interrupted assignment and triggers the bounded draw. Accepted opinions
are skipped. Recovery never changes the pass count or the FIXED answers. State schema 9 migrates schema 8 with
empty attempt history, keeping legacy assignments and their accepted answers readable. `agent status` exposes
the finding, owner and assignment time, or the wait for a free investigator. Each `second_opinion_asked`
event names the selected investigator. History schema 10 adds `second_opinion_ended`, recording each failed
investigator, assignment number and cause. The story renders replacements and exhaustion with evidence
previews; the migration preserves existing records, round links and search.

Every posted review starts with the investigator's name and its configured CLI and model
(the model is omitted when none is configured), a verdict heading, and the review pass and commit. The approval
heading is green bold `APPROVED` with one `✅` (the mark of the status `done`: the double mark is the notification's "ready to
merge", which one review is not); changes requested are red bold
`REQUEST CHANGES` with its `❌`. A horizontal rule separates the head from the report, whose `VERDICT:` line is
removed only for publication. The saved report and history keep the reviewer's original verdict. The layout and
both verdict texts live in `prompts/harness/review/`; changing their wording needs no code change.

Before sending a pull request description (new or updated), an updated-branch comment or a review to GitHub,
the daemon links commit IDs and repository files to GitHub, using the owner and repository from `origin`.
The pure `agent-core` rewriter shares one Markdown walk for both kinds of reference, so generated links are never
processed again. Only words of 7–40 hexadecimal digits that git resolves to a commit become commit links, with
the original spelling as the label and the full ID in the URL.

A repository-relative path becomes a file link only when it names a blob in the tree of the commit the text is
about: the reviewed commit for a review, or the pull request head for descriptions and updated-branch comments.
`src/lib.rs:7` adds `#L7`; `src/lib.rs:7-12` adds `#L7-L12`. Line numbers need not exist in the file. File labels
preserve the author's spelling and single backticks; unmatched Markdown label brackets and literal backslashes
in bare labels are escaped so the links render correctly. URL path characters are percent-encoded. Trailing sentence
punctuation is kept outside the link. Directories, missing files, absolute paths and paths containing `..` stay
as written; files are never searched for under another name or at another commit. Git supplies the committed tree
in one listing per text, rather than starting a process for each path. A truncated listing is a lookup failure;
the harness never treats an incomplete tree as the full set of files.

When a published description is rebuilt, generated file links in identifiable
harness-authored lines are rebuilt at the new head only when their destinations belong to the publishing
repository from `origin`. Generated commit labels are recognized regardless of hexadecimal letter case.
Other existing links retain their destinations. A description
that does not change retains the links from its original publication.

Single-backtick references are linked too, including file names containing spaces or `#`; longer inline code spans,
fenced code blocks (including blockquote and list containers), existing links (including bare URLs, email autolinks,
quoted titles and shortcut references), HTML tags and comments, and longer words stay as written. Reference
definitions are read across the document, excluding fenced code examples. If a lookup fails, the original text is
sent and the failure is logged with an existing event; linking never prevents publication. Saved reviewer reports
and their history retain the author's original text.
Review and developer rules require language-labelled fenced code blocks (`text` for output); the harness does
not infer languages.

**What a verdict leads to** (`daemon/merge.rs`, the round machine). A review pass is decided only when its configured number
of verdicts is in. The count is frozen in the PR ledger when it opens. Every verdict must approve for the PR to be `approved`; with at least one request for changes the next fix pass starts, until `[review] fix_passes`, and then
the human is asked. A reviewer that cannot give its review is replaced (above), not left out.

- **Merge.** In autopilot the pull request is read from GitHub and merged with a merge commit at the head it carries (a merge
  made elsewhere or by the human completes the items as well); the items are `done`, the base branch is fetched, and the next
  item starts from the remote's base when it is ahead of the checkout's, so that it builds on what was merged. A merge GitHub
  refuses for a reason waiting does not change is told once with GitHub's reason and not tried again. A refusal that names a
  conflict with the base (a 405 saying the pull request has merge conflicts, is not mergeable, or whose base was modified) is
  a merge conflict pass instead while `[merge] conflict_passes` allows one: the item keeps `merge_conflict` in its state
  (`merging_base` with the approved head, or `asking_human`), the lead is woken with `harness/pr/merge-conflict.md`, and the
  pass is judged when the lead is done: a tip that merges the base and passes the gate is pushed, the files the lead changed
  beyond the base merge are checked against `Allow:`; the merge is tried again when they are allowed, the review is reopened
  when they are not (a verification review of what changed since the approval), and the lead is asked to put the question
  in its questions file (`harness/pr/merge-conflict-ask.md`) when the passes are spent or it gives no result. The pass is
  read from the state, so a restart goes on from it; a pull request closed
  without a merge is told once. An approved pull request that waits is read again every 30 seconds.
- **`--no-merge`.** Nothing is merged: the human is told once that the pull request is approved. The next item of the feature
  starts at the approval; what depends on the pull request waits until it is merged, which the daemon finds when it reads it.
- **Fix pass.** It goes to the lead, which has the integrated branch: it is woken with the open numbered findings and
  reviewer reports, the configured allowance and branch (a solo agent switches to it first). With the gate green at
  the checked tip the same PR is updated and a rotating verification pass starts. A fully approved first pass merges
  without verification.
- **The question.** When a reviewer still asks for changes after the configured passes, the lead is woken with the reports and the
  questions file to write; its question reaches the human like any other (`agent answer`). The answer grants
  `[review] fix_passes` more fix passes. The notice lists the allowance and open blocking findings. A lead that ran
  and put no question leaves the human told directly, once.
- When every item is merged the team ends: its tasks, the reviewers among them, are `done` and the human is notified.
  When it ends so — every item done, at least one by its merge (by the daemon or by the human; the others ticked `[x]` in the
  list), and no round open — git removes the worktrees of its
  tasks that are not running (`git worktree remove`, which refuses a changed checkout). Their branches stay, local and on
  origin. A worktree with changes not committed, or with commits no branch of origin has (origin is fetched first, with
  `--prune`), is kept and logged (`git_call_failed`, `warn`, with the reason), as is one git cannot remove; neither stops the
  team from ending. A team that ends any other way, stopped or failed, keeps every worktree.
- **Build directory release and disk space limits.** After every round closes, any `target/` build directory in worktrees of its tasks is removed (`build_directory_removed`, `info`, logging reclaimed bytes). When a reviewer run completes and free space is below `disk.low_free_gb` (default 100), the reviewer's `target/` is also removed while keeping the reviewer's worktree. Only `target/` located inside a harness worktree is ever removed (never symlinks, never outside `AGENTS_HOME`). Free disk space is monitored in `AGENTS_HOME`: when free space drops below `disk.low_free_gb`, a notification is sent once listing the five largest worktrees and `agent status` reports free space; below `disk.min_free_gb` (default 20), `agent run` is refused with a diagnostic naming free and required space and what to clean, open rounds wait with a single notification, and waiting rounds resume automatically once free space is restored.

**Known limits.** The owner waits for GitHub and git calls (GitHub's own timeouts, at most 30 s an attempt; a push at most 10
minutes): a call that hangs leaves the socket unanswered that long. The merge is at the head GitHub carries, so a commit someone
else pushed after the reviews is merged as well. A review that cannot be posted counts and is only logged. A reviewer whose run
fails or is stopped is not replaced: the human resumes it. A solo agent that takes the next item while its first is in review
switches to its fix pass's branch by what the prompt says, not by the daemon. The reports are cut at 20,000 characters.

A completed fix handover at the reviewed tip is decided by the pure `decide::fix::fix_step` table. A claimed
FIXED answer or a legacy unanswered blocker returns the lead with `harness/review/fix-no-commit.md`; all blockers
explained or closed by independent opinions permit verification at the same tip. Handover corrections share the
publication correction counter (three returns), including a stale gate after the harness has no gate left to run.
Exhaustion sends `harness/pr/notice-handover-held.md` once for the held tip, naming the round and the lead's
`agent say` command. Publication clears the counter. The counter and notification deduplication remain transient,
like the existing publication corrections; a daemon restart may issue a fresh correction.

The owner's `rounds/waiting.rs` projects task activity, pending opinions, durable response corrections and its
queued gates/publication actions into `decide::wait::round_wait`. This pure decision returns no wait for active
or closed rounds and an owned obligation or a defect for idle open rounds. `RoundDetail.waiting` carries its
catalog text to both status views. After scheduling each team's work, an unowned idle round logs one error using
`daemon_task_failed` with `operation=round_wait` and its round ID. Deduplication lasts for the daemon's lifetime.
No wait label or publication correction counter is added to `state.json`.

An accepted delivery claim is optional durable round metadata naming another round's PR in the same team.
The daemon binds a claim to the coordinator run's round references, checks the clean member checkout against
fresh base and published PR commits, and refuses unverifiable claims before publication. Accepted claims are
excluded from publication and idle-work alarms. Their merge obligation survives restart, and separate polling
observes the delivering PR even while its owner is in review or a fix pass. A merge completes the checked item
and round before normal team completion and worktree cleanup. A stop of the whole team abandons the round but keeps
the claim: the item stays out of publication, the delivering PR is still polled after a resume, and its merge completes
the item without recording the end of the round a second time. An observed unmerged close ends the claim in a
checked transition and saves `delivery_closed_at` before notification or scheduling. The abandoned round retains
its delivery record. Polling recovers a marked round that is still the latest for its in-progress item: notify the
lead from the team prompt catalog, open a fresh round for the same item (or split part), and queue the member.
The new round and queue state are saved together. Recovery retries after a restart without consulting the old PR;
therefore a later reopening cannot revive the claim. Notification precedes the new round, so a crash after sending
but before saving may repeat the message. A stopped member keeps the recovery pending until resumed. Publication
and subsequent merge use the ordinary item lifecycle. A returned item publishes the integrated, gated tip on a
branch named for its fresh round, independently of the closed PR and any other item still attached to that PR.
A failed bus send logs `bus_down` and leaves the saved notification and reopening obligation for the next poll;
it does not stop the owner loop.

### Snapshots (`agent-runtime`, `agent`)

A snapshot keeps what a worktree holds when its run was cut, so that nobody else can lose it. The daemon takes one of
every task whose run did not end when it starts, when the round that run worked for is still open. The snapshot is a
commit of everything in the worktree (modified, staged, deleted, untracked and binary files; ignored files are left out),
made in a scratch index with the repository's filters, hooks and filesystem monitor switched off: the worktree, its index
and its files are not changed. The commit is kept under `refs/agents/snapshots/<round>/<task>/<n>`, `n` counted from 1,
and its parent is the head; the history records it ([`history.md`](history.md), `snapshots`). A snapshot above
`[snapshot] warn_mb` is still taken and logged as `snapshot_large`; a failure is logged as `snapshot_failed` and the daemon
goes on. Nothing prunes the snapshots yet, and nothing pushes them. `agent status <round>` shows the last snapshot of each task.

### The history

The state file says what is; the history says what happened. It is one SQLite database, `history.db` in the home, written
by the daemon and read by anyone: a team with its task list and its plan (who got which role, and how: named, by the flag,
by the `Coordinator:` line, drawn among whom, the configured order), a round with its item in full, a run with its agent, its
reason, the whole prompt, its worktree, the command line its CLI saw, every step, what it changed in git (commits, files,
what it left uncommitted) and how it ended. [`history.md`](history.md) documents every table and column, the pragmas and the
queries to ask it with `sqlite3`; [`history.schema.sql`](history.schema.sql) pins the schema and a test compares it with a
fresh database.

The records are the types of `agent_core::history` (with the limit of a text and `explain_plan`, which says how each participant
of a team got its role), the database, the writer and the reader are `agent_runtime::history`, the commits and files of a run
come from `agent_runtime::git` (`work_changes`, read without changing the repository), and the daemon opens the history at its
start and closes the writer at its end: the owner records teams, rounds, runs and their ends, the run engine the launch, the
steps and the changes.

**One writer, never in the way.** The daemon is the only writer, on a thread of its own behind a queue of 4,096 records; a record
or a batch of up to 256 per transaction. Storage never delays the work: a record that cannot be kept — the queue is
full, the disk, the file, a database of a newer version — is counted as lost and its cause is told once on the standard error;
`agent status` shows the count (`Reply::Overview`, only when the history is not whole). A daemon killed with `SIGKILL` leaves a
file that opens and holds whole records; the next one ends the runs it finds going. Accurate Git attribution uses the
human-authorized capture exception: a `2`-second overall budget at each run boundary, plus at most `1` second to reap a
killed Git process. Captures that exceed it are abandoned and counted before the next run starts.

**Readers need no daemon.** `Reader::open` opens the file read-only; `agent log` (8.5, 8.7) uses it. The schema has a
version (`PRAGMA user_version`) and forward-only migrations in code; a database of a newer version is never written.

**Known secrets are redacted.** The launch records environment variable names, never values. The writer replaces configured
credential values in free text with `[REDACTED]` before SQLite receives them. Credentials in structured identities cause
counted omissions; the warning contains no credential. A merged fake-CLI round echoes a credential and searches both the
database and live WAL bytes. See [`history.md`](history.md#what-is-never-in-it) for scope and the exact-recording exception.

**Events and the story.** Beyond what runs do, the daemon records the happenings of a round as events (`agent_core::history::HistoryEvent`:
a gate, the checks and what followed, a pull request opened, pushed, ready, reviewed, approved, merged, handed over or closed, a fix
pass, a question asked or answered, a control, a notification, the end of a round or a team). Every record of every kind has a place
in one ledger (`history_records`), which orders records of the same time by the order they were written. A record that cannot be
written or captured is counted as lost under its cause, like every record of 8.4. Database refusals receive best-effort loss
notes when writing recovers; stories also identify missing run changes. See [`history.md`](history.md#what-happens-when-the-history-cannot-be-written) for the limits
of durable loss reporting.
`agent log --round` and `--team` ([`log.md`](log.md)) read the ledger directly and give it to `agent_core::story`, a pure function
from records to lines (or JSON lines); there is no socket call in it, so it works without the daemon.

### Debug diagnostics: commands and pure decisions

`agent debug (--round ID | --team ID | --task ID) [--out DIR] [--daemon-log FILE]
[--stale-minutes N]` resolves one team and writes a new diagnostic directory beneath `<home>/debug/` or `DIR`.
It reads the state and existing status protocol, tasks and runs (including prompts, final messages, steps, raw
output and standard error), questions and answers, gates and their outputs, check records, worktree git facts
and team history. It does not change task state, emit history events, fetch or move refs. A daemon that does not
answer does not prevent collection. A missing target fails without leaving a directory.

Secrets are loaded before any artifact is written; an unreadable secrets file is fatal. Every artifact, including
the summary, replaces exact secrets-file values and values of environment variables whose names end in `TOKEN`,
`SECRET`, `PASSWORD` or `KEY` with `<cut>`. The secrets file itself is never copied. Other read failures become
assessment read failures and do not stop unrelated collection.
Task files and subprocess output are bounded to 4 MiB each, task bytes to 64 MiB, directory entries to 4096 per
task and nesting to 16 levels. Task symlinks are not followed. Diagnostic history copies the database and its
WAL/SHM files (at most 64 GiB each) into a temporary directory before opening SQLite, preserving committed events
while the daemon runs without creating or writing side files in the home. Up to three snapshot attempts
accommodate source changes before they are reported as unreadable.

The daemon process is selected by the owner of `daemon.lock` in Linux procfs, never by its name. Bounded
observations include its start time, command line, environment names without values, running executable version
and whether the executable on disk is newer. Its log is the explicit `--daemon-log` file or `<home>/daemon.log`;
the bounded tail retains all team/task lines, plus warnings/errors in its last 500 lines. Missing defaults
and unreadable explicit files are distinguished. No process is signalled or restarted.

Task details include the latest run's reason, final message and steps. Questions are sorted numerically; gate
outputs retain their paths and modification times. Missing worktrees of each task's associated merged round
are expected, with retained branch tips and commits over the base read from the caller's clone. An unpublished member branch
is expected; coordinator origin differences remain assessable. Empty history and unreadable history have
different section text.

Collection covers machine metrics (load average, memory and swap, filesystem space and inodes for home and temporary directories, daemon descendant processes with age and redacted command lines, and tasks and worktrees directory sizes) and the fifty newest untracked files per worktree from a single git listing. Filesystems at least 90% full by space or inodes trigger an anomaly. Complete untracked listings are saved as `untracked/<task>.txt`, separate from copied task files.

The pure layer receives observed facts, the collection time and a UTC offset from its caller. The CLI passes the
machine's local offset; summaries show both UTC and local times.
Unavailable facts are distinct from observations that something is absent. Known anomalies are reported from
those facts without asking the daemon, running git or reading the machine's clock. A summary with no known
anomaly says "none of the known ones", which does not establish that the team is healthy.

The summary puts anomalies first, followed by daemon and versions, team/items/rounds, the tasks table, the last
runs and questions, git, gates, history, daemon log, bus, notifications and machine. It stays below 100,000 bytes;
individual long text fields share the remaining budget and retain their ends with an explicit cut marker.
Anomalies are deduplicated, and structural headings, all task rows and gate records remain intact. Only the
last three gate outputs for each task are included. If mandatory facts and cut markers alone would reach the
limit, the renderer returns `SummaryError::RequiredFactsTooLarge` instead of omitting facts. Times include UTC
and the injected local offset; the collection timestamp is the only reference for activity ages.
The collector retains full details in separate artifacts and retries `RequiredFactsTooLarge` with shorter
presentation fields, including unreadable-part reasons and commit lists. Secret redaction precedes both
presentation shortening and renderer byte budgeting, including expansion of short secrets to `<cut>`. It fills
`diagnostic_assessment::Assessment` once, then maps its
deduplicated findings and read failures to the presentation input; summary fields are for presentation, not
a second source of anomaly decisions. A known missing origin ref is deliberately reported as differing from
the local tip, including a never-pushed branch; an unavailable origin observation produces no such finding.
These new facts are diagnostic inputs, not a new persisted state or socket format.

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
| an item is given again (its member left) | keeps its existing round | |
| a verified delivery PR is observed closed without merging | abandons the claim round, then opens a fresh round for the same item or split part and queues its member | `round_closed`, then `round_opened` |
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
| any task, for a fix pass, the answers of the human, or a resume | the rounds its last run worked for; a member queued after an ended delivery uses its replacement round even after a restart |

A run names only rounds of its own team, none twice. The daemon that restarts reads them from the state and goes on with the same rounds.

**What the state checks**, by its operations and again when it is read: an ordinary item has at most one round apart from abandoned rounds with ended delivery claims;
a split item has one round per started part apart from ended claims, and at most one open round. An item has a round as soon as it leaves `pending` (`Team::advance_item`; an item the list marks `[x]` has none); a round belongs to an item of its team, or is the one
round of a list without items; an ordinary round closed as `merged` belongs to an item done as merged. A merged split
round retains its own pull request while the parent advances. An ordinary round closed as `approved` belongs to an item
that is approved or done as merged; a round is not closed before it was opened; a run names only the rounds of its own team. The full list is
in [`state.md`](state.md).

**The CLI.** `agent run` prints the round of each item it started, after the tasks; `agent status` lists the rounds of every team under
its line, and `agent status --running` only the teams that have an open round, with those rounds. After the line of a round the
command says when the round started and, once it is closed, how long it took; for a round that goes on it says where the work is
(`agent_core::round_work`): it asks the daemon to show each such round and reads from the history whether a draft pull request is
open, which the state does not keep — the daemon and the protocol are the same as before. `agent status <round>` (the protocol's `show` of a round) shows its item, its state, the tasks that worked for it with their
role, agent, worktree and branch and their runs, its pull request and its review passes (the reviews of the pass that was approved are
not kept once the item is done: the history of a later item has them).

**What the investigators did.** The summary of a team in the daemon's answer to `status` carries the team's recorded
activities (`TeamSummary::activities`, optional and absent while a team has none, so that an answer without them is the
one a daemon before this change writes): each activity's investigator, task, rounds, request, when it was asked and how it
ended. `agent status` reads them from that answer and nothing else; nothing on disk changed. The lines of the coordinators,
members and workers stay those of their status; the line of a reviewer is its newest activity by the time it was asked — a
review that goes (`working`, with the last activity of its run), a report (`done, <n>th review finished <time>`, the
ordinal counting the reviews the investigator gave for the round) or an assignment that ended without a report, stopped or
failed, with the time and the reason the daemon kept (that outcome takes precedence over the status of the task, which is
`waiting` between reviews). A reviewer task without a recorded activity keeps the word of its status. `agent status
--investigators` (a view of its own, which takes no target and no `--running`) shows, for every investigator of
`[investigators.NAME]`, the last two of its activities in the teams that are not closed (a team with a round that goes on,
as for `--running`), newest first (`agent_core::kinds::ActivityKind` names the kind of each): `done at <time>, <what>`,
`working`, or `stopped at <time>, reason: …` in the words of a reviewer's line, and `free, no activity in an open team` for
one that has none. The ordinal counts the reviews that gave their report.

### Round checks before the review

Before the head of a round goes to the reviewers — the first time and after every fix pass — the daemon checks the round's own
diff and its commits through git, with no model, as the repository's `checks` table says ([`configuration.md`](configuration.md)).
A repository without the table goes straight to review. Nothing in the code names a repository or its layout: what is a test,
what is production code and what judges a round are the three lists of path patterns.

**When.** In the autopilot's publishing (`owner/rounds/publish.rs`), once the lead waits and its gate passed at the tip: for a
first round after the pull request is opened as a draft and before it is marked ready; for a fix pass before the branch is
pushed, so that a head the checks hold back is not pushed and the reviewers keep the head they reviewed. The facts are the
files, the commits and the added lines from the merge base with the fetched base branch to the tip (`agent_runtime::git`:
`round_facts`), the authored parts of the current GitHub description, and the fresh lead contribution. The
harness header is excluded from authored disclosures, so merely listing a path in the task does not authorize it.

**Task-authored allowances.** `task_list::TaskList` parses `Allow:` headers into validated `TaskAllowance`s.
Their matching is rooted, unlike configuration `PathPattern`s with bare names at any depth; unsafe patterns that
reach rule files or workflows are rejected at parsing. `Team::allowances` exposes them from the team's persisted
original `TaskListText`, so neither item changes nor recovery can lose them. State schema 10 migrates versions 1–9;
a schema-9 team without allowances keeps none. The task-list string remains the wire and disk representation,
including the existing `Reply::Team` response used by `agent run`; `Reply::Started` is unchanged.
The checks filter covered path issues separately from per-path human answers; marker and missing-test issues
cannot be allowed this way. `checks.json` keeps its schema. The harness writes `## Allowed beforehand` from the
round's changed paths and covering directives, strips forged copies from lead contributions, and excludes this
section from marker disclosures. The CLI prints the original directives and reviewers judge their actual changes.

**The rules** (`agent_core::decide::checks`, each with the case that raises it and the nearest that does not):

| Rule | Who may allow it |
|---|---|
| an existing test file is changed other than by added lines, removed or renamed — a change of its mode (an executable bit) is a change too, with lines added or none (the facts read it with `git diff --raw`) | only the human |
| a protected file is changed in any way | only the human |
| a test file a `RED:` commit of the round touched has lines removed or replaced by a later commit (a descendant of it in the history), unless that commit touches test files only, its subject starts with `TEST-FIX:` and the description names it, by its ID or at least seven digits, under `## Tests changed after RED` (TDD-14). A later merge counts only its diff against its first parent, when that parent has the `RED:` commit, and only paths that the round changes against its base. Incoming base changes and `git merge -s ours` do not edit a frozen test; an edit inside a merge or a conflict resolution that drops a frozen line still does | the team, or the human |
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
- both kinds, with team passes left — the human is asked about only the human-only findings first; after the answer,
  the remaining findings go to the team. Asking the human spends no team pass;
- both kinds, with team passes used up — the human is asked about all findings.

**The message to the team.** The lead is woken (`Why::Checks`) and told the reasons — one for each issue, from
`prompts/harness/checks/` — with the pull request, the pass and what to do: settle every finding, get the gate green, end with
`STATUS: DONE`. It is made when the round is held and kept in the record, so a run that starts after a restart of the daemon is told
the same.

**The question to the human.** The daemon cannot put a question for a task that waits (the status table has no `waiting →
questions`), so — as for the reviewers' used-up passes — the lead is the messenger: the daemon writes the questions file of
the lead's next round itself, one question for each finding kind (`Issue::kind`, including the `TestChange` variant)
and judge. Each question lists every path and finding of that kind, including removed lines and the edit and RED commits,
why the human is asked, and the recommendation to refuse. It wakes the lead with the path and has it end its turn with
`STATUS: QUESTIONS` and change nothing.

The three explicit verdicts are `allow`, `refuse`, and `allow except <path> <path> ...`. The exceptions must be exact paths
of that group; only they are refused. A path mentioned by several groups is allowed only when every group allows it.
The CLI validates the entire file before sending any verdict, and the daemon uses the same validation for socket answers
and a hand-saved `**ANSWERED**` file. Invalid verdicts (including `ok`, an empty verdict, an unknown word or an unknown
exception path) leave the questions open and name what to correct. Ordinary questions still accept `ok`.

New files carry `[checks:v1]` in their title and a `**Checks version one:**` part in each question containing its subjects:
`p:<repository path>:` or `n:` for missing tests. Delimiters preserve spaces and non-ASCII paths through a Markdown round
trip; longest complete paths disambiguate exceptions containing spaces. The title also prevents a question whose membership
part was removed from silently reverting to the legacy grammar. This metadata lives only in the questions file; the
persisted checks record still stores the unique subjects and per-path allowances, with no schema change. Group membership
keeps a question's meaning when questions are reordered; a missing question refuses its subjects.

An unanswered file written by an older daemon has neither marker and is read by the legacy positional rules: one question
per subject, only a first word `allow` (case insensitive) allows; `ok` accepts the recommendation to refuse. Its existing
record and scheduled answer resume across a daemon restart without rewriting the old question. `grouped_decisions` selects
the grammar; `QuestionsFile::allowed_check_subjects` projects the validated group verdicts, and the daemon returns the
same per-subject `Decisions` used by the owner.

- *Allowed* — the subjects are recorded for the items with the head they were allowed at (it is not asked again for the same
  file, but it is when the round changes that file again), and the count of passes starts again, except after a question about
  the human's part alone, which leaves it as it is; such an allowance does not cover an issue of the team's about the same file,
  for example a lint suppression added to an allowed protected file. The lead, resumed with the answers, is told to change nothing and end with `STATUS: DONE`. The checks run
  again: the review starts when nothing remains, otherwise the team receives its remaining findings.
- *Refused* — back to the team, with what the human refused named, as the next pass; the lead's resumed run is told to undo
  what was refused.
- A lead that ends without putting the question is woken again, twice; then the human is told directly (`notice-not-asked`).

**The record** is `tasks/<lead>/checks.json` ([`state.md`](state.md)): the passes and what was allowed for each item, the
question the round waits on, what the lead is to be told. Nothing of the checks lives in memory only; the count of reviews and
fix passes of the state is not touched.

**Known limits.** The description is read from GitHub when the check runs, without the lines the harness itself wrote for the
items (their text names files in passing): the harness writes the description of the pull request it opens, so the name of a file with a suppression comes from the team
or the human editing it afterwards — the first check of such a round sends it back and the next reads the edit. The section
`## Tests changed after RED` is the harness's own (`agent_core::decide::disclosure`): it lists every commit of the round whose
subject starts with `TEST-FIX:`, with its ID and its message, between two comment lines that mark it; the checks judge the round
with that section in place, and the harness writes it into the description when the pull request is opened and at every update,
replacing the one it wrote before and touching nothing else. A lead has one question at a time: while one waits, no
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
in the worktree in a process group of its own, with the `PATH` starting with the daemon executable directory followed by `[binaries] agent_path` without duplicating it, its output (standard output and
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
A word to a task of a team on the bus goes with `agentbus send --team <team>`, and so do the notices the daemon writes
to a member (`member_finished`, the answer to a request): an agent can be in several open teams, with a session in
each, and the bus delivers a message bound to a team to the session of that team only. A word to a task whose team is
not on the bus (a solo task) goes with `agentbus send --for-task <task>`, which binds it to that task's session alone.
The bus refuses a direct message that names no team when its agent is in more than one open team.

`run` answers with the agents of the new crew that are also in the crew of a team that is not finished, and whose task
there is not done (`shared` of the `started` reply, absent when there is none); the CLI prints a line for each. The
team starts all the same.

**The watch is bounded.** The saves that wait for the owner are at most 256, each file once; a save that finds the queue
full, and an error of the system's watch, make the owner read every open questions file (the look it takes on a start),
so no save is lost for good.

**Known limits.** A save of a questions file that the system's watch limit would not let the daemon watch is not noticed
(`agent answer` and the dashboard still work); the daemon says so in its log. A save in the middle of a write can be read
half-way: the next event of the save has the whole file. The words of a `say` to a solo task that is `waiting` are read
when it next runs; nothing wakes it. A human's stop of a worker that serves a request answers the request as failed.

### Notifications

Every notification sent to the human follows a standardized layout rendered through `layout_notification`:

- **Title**: `<icon> <repo>`
- **Body**:
  ```text
  Repo: <repo>
  Agent: <agent> (<cli>, <model>)
  Time: <YYYY-MM-DD HH:MM>
  Task: <task>
  Team: <team>
  Status: <icon> <label>
  Note: <summary sentence>
  PR: <pull request URL>

  <details / body text>
  ```

The `Team` field is omitted for solo tasks (and worker tasks without an assigned team), and `PR` is omitted when no pull request URL is provided; neither is printed empty. When an agent configuration has no model specified, `Agent:` prints `<agent> (<cli>)`. `Time:` formats the local time using the daemon's host `local_offset`.

The notification statuses, icons and labels:

| Status | Icon | Label | When used |
|---|---|---|---|
| `Questions` | `❓` | `waiting for answers` | Questions are open: waiting for answers |
| `Decision` | `❓` | `needs your decision` | Human decision needed (e.g. check allowance, review limit exceeded) |
| `Failed` | `❌` | `failed` | Task or run failed |
| `Hung` | `⌛` | `hung` | Watchdog detected hang and stopped task |
| `Paused` | `⏸` | `paused` | Task paused for provider limits or rate limits |
| `Approved` | `✅✅` | `approved: ready to merge` | Pull request approved by reviewers under `--no-merge` |
| `NotMerged` | `❌` | `not merged` | Pull request merge failed |
| `Closed` | `🚫` | `pull request closed` | Pull request closed without merge |
| `NotPublished` | `📕` | `not published` | Branch publish failed |
| `NoVerdict` | `❗` | `review without a verdict` | Reviewer ended without a verdict |
| `Merged` | `🟣` | `merged` | Pull request merged on GitHub by the harness or the human |
| `Asleep` | `💤` | `team asleep` | Team asleep |
| `Done` | `✅` | `done` | Task or team finished |

Merged item completion is saved before its notification is queued. If saving fails, no merge notice is queued; after a restart, persisted completed items prevent a repeated notice.

## Process model

`agent daemon`, a systemd user service, is the only owner of the state and of every model run. Every other command
— `agent run`, `answer`, `status`, `stop`, `gate`, ... — and the dashboard `agent-web` are clients of its Unix
socket. One owner, no file races (CON-3).

- `agent run <agents...> "tasks" | -f tasks.md [--coordinator NAME] [--no-merge]`: one agent is a solo task, more
  than one is a team. `--autopilot` is the default: PR → configured reviews → merge → next item; `--no-merge` stops before
  the merge and hands the PR to the human.
- The coordinator: an explicit `Coordinator: NAME` line at the top of the task list or `--coordinator` wins;
  otherwise an investigator in the team (randomly among two or more); otherwise a random normal agent. Workers never
  coordinate. An investigator coordinating weaker models leads them: explains, shows the way, and writes or fixes
  code itself when needed.
- The daemon assigns items, one to each normal member at a time. Workers do not take items: they serve `delegate`
  requests, the first free worker in the configured order.
- The coordinator integrates a member's branch, gets the gate green and opens the PR. The next item of a feature
  starts when the previous one is merged (with `--no-merge`: approved by every reviewer).
- Every PR gets `[review] min_first_full_reviews` reviews (default two) from investigators outside the team. A team that leaves fewer configured
  investigators outside it is refused at start: "too many investigators in the team". Fewer than twice the required
  count outside the team warns that reviewer rotation may be unavailable, but the team starts. "Changes requested" goes back
  to the team at most 5 times; after that the human is asked.

## Plan-step judgements

`agent-core::plan::judge` selects done steps that need judgement and defines `confirmed`, `declared` and `mismatch`.
`agent-runtime::plan::check` checks file and directory existence asynchronously, resolving paths inside the worktree.
The owner runs these checks sequentially after reading a plan at every run's end, independently of role and outcome,
and awaits durable history acknowledgement before settling the run. Commands stay declared in this part.
Task and round status use cached plan summaries; their read path does no filesystem or database work.
History schema 17 adds append-only step judgements and migrates older databases.

## Rules of the plan

These hold for every item of the plan, on top of the rules each role is given — the developer rules to write Rust, the
review rules to review it — whose rule IDs (ERR-2, PR-1, ...) they cite. Both are in the prompt catalog (`PromptId`);
the developer rules also come in a variant for workers (a delegated task instead of a pull request).

- **Size:** PR-1 and PR-2 as written: a PR is planned for at most 1500 changed lines, tests not counted, and never
  passes 2000; the 500 between them are a reserve for what the estimate missed. An item estimated above 1500 is
  split before coding, into items of their own.
- **One item, one PR.** The PR description quotes the item in full under `## Task`; the reviewers check the PR
  against it and run its checks. A file outside the item's expected list, an expected part left out, or a check that
  cannot pass yet needs its reason in the PR description. Each round starts from its own file,
  `rounds/round-N.M.md` ([`rounds/round-1.1.md`](../rounds/round-1.1.md), [`rounds/round-2.1.md`](../rounds/round-2.1.md)).
- **Every item stands alone:** it compiles and passes the whole gate on `main` without the other items of its
  feature, and uses only code from the features its feature's "Depends on" line names, all of them merged. Items of
  one feature are taken in order; features whose dependencies are merged run in parallel.
- **The old implementation is gone.** `bin/agent` (Bash), `agent/` (the unfinished Go version) and `live/` (the old
  dashboard) were removed in 0.25.0; where the plan names them, git's history shows how they worked. Nothing is
  ported line by line or kept compatible: no shared files, no shared state format, no shared commands. `agentbus/`
  (Go) stays and is used as it is.
- **No prompt text in `.rs` files, ever.** Everything a model reads, and everything written into a file a human
  answers (questions-file instructions, notification texts), lives in `prompts/` as a template with `{{name}}`
  variables. Error messages of the CLI and log events stay in code.
- **Async with `tokio`.** No external process except the model CLIs (`claude`, `codex`, `agy`, `opencode`), `git`,
  `agentbus`, the repository's own gate command, the configured notification command and the configured editor.
  Everything the Bash version did with `sed`, `awk`, `jq`, `grep`, `flock`, `gh` or `tailscale` is done natively.
- **Approved dependencies:** `tokio`, `tokio-util`, `clap`, `serde`, `serde_json`, `toml`, `thiserror`, `tracing`,
  `tracing-subscriber`, `tempfile`, `rustix`, `reqwest` (rustls), `axum`, `notify`, `rand`, `time`, `strum` (`derive`), `rusqlite` (`bundled`; the history, from item 8.4). Any other needs
  a DEP-1 justification. Each is declared once, in `[workspace.dependencies]`, with its minimal features.
- **A separate `AGENTS_HOME`.** New code is developed and tested against its own `AGENTS_HOME`; nothing touches the
  live `~/agents` of the running Bash harness until feature 10.
- English everywhere: code, comments, docs, commit messages, PR descriptions.

Out of scope for the plan: remote nodes and the lead, investigation teams with phases and standalone investigation
requests, `agent live`, archiving, the cost of tokens in money, moving the state file into the database, one agent
on several accounts, automatic PR size limits (reviewers check PR-1), a proof by the harness that a RED commit
fails (a reviewer runs one, TDD-1), an `agentbus` socket API (it is called as a process).

## The gate

The workspace dev and test profiles keep line tables for backtraces with file and line; release builds are unchanged.
Agent CLIs and gates receive `CARGO_INCREMENTAL=0` by default (`[build] incremental = true` sets `1`), while manual builds keep Cargo's incremental policy.

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

`make gate` runs it alone: the lints and the tests of `agentbus` (Go) are steps of `make gate-full`, for a change of
`agentbus/` or the tools. It is also a step of `make test` and `make lint`. The existing CI workflow
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

### Turn budgets (item 8.8)

Claude Code `error_max_turns` is `StreamOutcome::TurnLimit`, recorded as `RunEnd::TurnLimit`;
`error_during_execution` remains a failure. Provider quota exhaustion wins when both are present.
The launch layer passes `[limits] max_turns` (default `300`) through the adapter contract; adapters without a CLI
turn limit ignore it and never emit this outcome. The owner saves the end and queued status together before the
next launch. It derives the consecutive count from the suffix of persisted run endings, so a restart between
settlement and scheduling neither resets the budget nor duplicates the continuation. Without a resumable session
the task fails instead of silently starting over. The resumed run records `why = turn_limit` in the history.

### Split-plan preparation

`agent-core::state::split` validates whole plans and assigns lettered `PartId`s separately from task-list `ItemId`s.
Teams retain immutable approved plans and reject unknown or repeated parent items. Schema 13 migrates schema 12
without reconstructing or renaming history; older migrations remain supported. The fixture and whole-document
comparison are described in [state.md](state.md#approved-split-definitions).

`questions::split::read_split` reads only an explicit human verdict, retaining typed validation causes. A bare `ok`
or free-text split answer returns no plan. `decide::split::next_parts` takes a plan and the set of merged keys and
returns eligible identities in declaration order, or completion when all parts are merged. It performs no I/O;
callers must not use it to reopen work already assigned in an active round.

The daemon activation described below adds durable per-part execution to these definitions. Existing items without
a split retain their lifecycle and names, including delivery through another pull request.

### Split proposal policy and persistence

`state::split::proposal::read_proposal` reads bounded coordinator JSON separately from legacy human verdicts.
Validated proposal fields include explicit scope and per-part numeric estimates with units; optional tiers default
to normal. Approved plans retain optional requirements without changing existing `PartDefinition` constructors.

`decide::split::approval::decide_proposal` takes parsed input (or an unreadable fault), `[split]`, a repository unit
and completed correction count. It returns a whole plan, a typed send-back fault or a reason to ask the human.
Plan validity is checked before scope escalation. Dependency traversal names an actual member of a cycle rather
than a downstream dependant, and accepts acyclic forward references. No I/O or code-specific checks occur here.

`state::split::keeping::ProposalKeeping` retains raw input, correction count and a pending/correction/asked stage;
`Team::keep_split_proposal` checks parent membership. Optional persisted fields preserve old state loading.
The fixture and representation are documented in [state.md](state.md#split-proposal-preparation).
The owner recognizes a coordinator's standalone `SPLIT: <item>` marker and reads its bounded task-local
`split-proposal-<item>.json`. It saves pending input before calling the pure policy with `[split] unit`.
Approval activates the whole plan atomically; correction retains the fault and queues the coordinator;
scope changes or exhausted corrections write one harness question using the existing human answer path.
Member markers produce a bus notice to the coordinator, without activating a split or asking the human.
Pending decisions are resumed before team assignment; saved corrections and asked stages are not replayed.
Catalog entries supply each fault, the next prompt and the question. The append-only history records the outcome
as `split_decision`, with parent, correction count and reason. At debug level the structured log uses the existing
`decision_taken` event with decision `split_proposal`. History migration 15 preserves
records and links while admitting the new event. Human verdicts continue through the existing atomic path.

### Execution of approved split plans

The state owner interprets coordinator answers atomically: a malformed plan records no parts and returns its
reason to the answer client. Valid plans bind the existing working round to part a. Optional round identities
record each part and its merged pull request without changing schema 13 or ordinary round names. Assignment
opens a later part only after its dependencies have merged, using a separate member branch. Publication snapshots
the gated integration tip onto a distinct branch for each part's pull request. Intermediate merges retain their
round history and return the parent to pending; the final merge uses the ordinary item and team closure flow.
The prompt catalog supplies each part's deliverable, checks and earlier merged pull requests. The status protocol
adds optional split plans, and answer responses carry an optional split acknowledgement alongside the ordinary
`applied` reply; the CLI derives the four part states from plans and durable rounds. Recovery reads this
same state, so neither the gap between parts nor an open part requires a separate restart mechanism.

### Tier-aware assignment

`agent-core::kinds::Tier` has a total capability order: investigator > normal > worker, independent of the
stable keyword declaration order. Task-list `Tier:` directives default to normal and are exposed through
`TaskList::required_tier`. The pure `decide::assign::tier::choose_agent` takes offered members in preference order
and an optional coordinator; it returns a fitting member, a fitting coordinator only if no member fits, or an
explicit missing-tier result. `decide::assign::dispatch` combines this choice with dependency readiness,
availability and reservations within one snapshot. A fitting but busy member prevents coordinator fallback.
The daemon uses this decision for initial assignment and subsequent parts. Activating a split may leave an
underqualified parent assignee; before assigning work, the owner returns that item to pending and chooses again.
Missing capability is retained in the team's optional `tier_waits` map, refreshed atomically with assignment.
Entering a wait logs `team_changed` with `kind=tier_wait`; restored waits do not log again. The status reply
exposes this optional map, and assignment prompts explain the minimum tier and selection. Coordinator takeover
after a failed run checks the working items’ requirements too. Split plans optionally retain their approval authority;
legacy plans leave it unknown until recorded explicitly.
