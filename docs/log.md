# `agent log`: the story of a round and the daemon's own log

`agent log` tells what happened from [the history](history.md). It opens `history.db` read-only and asks no daemon, so it
works when the daemon is stopped, crashed or never started. This page describes the story of a round or team and the
daemon's own structured log; questions, search and following (8.7) come with their item.

```sh
agent log --round round-20261114-164520-ab12        # one round, from its item to its end
agent log --team  team-20261114-164000-cd34         # the stories of the team's rounds, in order
agent log --round <id> --prompts                    # every prompt whole, not its first lines
agent log --round <id> --no-steps                   # without the steps of the runs
agent log --round <id> --json                       # the records as JSON lines
```

Exactly one of `--round` and `--team` is given. `--prompts` and `--no-steps` change the text only.

| Exit | Meaning |
|---|---|
| `0` | the story was printed |
| `1` | the history cannot be read (no file, a newer version, a row that breaks its rules), or the output cannot be written |
| `2` | command-line parsing fails, for example a missing or conflicting subject, an invalid ID or an unrecognised option |
| `3` | the history has no such round or team |

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

## `--json`

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
| `run_ended_without_status` | `warn` | A model CLI run cannot start, ends without reporting a status or verdict, or fails. |
| `run_hung` | `warn` | The hang watchdog stops a silent run. |
| `run_quota_exhausted` | `warn` | A run stops because its model quota is exhausted. |
| `run_rate_limited` | `warn` | A run stops because of a rate limit. |
| `bus_down` | `warn` | An `agentbus` operation fails and what waits for it is retried or dropped. |
| `git_call_failed` | `warn` | A `git` call, worktree operation, or branch operation fails or is retried. |
| `github_call_failed` | `warn` | A GitHub API call, pull-request publication, or merge fails, is retried, or has no token configured. |
| `gate_cannot_start` | `warn` | A gate command cannot start or be recorded. |
| `gate_timed_out` | `warn` | A gate command exceeds its timeout and is killed. |
| `gate_dirty_tree` | `warn` | A gate runs on or leaves a dirty worktree or moves `HEAD`. |
| `state_save_failed` | `error` | The daemon's state file, activity log, or notification cannot be saved or delivered. |
| `questions_file_invalid` | `warn` | A questions file is missing, invalid, or cannot be prepared, watched, or opened. |
| `configuration_name_missing` | `error` | An agent, investigator, or repository named in the state or a request is no longer in `agents.toml`. |
| `restart_corrected` | `warn` | A daemon start or restart found an interrupted run or host environment fallback and corrected it. |
| `request_refused` | `warn` | A client connection, subscription, or request cannot be accepted or is refused. |
| `daemon_panicked` | `error` | A task or thread in the daemon panicked or failed unexpectedly, with its message and location. |
| `team_changed` | `info` | A team is started or closed. |
| `round_changed` | `info` | A round is opened or closed. |
| `item_phase_changed` | `info` | An item's phase changes. |
| `task_status_changed` | `info` | A task's status changes. |
| `run_started` | `info` | A model CLI run starts. |
| `run_ended` | `info` | A model CLI run ends. |
| `decision_taken` | `debug` | A pure decision of `agent_core::decide` takes action, with `decision`, `input`, and `result`. |
| `outside_call_completed` | `debug` | An outside call (`git`, `agentbus`, GitHub API, model CLI) completes, with `tool`, `operation`, `duration_ms`, `outcome`, and, when available, `exit_code`. |
