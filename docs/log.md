# The daemon's own log

`agent daemon` runs unattended under `systemd --user`. When something goes wrong inside the daemon itself — a model CLI
prints a line that is not JSON, a `git` or GitHub call fails, `agentbus` is unreachable, a gate times out, or a task
panics — the daemon records a structured log entry in two places at once:

1. **Standard error (`stderr`)** — one human-readable line per event, captured by the `systemd` journal
   (`journalctl --user -u agent-daemon`).
2. **`history.db` (`daemon_logs` and `daemon_log_rounds`)** — a structured row in the same SQLite database that holds
   the history of teams, rounds, and runs ([`history.md`](history.md)), carrying the `team`, open `rounds`, `task`, and
   `run` from the enclosing `tracing` spans so a failure can be queried alongside the work that was running when it
   happened.

## Configuration (`[log]`)

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

## Spans and scope

Every unit of work in the daemon runs inside a `tracing` span that carries its identifiers:

- `team` (`TeamId`) and `round` (`RoundNumber`, or comma-separated open rounds when a team has more than one open round)
  around team, round, check, publication, review, and merge operations;
- `task` (`TaskId`) around task lifecycle, gate, and worktree operations;
- `run` (`RunNumber`) around each model CLI invocation.

The tracing layer walks the active span stack from root to leaf (and any explicit `team`, `round`, `rounds`, `task`, or
`run` fields on the event itself) and attaches them to the `LogRecord`:

- `team` and `rounds` are stored in `daemon_logs.team` and `daemon_log_rounds(log, round)` (strictly ascending, never
  present without a `team`);
- `task` and `run` are stored in `daemon_logs.task` and `daemon_logs.run` (`run` is never present without a `task`).

An event emitted outside any team or task (such as a refused client request or a startup check) has `team = NULL`, no
`daemon_log_rounds` rows, `task = NULL`, and `run = NULL`.

## Coalescing repeated warnings

A broken external dependency (`agentbus` down, a failing `git` remote, a rate-limited GitHub endpoint) can fail on every
tick while a team waits. Instead of writing thousands of identical rows:

- When a `warn` event has the same `(event, team, rounds, task, run, message, fields)` as an existing `warn` row in
  `daemon_logs`, the writer increments `count` by `1` and updates `at` to the new timestamp, leaving `first_at` at the
  timestamp of the first occurrence.
- `error`, `info`, and `debug` rows never coalesce (`count` is always `1` and `first_at == at`).

## Decisions and outside calls (`debug`)

At `level = "debug"`:

- `decision_taken` is emitted whenever a pure decision function in `agent_core::decide` takes action (forming a team,
  assigning an item, starting or advancing a round, acting on a run outcome, scheduling a retry or pause, evaluating
  round checks, or choosing a merge action). Its `fields` carry `decision`, `input`, and `result`. A decision check on
  an idle team or task that produces no action logs nothing.
- `outside_call_completed` is emitted whenever an external call (`git`, `agentbus`, the GitHub API, or a model CLI
  process) finishes, with `call`, `operation`, `elapsed_ms`, and `outcome` in `fields`.

## Panic hook

When the daemon starts, it installs a panic hook before spawning its tasks. If any thread or task panics, the hook emits
a `daemon_panicked` (`error`) event with the redacted panic message and `location` (`file:line:column`) so both `stderr`
and `history.db` record the fault before unwinding.

## Redaction, bounds, and storage failures

- **No secrets.** Before any line is written to `stderr` or queued for `history.db`, every configured secret from
  `secrets.toml` (`github_token`, `[agent_env]`, and `[notification_env]`) is replaced with `<redacted>` in the event's
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
| `run_ended_without_status` | `warn` | A model CLI run ends without reporting a status or fails. |
| `run_hung` | `warn` | The hang watchdog stops a silent run. |
| `run_quota_exhausted` | `warn` | A run stops because its model quota is exhausted. |
| `run_rate_limited` | `warn` | A run stops because of a rate limit. |
| `bus_down` | `warn` | An `agentbus` operation fails and what waits for it is retried or dropped. |
| `git_call_failed` | `warn` | A `git` call fails or a branch operation is retried. |
| `github_call_failed` | `warn` | A GitHub API call fails or is retried. |
| `gate_cannot_start` | `warn` | A gate command cannot start or be recorded. |
| `gate_timed_out` | `warn` | A gate command exceeds its timeout and is killed. |
| `gate_dirty_tree` | `warn` | A gate runs on or leaves a dirty worktree or moves `HEAD`. |
| `state_save_failed` | `error` | The daemon's state file cannot be saved. |
| `questions_file_invalid` | `warn` | A questions file is missing or not valid. |
| `configuration_name_missing` | `error` | An agent, investigator, or repository named in the state or a request is no longer in `agents.toml`. |
| `restart_corrected` | `warn` | A restart of the daemon found an interrupted run and corrected it. |
| `request_refused` | `warn` | A client request is refused as invalid. |
| `daemon_panicked` | `error` | A task or thread in the daemon panicked, with its message and location. |
| `team_changed` | `info` | A team is started or closed. |
| `round_changed` | `info` | A round is opened or closed. |
| `item_phase_changed` | `info` | An item's phase changes. |
| `task_status_changed` | `info` | A task's status changes. |
| `run_started` | `info` | A model CLI run starts. |
| `run_ended` | `info` | A model CLI run ends. |
| `decision_taken` | `debug` | A pure decision of `agent_core::decide` takes action, with `decision`, `input`, and `result`. |
| `outside_call_completed` | `debug` | An outside call (`git`, `agentbus`, GitHub API, model CLI) completes, with `call`, `operation`, `elapsed_ms`, and `outcome`. |
