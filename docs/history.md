# The history of the Rust `agent`

The state file says what is; the history says what happened. It is one SQLite database, `history.db` in `AGENTS_HOME`,
written by the daemon and read by anyone: who was chosen and how, every prompt, every step and change of every run. The
state file (`state.json`), the questions files, the worktrees and the files of a gate's output stay where they are
([`state.md`](state.md), [`architecture.md`](architecture.md)).

This page is the reference of the schema — the checkpoint after item 8.4 of the plan: read it, then ask the database of
one round with `sqlite3` (the queries are below). The schema is expensive to change once an installation has a history.
The pinned text of the schema is [`history.schema.sql`](history.schema.sql); a test compares it with a fresh database.

The records are the types of [`agent_core::history`](../crates/agent-core/src/history.rs), the schema and its migrations are in
[`schema.rs`](../crates/agent-runtime/src/history/schema.rs) (the one module with `CREATE TABLE`), the writer, the database and
the reader in [`agent_runtime::history`](../crates/agent-runtime/src/history.rs); the daemon and the run engine record.

## What is recorded

Each record has its time. Everything is keyed by its team and, where it has one, its round.

| Record | When | Tables |
|---|---|---|
| a **team** starting | `agent run` is carried out: the task list as given, the repository and its base branch, the merge mode, and the **plan** — every participant with its role and how it got it | `teams`, `plan` |
| a **round** opening | an item is given to a member for the first time (and for each item given when the team starts), or a list without items starts: the item in full — title, description, expected result, checks — or the brief | `rounds` |
| a **run** starting | the owner makes the run ready: its task, agent (name, tier, CLI, model), role and rounds; why it starts; the whole prompt; the worktree, the branch; the session | `runs`, `run_rounds` |
| the **launch** of its process | the engine starts the CLI: the command line the CLI saw, the directory, the commit the worktree was at, the *names* — never the values — of the environment variables the daemon set | `launches` |
| a **step** | every event of the CLI's output, as the activity log gets it: a message, a command, a file edit, a bus call, another tool call, an error, information about the run, and a control of the human used while the run goes — numbered in the run | `steps` |
| a **session** observed | a CLI announces a session that differs from the one already known; the original launch session remains in `runs.session` | `run_sessions` |
| the **end** of a run | how it ended, the agent's final message whole | `run_ends` |
| what a run **changed** | the process is gone: from git, the commits between the two heads (hash, subject, author time), the files that differ (added, deleted, modified, renamed from where; lines added and removed; binary or not), and what it left uncommitted in the worktree | `run_changes`, `commits`, `changed_files` |

**Not here** (later items of the plan): gates, checks, pull requests, reviews, questions (8.5); the daemon's own log (8.6);
`agent log` (8.5, 8.7); what is deleted and when (8.7); tokens (11.1).

### Why a run starts

`runs.why` is one of these; the columns beside it say more:

