# `agent log`: the story of a round and the daemon's own log

`agent log` tells what happened from [the history](history.md). It opens `history.db` read-only and asks no daemon, so it
works when the daemon is stopped, crashed or never started. Without arguments it prints the newest `100` daemon log
rows, in write order. A single round or team argument retains the story described below. Search, following,
focused views, statistics and retention use the same database.

```sh
agent log --round round-20261114-164520-ab12        # one round, from its item to its end
agent log --team  team-20261114-164000-cd34         # the stories of the team's rounds, in order
agent log --round <id> --prompts                    # every prompt whole, not its first lines
agent log --round <id> --no-steps                   # without the steps of the runs
agent log --round <id> --json                       # the records as JSON lines
agent log --json                                   # the newest daemon log rows as JSON lines
agent log --round <id> --show log                   # only this round's daemon log rows
agent log --team <id> --level warn -n 20            # this team's newest 20 warning rows
```

A lone `--round` or `--team`, with only `--prompts`, `--no-steps` or `--json`, prints the existing story.
`--prompts` and `--no-steps` change its text only. No scope, both scopes together, `--show log`, or any row filter
selects daemon log rows instead. Row filters are `--task`, `--run`, `--agent`, `--level`, `--event`, `--since`,
`--until`, `--grep`, `-n` and `-f`. Tests of the round and team
filters use explicit `--show log` so they compare the same kind of rows.

Filters intersect. `--level` matches exactly one severity; `--agent` matches the agent of the associated run.
`--since` and `--until` are inclusive UTC timestamps. `-n 0` prints no rows; the default is `100`.
The newest matching rows are printed oldest first by database insertion ID. Row-mode `--json` prints each log's
`id`, `first_at`, `at`, `level`, `event`, `module`, `team`, `rounds`, `task`, `run`, `message`, `fields`, `count`
and `cut`. A filter that matches nothing succeeds with empty output. Older schemas without daemon logs also
produce empty row output, without changing the database.

| Exit | Meaning |
|---|---|
| `0` | the story or matching log rows were printed |
| `1` | the history cannot be read (no file, a newer version, a row that breaks its rules), or the output cannot be written |
| `2` | command-line parsing fails, for example an invalid ID, an unrecognised option or `--show` without a scope |
| `3` | story mode or focused view (`--show`): the history has no such round or team |

## Search, following and focused views

```sh
agent log --grep 'Zażółć AND OR NEAR "quoted" % *' --json
agent log --round <id> --show prompts
agent log --team <id> --show agents
agent log --event run_started -n 0 -f
agent log --stats
```

`--grep TEXT` searches the FTS5 index of prompts, step text and tool details, final messages, review reports,
questions, answers and log messages. It returns matching log rows and rows associated with a matching run or task;
round-wide events match the round's log rows. Text is a literal token phrase, never an FTS expression: quotes are
escaped, `AND`, `OR` and `NEAR` are ordinary words, and `%` and `*` are not wildcards. Unicode tokenisation handles
Polish letters. Punctuation alone has no searchable token and returns no rows. The index contains the same bounded,
redacted text as the history. A read-only pre-search database returns no search matches until the daemon migrates it.

`-f` prints the initial newest matching rows and then new rows in insertion order, each ID once. `-n 0 -f` starts
with future rows only. The reader polls every `100` ms without contacting the daemon, so a daemon restart does not
interrupt following. The durable cursor survives retention even when every log row is deleted. Updates to an existing
coalesced warning are not new rows and are not printed again. Stop following with Ctrl-C.

`--show` takes exactly one view and requires a round or team. Row filters apply to `--show log`; another
view selects its full scoped projection. `--json` makes their structured output JSON lines.

| View | Contents |
|---|---|
| `tasks` | each recorded task's latest role, agent and run status |
| `agents` | participants and reviewers, roles, and how they were chosen |
| `worktrees` | each task's latest path, branch and recorded head |
| `runs` | run starts, including identity, reason and scope |
| `prompts` | the whole stored prompt of each run, with run, task and agent |
| `changes` | recorded commits, changed paths and uncommitted changes |
| `reviews` | review requests and submitted reports |
| `log` | scoped daemon log rows |
| `usage` | an explicit notice that tokens are not recorded yet (`11.1`) |

`--stats` prints a JSON object with `bytes` (logical database pages times page size), `tables` (the count of every
ordinary, virtual and shadow table), and `oldest` (the oldest record timestamp, or `null` for empty history).
It uses one read snapshot and works without a daemon. WAL and shared-memory sidecars are not included in `bytes`.

## Retention

Set `[history] keep_days` for closed rounds and `[log] keep_days` for log rows belonging to no round. Both default
to `0`, which disables that policy. At startup and every `24` hours, the daemon removes rounds closed strictly
before the configured cutoff, their owned records and gate-output files, and teams whose rounds are all gone.
An open round is never removed. A shared run remains while any of its rounds remains. Log retention does not trim
rows of a surviving round. Readers see complete database snapshots while deletion commits; `auto_vacuum = FULL`
reclaims freed pages. A failed deletion is reported without stopping work; the next scheduled pass retries it.
See [history retention](history.md#retention-and-task-activity) for the file-cleanup contract.

## The text

A story is made by `agent_core::story::render`, a pure function from the records of the round (or team) to lines; nothing
else is read. Its parts, in the order of time (records with the same time keep the order they were written in):

1. the round and its team, with its item in full;
2. the agents by role, and how each was chosen;
3. every run: who, in which role and why it started, its task, worktree and branch, how long it took, its prompt (the
   first lines and the size, or whole with `--prompts`), its command line, its steps numbered with kind and time, and how it
   ended with its final message;
4. after each run, its changes: the commits, the files added, deleted, modified and renamed with their lines, and what was
   left uncommitted;
5. the gates and the checks before a review; the pull request from opening to merge or hand-over (the body whole); every
   review pass with the reviewers' runs, each verdict and report whole; every fix pass with the findings it sent the team; the
   questions asked and the answers; the notifications; the controls; the end of the round.

A team's story is the stories of its rounds, in the order of their records, followed by a `Team events` section when the team
has records that belong to no single round (such as a coordinator run started while more than one round is open, a team-wide
control or notification, or `team_ended`). Because `team_ended` is linked to every round of its team, in a team of one or
several rounds every round's story (`agent log --round <id>`) ends with the team's end, which happens after the last round, while
`agent log --team <id>` prints `team ended` once at the end of the team's story. `agent log --round <id>` prints only the records
linked to `<id>` (and the start of its team), so team-wide records with no round appear in `agent log --team <id>` and not in
`agent log --round <id>`.

A record the database refused has a best-effort loss note: where it would stand, the story says that it was lost, what
it was and why (the cause the writer gave). The note needs a subsequent successful write; permanent storage failure
cannot leave a durable explanation. A permanently invalid note is warned about and dropped so it cannot block later
notes; the loss it described was already counted. Queue and capture losses are counted by `agent status`; a run without its end or
changes also has a missing-record notice in its story. A text cut at the history limit says so after it.

## Story-mode `--json`

One JSON object a line, each record of the round exactly once, ordered by its time and then by its write sequence:

```json
{"sequence":41,"at":"2026-11-14T16:45:20Z","team":"team-…","rounds":["round-…"],"kind":"review_submitted","cut":[],"record":{"…":"…"}}
```

`kind` is the kind of the record: `team`, `round`, `run`, `launch`, `step`, `run_end`, `changes`, `session` or one of
the kinds of an [event](history.md#events). `record` holds every field whole; `--prompts` and `--no-steps` do not apply.
`agent log --round <id> --json | wc -l` is the number of the round's records in the database
(`SELECT count(*) FROM history_record_rounds WHERE round = '<id>'`, plus the one record of its team, the start of the team).

The text of a story is pinned by golden files under `crates/agent/tests/golden/`, each made by a daemon run with fake CLIs and
a fake GitHub server, IDs and times normalised.

## The daemon's own log

`agent daemon` runs unattended under `systemd --user`. When something goes wrong inside the daemon itself — a model CLI
prints a line that is not JSON, a `git` or GitHub call fails, `agentbus` is unreachable, a gate times out, or a task
panics — the daemon records a structured log entry in two places at once:

1. **Standard error (`stderr`)** — one human-readable line per event, captured by the `systemd` journal
   (`journalctl --user -u agent-daemon`).
2. **`history.db` (`daemon_logs` and `daemon_log_rounds`)** — a structured row in the same SQLite database that holds
   the history of teams, rounds, and runs ([`history.md`](history.md)), carrying the `team`, open `rounds`, `task`, and
   `run` from the enclosing `tracing` spans so a failure can be queried alongside the work that was running when it
   happened.

### Configuration (`[log]`)

In `agents.toml` ([`configuration.md`](configuration.md)):

```toml
[log]
level = "info"
```

`level` sets the lowest severity kept by both `stderr` and `history.db`:

| Level | Keeps | Default | What it records |
|---|---|---|---|
| `error` | `error` | no | unrecoverable daemon errors: state save failures, missing configured names, panics |
| `warn` | `error`, `warn` | no | every degraded condition, retry, invalid client request, or abnormal run stop |
| `info` | `error`, `warn`, `info` | **yes** | every lifecycle change of a team, round, item, task, or run — and nothing on an idle tick |
| `debug` | `error`, `warn`, `info`, `debug` | no | every decision of `agent_core::decide` that takes action (`input` and `result`) and every outside call (`git`, `agentbus`, GitHub API, model CLI) with its duration and outcome |

When `[history] enabled = false`, the daemon still writes its log to `stderr` at the configured `[log] level`, and
opens no `history.db`.

### Spans and scope

Every unit of work in the daemon runs inside a `tracing` span that carries its identifiers:

- `team` (`TeamId`) and `round` (`RoundId`, or comma-separated open rounds when a team has more than one open round)
  around team, round, check, publication, review, and merge operations;
- `task` (`TaskId`) around task lifecycle, gate, and worktree operations;
- `run` (`RunId`) around each model CLI invocation.

The tracing layer walks the active span stack from root to leaf (and any explicit `team`, `round`, `rounds`, `task`, or
`run` fields on the event itself) and attaches them to the `LogRecord`:

- `team` and `rounds` are stored in `daemon_logs.team` and `daemon_log_rounds(log, round)` (strictly ascending, never
  present without a `team`);
- `task` and `run` are stored in `daemon_logs.task` and `daemon_logs.run` (`run` is never present without a `task`).

An event emitted outside any team or task (such as a refused client request or a startup check) has `team = NULL`, no
`daemon_log_rounds` rows, `task = NULL`, and `run = NULL`.

### Coalescing repeated warnings

A broken external dependency (`agentbus` down, a failing `git` remote, a rate-limited GitHub endpoint) can fail on every
tick while a team waits. Instead of writing thousands of identical rows:

- When a `warn` event has the same `(event, team, rounds, task, run, message, fields)` as an existing `warn` row in
  `daemon_logs`, the writer increments `count` by `1`, sets `first_at` to the earliest timestamp and `at` to the latest,
  regardless of the order in which concurrent producers enqueue their events.
- `error`, `info`, and `debug` rows never coalesce (`count` is always `1` and `first_at == at`).

### Decisions and outside calls (`debug`)

At `level = "debug"`:

- `decision_taken` is emitted whenever a pure decision function in `agent_core::decide` takes action (forming a team,
  assigning an item, starting or advancing a round, acting on a run outcome, scheduling a retry or pause, evaluating
  round checks, or choosing a merge action). Its `fields` carry `decision`, `input`, and `result`. A decision check on
  an idle team or task that produces no action logs nothing.
- `outside_call_completed` is emitted whenever an external call (`git`, `agentbus`, the GitHub API, or a model CLI
  process) finishes, with `tool`, `operation`, `duration_ms`, `outcome`, and, when available, `exit_code` in `fields`.
  For a GitHub call, `exit_code` is the actual HTTP response status.

### What only `stderr` has

Every event of the daemon has one of the names above and goes to both places, with these exceptions, which go to `stderr`
only:

- a failure of the history itself (the writer cannot keep a record, a loss note cannot be written, the writer does not
  close cleanly): a failure to write the log is never logged into the history, so that it cannot loop;
- an event of a time the clock cannot give: it has no time to be stored with;
- `trace` events, which no `[log] level` keeps in the history;
- the note of the GitHub client that it left out what accounts that are not trusted wrote, which no call of the daemon
  makes yet.

Events from before the history is open (such as `restart_corrected` for an unknown time zone at the start of `agent daemon`)
and after it is closed are on `stderr` only.

### Panic hook

When the daemon starts, it installs a panic hook before spawning its tasks. If any thread or task panics, the hook emits
a `daemon_panicked` (`error`) event with the redacted panic message and `location` (`file:line:column`) so both `stderr`
and `history.db` record the fault before unwinding.

### Redaction, bounds, and storage failures

- **No secrets.** Before any line is written to `stderr` or queued for `history.db`, every configured secret from
  `secrets.toml` (`github_token`, `[agent_env]`, and `[notification_env]`) is replaced with `[REDACTED]` in the event's
  `message`, structured `fields`, and span fields.
- **Size bounds.** `message` and `fields` are bounded by `[history] max_text_bytes`. A longer `message` is cut at a
  UTF-8 character boundary and `"message"` is recorded in `cut`; a `fields` JSON object whose serialized form exceeds
  `max_text_bytes` is replaced with `{"truncated":true}` and `"fields"` is recorded in `cut`.
- **Non-blocking and non-looping.** If `history.db` cannot be opened or written, or if the writer queue is full, the
  event still goes to `stderr`, the writer prints a rate-limited line (`the history cannot keep a log entry`) directly
  to `stderr` without re-entering `tracing`, and the daemon continues its work.

## Events (`agent_core::log::LogEvent`)

Every daemon log entry uses one of the fixed `LogEvent` names below. A test compares this table against
`LogEvent::ALL` and `LogEvent::level`.

| Event | Level | Meaning |
|---|---|---|
| `cli_output_unreadable` | `warn` | A line of a model CLI's output cannot be read as its event format. |
| `run_ended_without_status` | `warn` | A model CLI run cannot start, ends without reporting a status or a reviewer's verdict, or fails. |
| `run_hung` | `warn` | The hang watchdog stops a silent run, or process cleanup cannot kill or reap a process so that its end is known. |
| `run_quota_exhausted` | `warn` | A run stops because its model quota is exhausted. |
| `run_rate_limited` | `warn` | A run stops because of a rate limit. |
| `bus_down` | `warn` | An `agentbus` operation fails and what waits for it is retried or dropped; also a member that did not read the message it was woken for `limits.wake_ups_per_message` times is not woken for it again (`task`, `message_id`, `from`, `wake_ups`). |
| `git_call_failed` | `warn` | A `git` call, worktree operation or branch operation fails or is retried. |
| `github_call_failed` | `warn` | A GitHub API call, a publication or a merge fails or is retried, or the home has no GitHub token. |
| `gate_cannot_start` | `warn` | A gate command cannot start or be recorded. |
| `gate_timed_out` | `warn` | A gate command exceeds its timeout and is killed. |
| `gate_dirty_tree` | `warn` | A gate runs on or leaves a dirty worktree or moves `HEAD`. |
| `state_save_failed` | `error` | The daemon's state file cannot be saved. |
| `questions_file_invalid` | `warn` | A questions file is missing or not valid, or cannot be prepared, watched or opened. |
| `configuration_name_missing` | `error` | An agent, investigator, or repository named in the state or a request is no longer in `agents.toml`. |
| `restart_corrected` | `warn` | A start or restart of the daemon found an interrupted run or a host fallback and corrected it. |
| `request_refused` | `warn` | A client connection, subscription or request cannot be accepted or is refused. |
| `daemon_panicked` | `error` | A task or thread in the daemon panicked, with its message and location. |
| `daemon_task_failed` | `error` | A task of the daemon (a side task or the task of a client connection) ends with an error it does not handle, such as a panic, with `error`. |
| `team_not_started` | `error` | A team that was formed is not started because of a fault of the daemon (the state refuses it, its events cannot be made, or forming it failed inside); the client is told to see this log. |
| `notification_failed` | `error` | A notification for the human cannot be made, timed, queued or delivered. |
| `history_record_incomplete` | `warn` | A record of the history cannot be made as it should: it is left out, counted as lost, or made with a fallback (such as a run's reason recorded as a continuation). |
| `team_changed` | `info` | A team is started or closed. |
| `round_changed` | `info` | A round is opened or closed. Checked delivery changes use `change=delivery_accepted` or `delivery_merged` at info, and `delivery_refused`, `delivery_closed` or `delivery_poll_failed` at warn. Records name `team`, the known `round` and `pr`, and an `error` cause chain for failed checks or observations. |
| `item_phase_changed` | `info` | An item's phase changes. |
| `task_status_changed` | `info` | A task's status changes. |
| `run_started` | `info` | A model CLI run starts. A run that the bus's wake-up starts also has `why` (`wake_up`), `message_id`, `from` and `wake_ups` (the number of this wake-up for that message). |
| `run_ended` | `info` | A model CLI run ends. |
| `daemon_started` | `info` | The daemon has opened its state and its history and serves its socket, with `home`. |
| `daemon_stop_requested` | `info` | A signal asks the daemon to stop, with `signal` (`SIGTERM` or `SIGINT`). |
| `daemon_stopped` | `info` | Everything of the daemon but its history has ended, with `outcome`: the last record of a daemon that ended; its absence after a `daemon_started` means the daemon was killed. |
| `request_delegated` | `info` | A request for a worker is given to a worker, with `request` and `worker`. |
| `questions_answered` | `info` | The open questions of a task are answered, with `number` and `channel`. |
| `control_used` | `info` | The human used a control (stop, pause, unpause, resume), with `action` and the number of `tasks` it reached. |
| `checks_decided` | `info` | The checks before a review send a round back to its team, ask the human, or take the human's answer, with `outcome`. |
| `reviews_requested` | `info` | The reviews of a pull request are asked for, with `pr` and the number of `reviewers`. |
| `review_report_late` | `info` | A reviewer's report comes when no review waits for it: it is kept or left out, and changes nothing. |
| `notification_sent` | `info` | A notification for the human is delivered, with the `file` it was written to. |
| `questions_file_opened` | `info` | A new questions file is opened in the editor, or is not because the editor is not there, with `outcome`. |
| `build_directory_removed` | `info` | A build directory of a worktree was removed to release disk space, with `task` and `bytes`. |
| `decision_taken` | `debug` | A pure decision of `agent_core::decide` takes action, with `decision`, `input`, and `result`. |
| `outside_call_completed` | `debug` | An outside call (`git`, `agentbus`, GitHub API, model CLI) completes, with `tool`, `operation`, `duration_ms`, `outcome`, and, when available, `exit_code`. |
| `peer_gone` | `debug` | The other end of an answer (a client of the socket, or a task of the daemon) was gone before the answer. |
| `github_step_waiting` | `warn` | A step of a round that calls GitHub or git fails and waits: once when it starts to wait, with `team`, `operation`, `pr`, `outlook` (`may_pass` or `will_not_pass`), `next_try` (empty for a refusal) and the `detail` line `agent status` shows. |
| `github_step_resumed` | `info` | A step of a round that waited for GitHub goes through, once, with `team` and `operation`. |