| `why` | The run starts because | More |
|---|---|---|
| `first_run` | it is the first run of the task | |
| `item` | the task is given an item (a member, or a solo agent's next item) | `why_item` |
| `wake_up` | a message or a board item woke the task | |
| `request` | a worker serves a request | `why_asked_by`: the run that asked (its task's latest run when the request was served) |
| `fix_pass` | the reviewers asked for changes and the lead fixes | `why_pass`: the number of the fix pass of the item, from 1 |
| `review` | a reviewer reviews a pull request — also again after a fix pass | `why_pr`; `why_pass`: how many fix passes the item had when the review was asked, `0` for the first |
| `ask_human` | the fix passes are used up and the lead puts the question to the human | |
| `verdict_missing` | a reviewer's report had no verdict and is asked for again | |
| `checks` | the checks before a review sent the round back or put a question to the human | |
| `answers` | the human answered the task's questions | |
| `resumed_by_human` | the human stopped the task and let it go on (`agent resume`, `unpause`) | |
| `continuation` | the task goes on after a restart of the daemon, a hang or a limit | |

### How a participant got its role

`plan.how`: `named` (on the command line, nothing else decided the role: a solo agent, a member), `named_by_flag`
(`--coordinator`), `named_by_line` (the `Coordinator:` line of the task list), `drawn` (the coordinator, drawn at random
among `drawn_among`: the investigators of the team, or when it has none every agent that takes items), `configured_order`
(a worker: it serves requests in the order of the configuration). The flag wins over the line.

## The file

`history.db` is a SQLite 3 database (3.54, built into the binary: `rusqlite` with `bundled`). It is **append only**: the daemon
inserts and never updates or deletes a row (item 8.7 decides what is deleted, and when). Its pragmas:

| Pragma | Value | Why |
|---|---|---|
| `journal_mode` | `WAL` (kept in the file) | a reader never waits for the writer and sees a whole snapshot |
| `synchronous` | `NORMAL` | in WAL mode a crash may lose the last commits, never corrupt the file |
| `foreign_keys` | `ON` (each connection of the writer) | the rows of one record hold together |
| `busy_timeout` | 5 seconds | a person with `sqlite3` open does not make the writer lose a batch at once |
| `user_version` | the version of the schema, `2` | forward-only migrations (below) |

Every table is `STRICT`: a column holds the type it says. The `CHECK`s of the schema repeat the rules of the typed records
(a step of the kind `message` has a text and no path), so a row that something else wrote is refused by SQLite or, if
the checks were switched off, by the reading API.

Foreign keys join the rows of **one record** (a run and its steps, a team and its plan). `team`, `task` and `round` in
`runs` and `rounds` are plain columns: a history that starts after its team did — the file was deleted, the daemon was
upgraded in the middle of a team — records what happens from then on, and a query joins by the ID.

### Times, IDs and texts

- **Times** are UTC text with nine digits of a second: `2026-11-14T08:13:10.000000000Z`. They sort as times and read back
  as the same `Timestamp`. Two rows of the same time are in the order they were written: `rowid` for `runs` and `rounds`,
  the number (`n`, `position`) for the rest.
- **IDs** (`team`, `round`, `task`, `run`) are the text the state has: `team-20261114-081310-k3x9`. A commit is its 40 or 64
  hexadecimal digits.
- **Texts** that a model or a person wrote are cut at `[history] max_text_bytes` (default 1 MiB) at a character boundary,
  and the `cut` column of their row names the fields that were cut — comma separated, sorted, `''` when none: `'prompt'`.
  A text of exactly the limit is kept whole. The fields are: `teams.task_list`; `rounds.title`, `description`,
  `expected_result`, `checks`, `brief`; `runs.prompt`; `launches.args` (each argument); `steps.text`, `path`, `tool`,
  `detail`; `run_ends.final_message`; `commits.subject`.
- **Lists** (`plan.drawn_among`, `launches.args`, `launches.env_names`) are JSON arrays of texts.

## Tables

### `teams`

One row for each team started: its task and how it was asked for.

| Column | Type | Meaning |
|---|---|---|
| `team` | TEXT, key | the team's ID |
| `at` | TEXT | when it started |
| `repo` | TEXT | the repository, as `[repos.NAME]` of `agents.toml` |
| `base_branch` | TEXT | the branch the team's branches start from |
| `merge_mode` | TEXT | `autopilot` or `no_merge` (`--no-merge`) |
| `task_list` | TEXT | the task list as it was given |
| `cut` | TEXT | the fields that were cut |

### `plan`

Every participant of a team: who, in which role, how it got it. Key `(team, position)`, `WITHOUT ROWID`.

| Column | Type | Meaning |
|---|---|---|
| `team` | TEXT | the team (foreign key to `teams`) |
| `position` | INTEGER ≥ 1 | the place on the command line: `agent run a b c` has `a` at 1; the agents named on the command line are this table in this order |
| `agent` | TEXT | the agent's name |
| `role` | TEXT | `solo`, `coordinator`, `member` or `worker` |
| `how` | TEXT | how it got the role: `named`, `named_by_flag`, `named_by_line`, `drawn`, `configured_order` |
| `drawn_among` | TEXT, JSON array | for `drawn`: the candidates, in the order named; `NULL` otherwise |

### `rounds`

One row for each round opened: one team's work on one item, from its start to its merge.

| Column | Type | Meaning |
|---|---|---|
| `round` | TEXT, key | the round's ID |
| `team` | TEXT | its team |
| `at` | TEXT | when it opened |
| `item` | TEXT | the item's number (`1.1`); `NULL` for the one round of a list without items |
| `title`, `description` | TEXT | the item's title and what to do; `NULL` without an item |
| `expected_result`, `checks` | TEXT | the reviewers' standard and what anyone can run; `NULL` when the item has none |
| `brief` | TEXT | the brief of a list without items; `NULL` for an item |
| `cut` | TEXT | the fields that were cut |

### `runs`

One row for each run: what the owner decided when it made the run ready. Its process is in `launches`, its end in
`run_ends`, its changes in `run_changes`.

| Column | Type | Meaning |
|---|---|---|
| `run` | TEXT, key | the run's ID |
| `team`, `task` | TEXT | the team and the task of the run |
| `at` | TEXT | when it started |
| `agent` | TEXT | the agent's name |
| `tier` | TEXT | `normal`, `worker` or `investigator` |
| `cli` | TEXT | `claude`, `codex`, `gemini` or `opencode` |
| `model` | TEXT | the model, `NULL` for the CLI's default |
| `role` | TEXT | the role of the task: `solo`, `coordinator`, `member`, `worker` or `reviewer` |
| `why` | TEXT | why it starts; see "Why a run starts" |
| `why_item`, `why_asked_by`, `why_pass`, `why_pr` | TEXT, TEXT, INTEGER, INTEGER | the details of `why`; `NULL` where the reason has none |
| `prompt` | TEXT | the whole prompt |
| `worktree` | TEXT | the worktree the CLI runs in |
| `branch` | TEXT | the branch the worktree is on |
| `session` | TEXT | the session the run begins with, when it is known before the CLI starts (a resumed one; a Claude Code session the harness names) |
| `cut` | TEXT | the fields that were cut |

### `run_sessions`

Sessions learned from CLI output are appended immediately, including notifications that have no activity event.
Repeated notifications of the same session are recorded once; a changed session gets the next number.

| Column | Type | Meaning |
|---|---|---|
| `run` | TEXT | the run; foreign key to `runs.run` |
| `n` | INTEGER | positive observation number within the run |
| `at` | TEXT | time of observation from the injected clock |
| `session` | TEXT | validated CLI session identifier |

The primary key is `(run, n)`; the table is `STRICT, WITHOUT ROWID`. Query a run's observations with
`SELECT n, at, session FROM run_sessions WHERE run = :run ORDER BY n;`. The initial session remains in `runs.session`; an echo of the session supplied at launch is not a new observation.

### `run_rounds`

The rounds a run works for, oldest first. Key `(run, position)`; `UNIQUE (run, round)`: a run names a round once.

| Column | Type | Meaning |
|---|---|---|
| `run` | TEXT | the run (foreign key to `runs`) |
| `position` | INTEGER ≥ 1 | its place in the list |
| `round` | TEXT | the round |

### `launches`

How the engine started the process of a run. One row for each run whose process started.

| Column | Type | Meaning |
|---|---|---|
| `run` | TEXT, key | the run (foreign key to `runs`) |
| `at` | TEXT | when the process started |
| `program` | TEXT | the program, as it was run |
| `args` | TEXT, JSON array | its arguments, in order: what the CLI saw. The prompt is one of them, unless it travelled in a file |
| `working_dir` | TEXT | the directory it ran in: the worktree |
| `head` | TEXT | the commit the worktree was at when the run started; `NULL` when git could not say |
| `env_names` | TEXT, JSON array | the names, sorted, of the environment variables the daemon set — `AGENTS_HOME`, `PATH`, the variables of `[agent_env]` of `secrets.toml`. **Never their values** |
| `prompt_in_file` | INTEGER | `1` when the prompt was too long for a command line and travelled in a file (`runs.prompt` has it whole; the last argument points to the file) |
| `cut` | TEXT | the fields that were cut |

### `steps`

Every step of a run, in order. Key `(run, n)`, `WITHOUT ROWID`. A step has the columns of its kind and no others.

| Column | Type | Meaning |
|---|---|---|
| `run` | TEXT | the run (foreign key to `runs`) |
| `n` | INTEGER ≥ 1 | the step's number in the run: 1, 2, 3, … A missing number is a record that was lost |
| `at` | TEXT | when it was seen |
| `kind` | TEXT | see below |
| `text` | TEXT | `message`, `command`, `error`, `info`: the text, the command line, the error, the information |
| `path` | TEXT | `file_edit`: the file, as the CLI names it |
| `tool`, `detail` | TEXT | `bus_call`, `tool_call`: the tool and, in `detail`, the arguments of a bus call or what another tool is asked to do |
| `action`, `client`, `login` | TEXT | `control`: what the human did (`stop`, `pause`, `unpause`, `resume`, `say`), through which client (`cli` or `dashboard`) and, for the dashboard, the Tailscale login |
| `cut` | TEXT | the fields that were cut |

`kind`: `message`, `command`, `file_edit`, `bus_call`, `tool_call`, `error`, `info` (the model and the end of the run) and
`control`. A control used while a run goes (a stop, a pause, a `say`) is a step of that run; one used on a task that has no
run going (a resume, an unpause) is in the activity log of the task, not in a run.

### `run_ends`

How a run ended: what the owner recorded. A run the daemon did not see end — it was killed with `SIGKILL` — gets its row
when the next daemon finds it going and ends it with no status.

| Column | Type | Meaning |
|---|---|---|
| `run` | TEXT, key | the run (foreign key to `runs`) |
| `at` | TEXT | when it ended; never before `runs.at` |
| `how` | TEXT | `reported` (the agent's last line was `STATUS: …`), `no_status`, `quota_exhausted`, `rate_limited`, `hung` (the watchdog), `stopped` (the human) |
| `outcome` | TEXT | for `reported`: `DONE`, `QUESTIONS`, `FAILED`, `SLEEPING` or `EXCLUDED` |
| `exit_code` | INTEGER | for `no_status`: the CLI's exit code; `NULL` when a signal ended it (or the daemon was killed) |
| `resets_at` | TEXT | for `quota_exhausted`: when the CLI says the quota is back, if it says |
| `final_message` | TEXT | the agent's final message, whole; `NULL` when the run ended without one |
| `cut` | TEXT | the fields that were cut |

### `run_changes`, `commits` and `changed_files`

What a run changed, read from git when its process was gone — also for a run that was stopped. One row of `run_changes` for
each run whose worktree git could read.

| `run_changes` column | Type | Meaning |
|---|---|---|
| `run` | TEXT, key | the run (foreign key to `runs`) |
| `head_before` | TEXT | the commit the worktree was at when the run started (`launches.head`); `NULL` when unknown |
| `head_after` | TEXT | the commit it was at when the process ended |

Between two heads: the commits `head_before..head_after` and the files that differ. Without `head_before` nothing is known
between the heads, and both are empty.

`commits`: key `(run, n)`, `WITHOUT ROWID`.

| Column | Type | Meaning |
|---|---|---|
| `run` | TEXT | the run (foreign key to `run_changes`) |
| `n` | INTEGER ≥ 1 | the order, oldest first; a merge after the commits it brought in |
| `hash` | TEXT | the commit |
| `subject` | TEXT | the first line of its message |
| `authored_at` | TEXT | when it was authored |
| `cut` | TEXT | the fields that were cut |

`changed_files`: key `(run, scope, path)`, `WITHOUT ROWID`.

| Column | Type | Meaning |
|---|---|---|
| `run` | TEXT | the run (foreign key to `run_changes`) |
| `scope` | TEXT | `committed`: differs between the two heads; `uncommitted`: the worktree has it and no commit does — a changed, new or deleted file, binary files included, files the repository ignores not |
| `path` | TEXT | the file, in the repository; for a deleted file, where it was |
| `change` | TEXT | `added`, `deleted`, `modified` or `renamed` |
| `renamed_from` | TEXT | for `renamed`: where it was |
| `is_binary` | INTEGER | `1` for a binary file: git counts no lines |
| `lines_added`, `lines_removed` | INTEGER | the lines it added and took away (a replaced line counts once in each); `NULL` for a binary file |

The files are read through an index of a scratch directory, so reading them changes nothing in the repository: the files of
`uncommitted` are changed or untracked files, with the lines counted by `git diff -M`. The snapshot disables configured
clean/process filters and filesystem monitors: it observes worktree bytes without running those external commands. A
repository whose clean filter transforms content can therefore show differences that its normal filtered `git diff` hides.

## Indexes

| Index | On | For |
|---|---|---|
| `rounds_by_team` | `rounds (team, at)` | the rounds of a team, oldest first |
| `runs_by_team` | `runs (team, at)` | the runs of a team |
| `runs_by_task` | `runs (task, at)` | the runs of a task |
| `run_rounds_by_round` | `run_rounds (round, run)` | the runs of a round, and the prompts of a round |

The key of each table is an index too: `WITHOUT ROWID` tables are stored in the order of their key.

## What happens when the history cannot be written

History storage never waits in the work path. The daemon is its one writer, on a thread of its own behind a queue of 4,096
records; it writes what is queued, up to 256 records in one transaction, and when a batch is refused it tries its records
one by one, so a record that cannot be written loses only itself.

When a record cannot be kept — the queue is full, the file or the disk cannot be written, the database cannot be opened —
the work goes on, the failure goes to the standard error of the daemon (the journal of the unit) **once for each cause**
(`queue_full`, `unavailable`, `capture_head`, `capture_changes`, `write_disk_full`, `write_read_only`, `write_io`, `write_corrupt`, `write_refused`, `secret_identity`, …), the
records lost are counted, and `agent status` shows the count. A failed starting-head lookup counts one incomplete launch; a failed change snapshot counts one missing changes record. When history is unavailable, the engine counts the skipped changes record without running Git for it. Disabled history neither captures nor counts records.

The human authorized a bounded capture exception for item `8.4`: before launching a CLI and after it exits, the engine
awaits a best-effort Git snapshot so a subsequent run cannot overwrite uncommitted changes before attribution. Each
boundary has one absolute `2`-second budget shared by all Git commands, including scratch-index commands and executable
startup retries. On timeout the process group is killed; reaping the leader has a separate `1`-second bound. A timeout
counts an incomplete launch or missing changes record and the run continues. Successful captures retain exact attribution.
These are cooperative runtime deadlines, not hard real-time guarantees for kernel scheduling or filesystem operations.
SQLite writes remain asynchronous and cannot hold up the run.

For example:

```text
history: not written: cannot open the history database "/home/you/agents/history.db": …; 12 records lost
```

| Situation | What the daemon does |
|---|---|
| `[history] enabled = false` | records nothing, loses nothing, `agent status` does not mention it |
| `history.db` is a directory, the home is read-only, the file is not a database | says so at its start, runs without a history; every record handed in is counted as lost |
| the database is of a **newer version** than the build writes | says so at its start, does not write to it (the file is left byte for byte as it was), runs without a history |
| the queue is full | drops the records that do not fit and counts them; goes on when there is room |
| a write fails (disk full, I/O error) | counts the records of the batch; the next batch is tried |
| a record the database refuses (a step of a run that has no row) | counts it; the others are written |
| the daemon is killed (`SIGKILL`) | what was queued is lost; the file opens, passes `PRAGMA integrity_check`, holds whole records only (a record is one transaction); the next daemon records the end of the runs it finds going |

A history that starts after its team (a deleted `history.db`) loses the records that hang on a row it does not have — the steps
of a run that started before — and counts them.

## Versions and migrations

`PRAGMA user_version` is the number of the last migration applied. The schema is the result of the migrations of
`schema.rs` applied in order to an empty database — a new database and an old one end up the same. A migration is never
edited once it has shipped; a change of the schema is a new migration, forward only, each in a transaction of its own
together with the update of `user_version`: one that fails leaves the database at the version before it.

A daemon that opens a database of a version newer than its own does not write to it (above); `Reader::open` refuses it
too. A database of version `0` holds no history yet. Migration `2` adds `run_sessions` without changing existing rows.
The read-only API accepts version `1` histories and returns an empty session-observation list for them; opening one
for writing applies migration `2`. A preserved version `1` fixture verifies that the upgrade retains existing runs.

## Asking with `sqlite3`

A reader opens the file read-only and needs no daemon: `sqlite3 -readonly`. The IDs below are the ones `agent status` and
`agent run` print; `.parameter set` binds them.

```sh
sqlite3 -readonly -header -column "$AGENTS_HOME/history.db"
sqlite> .parameter set :round 'round-20261114-081310-k3x9'
```

The runs of a round, with agent and role and why each started:

```sql
-- runs of a round
SELECT r.at, r.run, r.agent, r.role, r.why
FROM run_rounds AS rr
JOIN runs AS r ON r.run = rr.run
WHERE rr.round = :round
ORDER BY r.at, r.rowid;
```

The whole prompt of one run (`.parameter set :run 'run-…'`), and whether it was cut:

```sql
-- the prompt of a run
SELECT prompt, cut FROM runs WHERE run = :run;
```

The prompts of a round, with who got them:

```sql
-- the prompts of a round
SELECT r.run, r.agent, r.role, r.prompt
FROM run_rounds AS rr
JOIN runs AS r ON r.run = rr.run
WHERE rr.round = :round
ORDER BY r.at, r.rowid;
```

The command line the CLI saw and what the daemon set in its environment:

```sql
-- the launch of a run
SELECT program, args, working_dir, head, env_names FROM launches WHERE run = :run;
```

The steps of a run, in order:

```sql
-- the steps of a run
SELECT n, at, kind, coalesce(text, path, tool || ' ' || detail, action) AS what
FROM steps
WHERE run = :run
ORDER BY n;
```

The files a run changed — committed and not — and its commits:

```sql
-- the files a run changed
SELECT scope, change, path, renamed_from, is_binary, lines_added, lines_removed
FROM changed_files
WHERE run = :run
ORDER BY scope, path;
```

```sql
-- the commits of a run
SELECT n, hash, subject, authored_at FROM commits WHERE run = :run ORDER BY n;
```

How each participant of a team got its role (`.parameter set :team 'team-…'`):

```sql
-- the plan of a team
SELECT position, agent, role, how, drawn_among FROM plan WHERE team = :team ORDER BY position;
```

The item of a round in full:

```sql
-- the item of a round
SELECT item, title, description, expected_result, checks, brief FROM rounds WHERE round = :round;
```

How the runs of a team ended:

```sql
-- how the runs of a team ended
SELECT r.run, r.agent, e.how, e.outcome, e.exit_code
FROM runs AS r
LEFT JOIN run_ends AS e ON e.run = r.run
WHERE r.team = :team
ORDER BY r.at, r.rowid;
```

A run with no row in `run_ends` is going — or the daemon was killed before it recorded the end.

## The reading API

`Reader::sessions(run)` returns validated session observations in sequence order.

`agent_runtime::history::Reader` opens the file read-only (`Reader::open(path)`) and gives typed answers, each read in one
snapshot — never a record the writer is in the middle of:

`team`, `rounds_of_team`, `round`, `runs_of_team`, `runs_of_round`, `run` (each with its start, launch and end), `steps`,
`changes`, `prompts_of_round` and `schema` (what `docs/history.schema.sql` pins).

Every answer is a `Stored<T>`: the record and the set of its fields that were cut. Every row is read through the checks of
its types and the rules of its record — an ID that is not an ID, a step of a kind that lacks its text, a run that ends before
it starts, a `cut` that names no field — and a row that breaks one is `ReadError::Value` with the table and the column. A
file whose version is newer than the build reads is `ReadError::Newer`; one no daemon has written to, `ReadError::Empty`.

## What is never in it

The launch records environment variable names, never their values. Before a record reaches SQLite, the writer
replaces occurrences of known secret values with `[REDACTED]` in free text, including prompts, command arguments,
activity payloads, final messages and commit subjects. Known values are the GitHub token and every value in
`[agent_env]` and `[notification_env]` of `secrets.toml`. Overlapping matches are removed together, before text truncation.
The original prompt and arguments delivered to the CLI are unchanged. This is an explicit exception to byte-for-byte
history recording, authorized for item `8.4`.

A credential inside a structured identity (for example a session, branch or Git path) cannot be replaced without
misrepresenting it. The writer omits that record, increments the lost count and reports `secret_identity` once without
the value. A replacement marker that itself contains a configured secret also causes omission. Writer error details
are redacted before reporting. Tests inspect both database and live WAL bytes after a merged round whose CLI echoes
a configured credential.

Unknown credentials, encoded or transformed values, and the separate raw CLI/activity files are outside this exact-value
history redaction. `History::start_with_secrets` provides the protection; direct `Store` callers must supply safe records.
