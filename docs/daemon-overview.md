# The daemon — how the harness watches and drives the agents

This is an overview of `agent daemon`: what it owns, what it checks, when it acts and why. It is written for
someone who wants to understand the harness before changing it or before diagnosing a stuck team. It describes
`main` at agent 0.141.0 · agent-core 0.138.0 · agent-runtime 0.126.0 (PR #336, 27.11.2026).

The details live in [`docs/architecture.md`](architecture.md), the keys in
[`docs/configuration.md`](configuration.md), the log events in [`docs/log.md`](log.md) and the state in
[`docs/state.md`](state.md). File references below are to the code, relative to the repository root. They were
right at the time of writing. Section 15 lists the places where the docs and the code disagree.

---

## 1. What the daemon is

team-agents does not implement an agent loop. It starts the vendors' CLIs (Claude Code, Codex, Antigravity/Gemini,
OpenCode) headless, gives each run a prompt and an MCP server (`agentbus`), and acts on how each run ends.
**The daemon is the only process that decides anything.** The agent CLIs do the work. `agentbus` is the shared
mailbox and board. Every other `agent` command is a client.

The daemon has three jobs:

1. **A scheduler.** It starts runs within limits: per agent, per machine, and only while there is disk space.
2. **A supervisor (the "watchdog").** It watches every run and every team, and steps in when one is stuck, silent,
   hung, over its context, out of quota, over time, or idle.
3. **A pipeline.** It carries each item of work from assignment through the gate, the prechecks, the PR, the
   review, the fix passes and the merge, and asks the human only where the rules say only the human may decide.

Everything it does follows one principle: **models are not trusted to follow the protocol**. Anything that must
happen is checked from outside, by the harness. Examples are a gate on the exact commit, an existing test left
unchanged, an answer delivered, and a run that ends. The prompts tell agents what to do; the daemon verifies it.

## 2. Process model

### One daemon per `AGENTS_HOME`

- **Start.** `agent daemon` (unit `config/systemd/agent-daemon.service`). It takes a non-blocking exclusive
  `flock` on `$AGENTS_HOME/daemon.lock`. A second daemon on the same home fails with "use another AGENTS_HOME for a
  second one" (`crates/agent-runtime/src/store.rs:479`, `crates/agent/src/daemon/mod.rs:350`).
- **Start order** (`daemon/mod.rs:341-451`):
  1. the lock;
  2. the settings (`agents.toml`, `secrets.toml`);
  3. the prompt catalogs, every entry checked at start;
  4. the bus client;
  5. `state.json`;
  6. `history.db`;
  7. the run engine (at most **16** runs at once on the machine);
  8. the notifier and the socket server;
  9. recovery (section 10);
  10. the main loop.
- **Configuration is read once, at start.** Changing `agents.toml` takes effect only after a restart of the
  daemon.

### Tasks inside the process

| Task | What it does |
|---|---|
| **owner** | The single task that reads and changes the state (rule CON-3). Every decision happens here, so there are no races between decisions. |
| **server** | `agent.sock`: accepts JSON requests (one per line, at most 2 MiB, at most 64 connections) and passes them to the owner over a channel (inbox of 64). |
| **notifier** | Delivers notices (queue of 256). |
| **gate service** | Runs `agent gate` requests, **one gate at a time on the machine**, with a queue of 16. |
| **runs** | One tokio task per agent run, each with a child of the shutdown token. |
| **side jobs** | Plan checks, worker-request checks, disk jobs and gate answers, as join sets the owner polls. |

The socket is created in a private 0700 directory, set to 0600 and renamed into place, so that it is never
reachable with a wider mode (`daemon/server.rs:81`).

### The main loop (`daemon/owner.rs:862-927`)

Every turn does the following, in order:

1. history retention, once a day;
2. the idle-team scan, once per 60 s;
3. pending checkpoints are written;
4. **team life**:
   - recovery retries of worker requests;
   - lead briefings and busy hand-over notices;
   - the outbox flushed to the bus;
   - Serena reminders;
   - one **look** at every team queued for one;
   - pool requests;
5. **start queued runs**.

The loop then sleeps until something happens:

- a socket request;
- the end of a run;
- a gate answer;
- a save of a questions file (inotify);
- a disk job, a plan check or a request check;
- shutdown;
- the **tick, every 5 s**. The tick wakes paused tasks whose time has come and queues a look at every team.

**The bus is polled, not pushed.** Each look reads the team from `agentbus team-get`. If the bus cannot be read,
the look logs `bus_down` and tries again next time (`owner/teams.rs:683`).

### One look at a team (`owner/teams.rs:523-563`)

1. split proposals;
2. reconcile with the bus: messages, status, board;
3. re-queue underqualified work (tiers);
4. pull requests: gate, publish, review, merge;
5. assign items;
6. tier waits;
7. worker results;
8. serve investigation and worker requests;
9. close the team if it is finished;
10. round waits.

Then, for every task with a run going, the per-run watchers run:

- new commits;
- the commit reminder;
- the context reminder;
- the round-limit cut.

The state is saved, and events published, only when the look changed something.

### The one fatal rule

If `state.json` cannot be saved, the daemon logs `state_save_failed`, runs the **emergency notification
command** and stops (`owner.rs:3638`). The reason given in the docs: models must not keep working on a state that
is not kept. The unit's `Restart=on-failure` starts the daemon again, and recovery takes over.

## 3. What the daemon owns, and where things are

| Path in `AGENTS_HOME` | What it is |
|---|---|
| `state.json` | Teams, tasks, rounds, items, PR ledgers (schema version 15, [`docs/state.md`](state.md)). Written only by the daemon. |
| `history.db` | Every run, its end, its usage, decisions, reviews and checkpoints (SQLite, schema 35). The memory that survives everything. |
| `daemon.lock`, `agent.sock` | The single-instance lock, and the socket the CLI talks to. |
| `.free_space` | The last free-space measurement. |
| `bus/` | `agentbus`'s own state (messages, board, reservations, investigations, notebook), under its own `flock`. The daemon only calls it. |
| `worktrees/<repo>/<task>/` | One git worktree per task, on its item's branch. `.agent/plan.md` is the agent's plan. |
| `tasks/<task>/runs/<run>/` | `prompt.md`, `out.jsonl` (the CLI's stream), `err.log`, `outcome.json`, `mcp.json`. |
| `tasks/<task>/` | `questions-N.md`, `session-prompt.md`, `context.json` (context size of the latest model call), `gate.json`, `gate-previous.json`, `gate-<n>.log`, `checks.json`, `description.md` (the PR text the lead writes), `review-pr<N>-pass<K>.md`, `fix-pass-N-findings.md`, `worker-*.json`, `split-proposal-<item>.json`. |
| `notifications/` | One file per notice sent to the human. |
| `removed/<team>.bundle` | Unmerged branches of a removed team, kept as a git bundle. |
| `debug/` | Bundles written by `agent debug`. |

Checkpoints are history records plus git refs `refs/agents/snapshots/<round>/<task>/c<n>` in the repository.

## 4. Teams, tasks, rounds and runs

- **Team.** What `agent run` creates (`daemon/team/form.rs:164`).
  - It is either **solo** (one agent) or a team of a **coordinator** (the lead), **members** and **workers**.
    Workers come from the `[tiers] workers` pool and serve requests; they are not named on the command line.
  - Only a team with a coordinator gets a bus team.
  - A task list whose items are all done is refused.
- **Task.** One agent's part in a team. Each task has its own worktree and branch, a status, and a series of
  runs. The roles are Solo, Coordinator, Member, Worker, and the ones the daemon creates itself: **Reviewer** and
  **Investigator**.
- **Item.** One `[ ] N.M` line of the task list ([`docs/task-list.md`](task-list.md)). `Allow:` lines in the
  header pre-allow paths for the prechecks.
  - They are never allowed for `AGENTS.md`, `CLAUDE.md` or `.github/workflows/`.
  - Over-broad patterns are refused: `*`, `**`, `*.md`, `tests/**`.
- **Round.** One item, one lead branch, **one PR**, from assignment to merge (or to approval with `--no-merge`).
  - A round is `open` or `closed`, with an end of merged, approved or abandoned. A closed round never reopens.
  - Item phases: Pending → InProgress → InReview → Fixing ⇄ InReview → Approved → Done.
- **Run.** One start of an agent CLI. A task's runs share a **session**, resumed with the vendor's resume flag,
  until the daemon decides to start a new session (section 6).

## 5. How a run starts

### Who may start

`start_queued` (`owner.rs:1013`) starts a queued task only when all of these hold:

- the daemon is not shutting down;
- the team is not being removed or replaced;
- for a worker, worker recovery has finished;
- the agent is not refused by its provider (a refused agent gets no fresh assignments);
- the agent has fewer than `[limits] runs_per_agent` (1) runs going; for workers, `runs_per_worker` (1);
- the machine has fewer than 16 runs going;
- free disk space is at least `[disk] min_free_gb` (20).

**Why one run per agent:** each agent is a subscription or account with a quota, and two runs of one account in
two teams race for it. A task waiting for a busy agent shows "queued, waits for X, running in team-…". After
`[replacement] busy_after` (300 s) a free agent of the same or a higher tier takes it over.

### The prompt (`crates/agent-runtime/src/run/compose/`, `owner/start.rs`)

The kind of prompt depends on why the run starts:

| Kind | When |
|---|---|
| First | A new session: a new task, a renewal, or the session bound reached. |
| Reported | Resuming after a `STATUS:` end: `# YOUR SESSION GOES ON`, pointing at `session-prompt.md`. The item is re-sent only if it changed. |
| Restart | The daemon cut the run. |
| NoStatus | The run ended without a `STATUS:` line. |
| Hang | The watchdog killed a silent run. |
| Limit | After a quota or rate pause. |
| TurnLimit | After `--max-turns`. |
| CommitOrder | The commit reminder was ignored. |

**A first run** gets:

- the working context: agent, task, repo, worktree, questions file;
- the Serena section;
- the role prompt and the rules for the role and tier;
- the brief;
- the item, with its numbered `Expected result` (R1…) and `Checks` (K1…) sections;
- the team's items, for a coordinator.

**Sections added when they apply:**

- answers to questions;
- autopilot rules;
- plan corrections;
- the split correction;
- a note after a run that called no tool;
- **the briefing**: snapshot, commits, uncommitted files, the plan, the last 10 runs and answers, unread messages;
- the context-size paragraph after a renewal;
- the last message.

`session-prompt.md` is written for every new session, so that a resumed run can re-read its instructions without
the daemon re-sending them.

### The MCP servers of a run (`owner/start.rs:787-863`)

- **`agentbus serve`**, with the run's identity: agent, task, repo, worktree, branch, team. Further flags depend
  on the role:
  - `--workers` when the team has workers;
  - `--role worker|investigator`;
  - `--investigations on` for coordinators and members;
  - `--plan <worktree>/.agent/plan.md` for the roles that keep a plan.
- **No bus for the independent first review**, so that reviewers cannot see each other's findings before they
  report.
- **Serena** (`start-mcp-server --project <worktree>`), for code navigation.

### The CLI (`crates/agent-runtime/src/cli/`)

| Vendor | Started as |
|---|---|
| Claude | `-p --output-format stream-json --verbose --max-turns N --mcp-config … --strict-mcp-config --disallowedTools … --disable-slash-commands`; `--session-id`, or `--resume` for a resumed session |
| Codex | `exec --json -C <dir> -c mcp_servers…`; `resume <id>` |
| Antigravity | `--output-format stream-json --print-timeout 8h`; `--conversation <id>`; MCP configuration in `.agents/mcp_config.json` (excluded from git) |
| OpenCode | `run --standalone --format json --auto`; `--session <id>`; configuration through `OPENCODE_CONFIG_CONTENT` |

All four run without the CLIs' own permission prompts, because nobody is there to answer them. The sandbox is the
worktree, the checks and the review. Each run is its own process group, so that it can be stopped as a whole.

## 6. How a run ends, and what the daemon does next

### Reading the end (`crates/agent-core/src/decide/outcome.rs`)

The daemon reads `out.jsonl`, `err.log` and the exit code, in this order:

1. A human stop → **Stopped**.
2. A `STATUS:` line in the final message (the first line beginning with `STATUS:` once markup is stripped) →
   **Reported**. The words are `DONE`, `QUESTIONS`, `FAILED`, `SLEEPING`, `EXCLUDED`.
3. A failed run whose error tail shows a provider limit → **QuotaExhausted**, or **RateLimited** for 429/503/529.
4. The watchdog killed it → **Hung**.
5. Anything else → **NoStatus**.

### What each end leads to (`daemon/settle.rs:41-62`)

| End | Task becomes | Why |
|---|---|---|
| `DONE` | Done for solo and reviewer tasks; Waiting for team roles | A member's DONE means "my part is delivered"; the round goes on. |
| `QUESTIONS` | Questions | The human is asked (section 8). |
| `FAILED` | Failed, with a notice holding the last 2000 characters | The agent says it cannot go on. |
| `SLEEPING` | Waiting | A member with nothing to do; a bus message wakes it. |
| `EXCLUDED` | Done | The agent left the team on purpose. |
| Quota / rate limit | Paused | Until the provider's reset time plus 1 minute (capped at 8 days). Without a reset time: `quota_pause_minutes` (30) or `rate_limit_pause_minutes` (5). |
| Stopped | Stopped | The human's decision. |

An **investigator** is Done only after `DONE`; any other end fails its request.

### The retry ladder (`owner.rs:2884`, decision `ladder`)

A run that ends badly is not failed at once. The daemon climbs a ladder:

1. **The same session again.**
   - No status: resumed up to 2 times.
   - Turn limit: resumed up to `[limits] turn_limit_resumes` (3).
   - Hang: until `[watchdog] hang_strikes` (3) hangs in a row; a run that made progress resets the count.
2. **A new session** (`[replacement] new_sessions`, 1), **briefed** from the checkpoint, so that a confused session
   does not carry its confusion along.
3. **Takeover** (`owner/takeover.rs`): the task goes to one of `[replacement] agents`, otherwise to the lead
   (`[replacement] mode` = auto | lead | off). It is logged as `task_handed_over`, with a reason of no_status,
   hang, turn_limit, busy or round_overdue.
4. **Last step** (`owner/last_step.rs`): where a failure would drop work that was handed over, the human is asked
   once whether to start again in a new session or stop.
5. **Failed**, with a notice.

**Why a ladder:** most bad ends are transient (a crash of the CLI, a hung tool, a long turn). Retrying the same
session is cheap and keeps the context. A new session fixes a session that has gone wrong. Another agent fixes an
agent that cannot do the task. Only when all of that is spent is the human bothered.

### Provider refusals (`owner.rs:2411-2593`)

- **Counting.** Quota and rate refusals in a row are counted, and the agent is marked "refused" until its reset
  time. While it is refused, it gets no new assignments.
- **ChangeAgent.** After `[replacement] rejections` (5), or when the reset is far off, the task goes to a free
  agent. The candidates are replacement agents, then normal agents, then investigators. The new agent must be of
  the same or a higher tier, and outside the team, not busy and not refused.
- **Nobody free.** The task stays Paused until the earlier of the agent's return and the end of the pause. The
  `NoFreeAgent` notice is sent once.

## 7. The watchdog: every periodic check

These are the conditions the daemon watches without being asked. Each is a reaction to a failure the harness has
seen in practice.

| Check | Condition (key, default) | Action | Why |
|---|---|---|---|
| **Hang** (run engine) | No new output in `out.jsonl` for `[watchdog] hang_minutes` (30). Polled every second. A run waiting on its gate holds the watchdog off. | The process group gets SIGTERM, then SIGKILL after 10 s. The run is resumed from its session, then climbs the ladder after `hang_strikes` (3). | A CLI blocked on a tool or a network call never ends by itself. |
| **No status** | The run ended without `STATUS:`. | Resumed, at most 2 times, with a prompt asking for the status line; then the ladder. | The daemon cannot act on an end it cannot read. |
| **Turn limit** | The CLI hit `--max-turns` (`[limits] max_turns`, 300). | Resumed, at most `turn_limit_resumes` (3) times; then the ladder. Without a session the task fails (`TurnLimitNoSession`). | It bounds a run that loops, without losing a long but healthy one. |
| **Session bound** (`owner/bound.rs`) | The last run's context is over `max_context` (`[session]` or per agent, 150 000; 0 = off). An unreadable size also counts. | The next run starts a **new session**, briefed from the checkpoint (decision `context`). | Every model call re-sends the whole context. Past a size it costs too much, and the model loses track. |
| **Context reminder** | `context.json` reaches `[commit_reminder] context_percent` (80) % of the bound. Once per session. | A bus message tells the agent to commit and to write down its state (`context_reminded`). | So that the renewal after the bound loses nothing. |
| **Commit reminder** (`owner/commit_reminder.rs`) | In an open round: at least `[commit_reminder] lines` (300) uncommitted lines **and** at least `minutes` (30) since the later of the last commit and the run start. Looked at once a minute. | One bus reminder (`commit_reminded`). A commit re-arms it (`commit_followed`). If it is ignored for `cut_after` (20) minutes while no gate or test holds the worktree, the run is cut and resumed with an order to commit (`commit_cut`). | Uncommitted work is lost on a crash, cannot be checkpointed, and cannot be taken over. The 300 lines come from measurements taken on 24.11.2026. |
| **Round limit** (`owner/round_limit.rs`) | A round is open, with its PR not yet in review, for longer than `[limits] round_minutes` (default 0 = off; the example configuration sets 120). | The run is cut, and the task is handed over: another agent, then the lead, then failed with a notice (decision `round_overdue`). | An agent that circles for hours without a PR is better replaced than waited for. |
| **Asleep team** (`rounds/asleep.rs`) | Every task in the team is asleep or waiting while the work is unfinished, for `[watchdog] asleep_minutes` (5). | The lead is woken with `pr/team-asleep.md`, at most 2 times; then a `TeamAsleep` notice goes to the human. | A team in which each member waits for another never wakes on its own. |
| **Wake-ups per message** (`owner/wakes.rs`) | A member was already woken `[limits] wake_ups_per_message` (2) times for the same unread message. | It is not woken again for that message. | A member that ignores a message must not be started over and over at the cost of its quota. |
| **Idle team** (`owner/idle.rs`) | Nothing has run in the team for `[limits] idle_team_minutes` (120; 0 = off). It does not count while anything is running or queued, while the human paused or stopped the team, during provider waits, or while an external merge is awaited. Scanned every 60 s. | The team is removed by the same path as `agent remove`. A decision record is written to history first, and at most one attempt is made per minute. | A finished or abandoned team holds its agents (one run per agent) and its disk. |
| **Busy agent** | A queued task has waited `[replacement] busy_after` (300 s) for an agent that is running elsewhere. | A free agent takes it over. | The quota of one account should not block a whole team. |
| **Worker pool wait** (`owner/pool_wait.rs`) | A worker request has been queued longer than `[workers] wait_seconds` (600). | The request is returned to the requester: "do the step itself" (`worker_wait_returned`). | The requester must not wait forever for a pool that is busy. |
| **Investigation deadline** | An investigator has not answered within `[investigations] answer_minutes` (30; allowed 10–1440). The clock starts when the investigator's task is created. | The request is failed or re-queued once (section 9). | The requester waits on the answer. |
| **Free disk** (`owner/disk.rs`) | Measured before every launch and every `agent run`. | Below `[disk] low_free_gb` (100): one notice listing the 5 largest worktrees, and a reviewer's `target/` is removed when its run ends. Below `min_free_gb` (20): no run starts, `agent run` is refused, and open rounds wait with one notice. If the space cannot be measured, nothing starts. | Rust builds fill disks, and a full disk corrupts runs and the state. |
| **Gate time** | A gate runs longer than `[repos.NAME] gate_timeout_minutes` (30). | The gate is killed and recorded `timed_out`. | A hung test must not hold the machine's only gate slot. |

Some of this state is **kept in memory only**, by design: the reminders, round-limit cuts, gate reuse and GitHub
retry waits. A restart re-arms them, and the next look decides again from the files and the history.

## 8. Questions: the human in the loop

- **Asking.** A run that ends with `STATUS: QUESTIONS` must have written
  `tasks/<task>/questions-N.md`. If the file is missing or invalid, the daemon logs `questions_file_invalid` and
  the task fails with the `QuestionsMissing` notice. A valid file gets the answering instructions added, and a
  notice is sent.
- **Watching.** Each task directory has an inotify watch. Saving the file with `**ANSWERED**` on its last line
  counts as an answer, as do `agent answer <task>` and the dashboard. Open questions files are re-read at start, so
  answers saved while the daemon was down are not lost.
- **Delivering.** The task goes back to Queued (`question_answered`). The answers are put into every following
  run's prompt until a run ends reported or exits 0. An incomplete answer is sent back (`notice-incomplete.md`).
- **Questions the harness writes itself.** The daemon writes some questions with fixed wording:
  - **checks questions** before a review (section 11.2), answered `allow`, `refuse` or `allow except <paths>`;
  - **last-step questions** (section 6);
  - publish failures (`notice-publish-question.md`);
  - spent fix passes and spent merge-conflict passes, which the lead writes on the daemon's order.
- **Why the human answers at all:** some decisions are reserved to the human by the rules of the repository and
  never to an agent. Examples are an existing test changed, a protected file touched, and a frozen RED test
  edited. The prechecks enforce this, because a model told not to do something sometimes does it anyway.

## 9. Workers and investigators

### Workers (`owner/worker_result.rs`, `owner/owned_request.rs`)

- **Requests.** A member or lead delegates a step (`delegate` on the bus) to the worker pool. The daemon starts a
  worker on it.
- **Results.** On `DONE` the result is written to a file and then settled **in stages**, each one recorded so that
  it happens once even across restarts:
  1. a check in the worker's worktree;
  2. the decision;
  3. the commit brought over to the requester's branch;
  4. the requester told.
- **Interruptions.** An interrupted request is re-queued. After three returns with no result, the fourth returns
  its causes to the requester (`worker_request_returned`).
- **Records.** Every request's end is written to history (`worker_request_ended`), with the requester, the
  server, how it ended and the cost.

### Investigators (`owner/investigations.rs`)

Investigators are strong agents outside the teams, used for reviews and for questions a team cannot settle.

- **Requests.** A member or lead requests an investigation through the bus (`request_investigation`). At most 3
  requests may be pending.
- **Choice** (`decide::investigator::choose`):
  - not the requester's own investigator, and not one that already failed this request;
  - ordered by last activity in the team, then anywhere, then by name;
  - ties at the oldest are shuffled, so that the work rotates.
- **Failure.** A request fails when:
  - the item has no open PR;
  - the requester's task ended;
  - the human stopped it;
  - there is no answer within `answer_minutes`.
- **One more try.** A request is re-queued once to another investigator, with the unfinished answer attached
  (`investigation_requeued`). A second failure fails it.
- **Kinds.** The `conflict` kind (PR #332) asks for a resolution of a merge conflict at the tip of the item's
  PR. The automatic step, in which the daemon asks an investigator by itself before asking the human, is item
  5.1b and is **not on main yet**.

## 10. Restart and recovery

### On start (`Owner::recover`, `owner.rs:932-1010`)

The daemon rebuilds everything from the files, because a pause, a strike count or a reminder is not a field of
the state.

1. **Plan checks** left running by a dead daemon are ended from their records.
2. **Runs that were going** are ended as NoStatus at `max(now, started)`.
   - With `[restart] session = "new"` (the default), a task that served an open round gets **a new, briefed
     session**, and the cut does not count as a retry (reason `daemon_down`, decision `restart`).
   - Interrupted tasks are queued ahead of the ones that were already waiting.
   - Tasks whose budget is spent climb the ladder.
3. **Pause times, provider refusals and hang strikes** are rebuilt from the history of runs.
4. **Worktrees** of interrupted tasks are snapshotted and checkpointed.
5. **PR items** whose findings disagree are aligned to the most advanced record (`restart_corrected`).
6. **Reviews and investigations.**
   - Reviewers whose reports were not taken are asked again.
   - Second opinions are resumed, retried or held with a notice.
   - Worker requests are reconciled with the bus. Worker dispatch stays fenced until that succeeds.
7. Open questions files are re-watched and re-read, and a look is queued at every team.

The next run after a restart is told that it was "cut by a stop of the daemon" and gets the briefing.

### On stop (SIGTERM or SIGINT; `owner.rs:904-926`)

1. Plan checks are stopped.
2. Every run gets SIGTERM, then SIGKILL after 10 s, and is settled as it ends.
3. Lead briefings and hand-over notices are flushed to the bus, so that a lead given a member's work at the last
   moment is told even when the daemon stops then.
4. Disk removals and gate answers are drained.
5. A running gate is cancelled and answered `conflict` ("the daemon is shutting down").
6. `daemon_stopped` is logged, the history closed and the socket removed.

A daemon killed with SIGKILL leaves its runs alive, because they are separate process groups. The unit's
`KillMode=mixed` ends them.

### Checkpoints (`owner/checkpoint.rs`, `owner/briefing.rs`)

- **When.** At every step that matters: a new holder, a new session, an interrupted run, a daemon start.
- **What.** A snapshot of the worktree as a git ref, a history record, and a flush.
- **Failure.** A failed checkpoint logs `checkpoint_failed` and never stops the round.
- **Use.** The briefing of a new session is built from the latest checkpoint. This is how a takeover or a renewal
  continues the work instead of starting it again.

## 11. The round pipeline: from code to a merged PR

```
 lead DONE ──► gate at the tip ──► prechecks ──► PR (draft → ready) ──► review ──► APPROVE ──► merge
                  │ fail                │ issues                           │ REQUEST_CHANGES       │ conflict
                  ▼                     ▼                                  ▼                       ▼
            harness reruns,       team pass / human               fix pass (≤ 5)          lead merges base
            then sends back       question                        then verification       (≤ 3), then human
```

### 11.1 Gate (`daemon/gate.rs`, `decide/publish.rs`)

- **What it runs.** `agent gate` sends the worktree's gate to the daemon's gate service. The command is
  `[repos.NAME] gate`; in this repository it is `make gate`: fmt, clippy `-D warnings`, the tests under the
  machine's test lock, and the docs.
- **The record.** It is written to `gate.json` and keeps the commit, the result and whether the tree was clean.
  A gate that changes the tree or moves HEAD is recorded **not clean** (`gate_dirty_tree`) and does not count.
- **No useless reruns.** On a clean tree at the same head, with the same command, a gate that passed, or failed
  twice in a row, is not run again. The reply says "gate: not run again" (`gate_reused`).
- **The test lock.** A gate refused by a busy test lock exits 75. It is recorded as failed, but it does not count
  toward the two failures, because it tells nothing about the commit.
- **Publish only on a passed, clean tip.**
  - While the lead works, the daemon publishes only when a record passes at the tip.
  - When the lead reports DONE:
    - no commit of its own → sent back;
    - no record → **the harness runs the gate itself**;
    - failed at the tip → the harness runs it **once more** (`gate_retried`);
    - still failing → sent back with `pr/gate-not-passed.md`.
  - After 3 send-backs the human is told once per tip.
- **Why it is strict:** the PR must hold exactly what was checked. A gate run on another commit, or on a dirty
  tree, proves nothing.

### 11.2 Prechecks before the review (`decide/checks.rs`, `daemon/checks.rs`)

The daemon diffs the round against its base and looks for what the rules forbid:

| Finding | Who may allow it |
|---|---|
| An existing test changed, removed, renamed or its mode changed | only the human |
| A protected file changed (`[repos.NAME.checks] protected_paths`: `.github/`, `AGENTS.md`, `CLAUDE.md`, the gate's scripts, `Cargo.toml`, toolchain files …) | only the human |
| A test frozen by its `RED:` commit edited outside a test-only `TEST-FIX:` commit (TDD-14) | only the human; a finding about the commit |
| Production code with no test line added and no `Test-First: not possible — <reason>` (TDD-1) | the team or the human |
| A skip, a lint suppression or a `cfg!(test)` marker in a file the PR description does not name (TDD-11, LINT-3, TDD-12) | the team or the human |

- **The order of decisions:**
  - no findings → the review starts;
  - human-only findings → the human is asked at once, and no team pass is spent;
  - team findings with passes left → back to the team (`[review] check_passes`, 2);
  - passes used up → the human is asked.
- **Allowances are kept** in `checks.json` per item, with the head they were allowed at. A file that changes again
  is asked about again.
- **The commit guard.** `agent guard-commit` (exit 86) refuses a frozen-test edit already at commit time
  (`commit_guard`, default on).
- **Why:** an agent that weakens a test to make it pass is the most dangerous failure of an AI-developed
  repository. Only a check from outside catches it every time.

The PR's size (PR-1: 1500 planned, 2000 hard) is **not** a precheck. It is computed and written into the PR
description, and the reviewers enforce it.

### 11.3 Publishing (`owner/rounds/publish.rs`)

1. **Push.** The branch is pushed only when the remote is behind; a diverged remote is refused.
2. **Draft.** An existing PR is reused; otherwise one is opened as a **draft**.
3. **Prechecks**, as above. While they hold the PR, it stays a draft.
4. **Ready.** The body is set from the lead's `description.md` plus the harness's own sections (gate, size,
   allowed beforehand), and the PR is marked **ready**.

A failed push or PR call is retried `[publish] tries` (3) times, 60 s apart. Then the lead is told. Then the human
is asked to retry or stop.

### 11.4 Review (`daemon/review.rs`, `owner/rounds/reviewers.rs`, `verdicts.rs`)

- **Who reviews.** At least `[review] min_first_full_reviews` (2) investigators from **outside the team**, who are
  free (not running or queued anywhere) and not refused by their provider. They rotate by activity.
- **The first pass is independent.** Reviewers run without the bus, and the reports are held until all are in, so
  that no reviewer anchors on another.
- **Reports.**
  - The first `VERDICT:` line counts: `APPROVE` or `REQUEST_CHANGES`. A report without one is sent back at most
    twice; then the human is told.
  - Findings use the form `N. [BLOCKER|MAJOR|MINOR] RULE place — problem → fix`. Open BLOCKER and MAJOR findings
    block approval.
  - Reports are capped at 20 000 characters and posted to the PR.
- **Fix passes.** The lead gets the findings (`fix-pass-N-findings.md`) and answers each one FIXED or EXPLAINED,
  with commits. There are up to `[review] fix_passes` (5) passes; after them, the lead asks the human for more.
- **Verification.** Later passes are checked against the diff since the last reviewed head.
  - A disputed finding goes to the verifiers, who answer ACCEPTED or REJECTED.
  - A finding accepted by all of them drops to MINOR.
  - A blocker may get a **second opinion** (`allow_second_opinion_on_blocker`).
- **Replacement.** A reviewer that is failed or stopped is replaced at once. One paused for
  `[review] replace_after_minutes` (15) is replaced then. After a provider refusal, the replacement comes from
  another provider.
- **No second round of review.** A first pass that everyone approves merges without verification.

### 11.5 Merge and conflicts (`owner/rounds/merges.rs`, `decide/merge_refusal.rs`)

- **The merge.** In autopilot the harness merges with a merge commit, at the head that was reviewed. With
  `--no-merge` the human is told that the PR is approved.
- **Refusals.**
  - A refusal that is a conflict sends the lead `pr/merge-conflict.md`: fetch, `git merge --no-ff origin/<base>`,
    no rebase, no force push. There are up to `[merge] conflict_passes` (3) such passes.
  - After a pass:
    - the files changed only within `Allow:` lines → merge again;
    - otherwise → a verification review first;
    - no result → the human.
  - When the passes are spent, the lead puts the question to the human.
  - Any other refusal is told to the human once. Rate limits and network errors are retried quietly.
- **After the merge.**
  - The items are Done and the round is closed (`round_closed` merged, `notice-merged.md`).
  - `target/` directories are removed.
  - When every item is done, the merged worktrees are removed. Branches are kept, and dirty or unpushed
    worktrees are kept with a warning.

### 11.6 Splits

A coordinator may propose splitting an item that is too big (`SPLIT: <item>`, `split-proposal-<item>.json`;
`[split] part_limit` 1500 lines, 2 corrections allowed).

- **Approval.** The plan is approved as a whole.
- **Parts.** Each part gets a letter ID and its own branch, and opens once its dependencies have merged.

## 12. Notifications (`daemon/notifier.rs`, `owner/notice.rs`)

- **Delivery.** Every notice is written to `notifications/` and then passed to `[notifications] command`, which
  gets the file path as its last argument (for example an ntfy hook) and has a 30 s timeout. Secrets reach the
  command only through its environment.
- **Records.** A delivered notice is logged as `notification_sent` and written to history. A full queue logs
  `notification_failed`; the daemon goes on.
- **Kinds:**
  - questions and invalid questions files;
  - hung, turn limit, paused, failed, no status;
  - start and run failures;
  - approved, merged, merge failed, PR closed, publish failed;
  - no verdict, second opinion;
  - team asleep, team done;
  - agent changed, no free agent;
  - low disk, disk wait.

**Notices are sent once per occasion**, and every repeating condition is armed and re-armed explicitly, so that
the human is not flooded.

## 13. The interface to `agentbus`

The daemon calls `agentbus` subcommands with a 15 s timeout each. The bus holds its lock only for one operation,
so a call that takes longer is stuck. The daemon parses their output and exit codes, which is why both are a
compatibility interface (AGENTS.md rule 9).

| Group | Subcommands |
|---|---|
| teams | `team-create`, `team-get`, `team-set`, `team-done`, `team-replace` |
| board and messages | `board-*`, `send`, `status` |
| workers | `delegate`, `helper-*` |
| investigations | `inv-next`, `inv-set`, `inv-get`, `inv-list` (and, from item 5.1b, `inv-add`) |
| checkpoints | `checkpoint-get` |

Agents see the bus as MCP tools (listed in `README.long.md`, "MCP tools available to agents"): messages,
reservations, the board, investigations, the notebook and delegation.

## 14. Diagnosing

| Command | Use |
|---|---|
| `agent status --running` | One line per working task. Look for `questions`, `failed`, a `waiting` team with an open round, or output unchanged since the last check. |
| `agent status --task <t>` / `--investigators` / `--json` | One task's history, the investigators' state, machine-readable output. |
| `agent log --team <team>` / `--round <r>` / `-f` | The story of a team or a round from the log events. `--show`, `--reminders` and `--stats` give other views. |
| `agent checkpoints <task>` | Where a takeover or renewal would continue from. |
| `agent debug --team <team>` | **Start here.** Writes a bundle (`SUMMARY.md`, state, logs, recent runs) to `debug/`. `--stale-minutes` (120) marks old activity. |
| `journalctl --user -u agent-daemon` | The daemon's structured log (`ts`, `level`, `component`, `event`, fields; see `docs/log.md`). |

### Log events worth knowing (73 in all; `docs/log.md`)

| Area | Events |
|---|---|
| runs | `run_started`, `run_ended`, `run_ended_without_status`, `run_hung`, `run_quota_exhausted`, `run_rate_limited` |
| decisions | `decision_taken`, with `decision` = `ladder`, `takeover`, `restart`, `context`, `limit`, `round_overdue`, `plan_team`, `choose_reviewers` … |
| rounds | `round_changed` (`round_opened`, `round_closed`, `delivery_*`), `item_phase_changed` |
| gate and checks | `gate_started`, `gate_finished`, `gate_timed_out`, `gate_dirty_tree`, `gate_retried`, `checks_decided` (`sent_back`, `asked_human`, `gate_reused`, `publish_failed_*`) |
| review and GitHub | `reviews_requested`, `review_submitted`, `review_report_late`, `github_call_failed`, `github_step_waiting` |
| watchdog | `commit_reminded`, `commit_followed`, `commit_cut`, `context_reminded`, `serena_reminded`, `task_handed_over` |
| requests | `investigation_started`, `investigation_requeued`, `investigation_failed`, `worker_request_*`, `worker_wait_returned` |
| lifecycle | `daemon_started`, `daemon_stopped`, `restart_corrected`, `checkpoint_failed`, `state_save_failed`, `bus_down`, `notification_sent`, `notification_failed`, `build_directory_removed` |

## 15. Known gaps and where the docs disagree with the code

These were found while writing this overview and checked against the code. Each one is worth an issue or a doc fix.

1. **Reviewer ties.** `docs/architecture.md:658` says "No random draw is used". The code
   (`crates/agent-core/src/decide/investigator.rs:85-89`) shuffles the candidates tied at the oldest activity.
   `docs/configuration.md:567` says reviewers are "drawn at random". The code is right; architecture.md is stale.
2. **Reviewer replacement.** The "Known limits" of `docs/architecture.md` (around line 822) say a reviewer whose
   run fails or is stopped "is not replaced: the human resumes it". The code (`decide/reviewer.rs` `replace_now`)
   and `architecture.md:603` replace it at once.
3. **Lock refusals.** A gate refused by the busy test lock (exit 75) is excluded from the "failed twice" reuse
   rule. However, it is still recorded as a failed gate, and the publish step (`decide/publish.rs`) has no case of
   its own for it. The commits titled "Rerun the gate after lock refusals" on `main` (798f5f920, 9e41d130a,
   c2aa9bc5e, eabc113aa) carry no changes. The working code came from 71305875d.
4. **Reserved keys.** `[limits] idle_team_restarts` (1) and `[review] allow_second_opinion_on_blocker` are
   documented as reserved or "stored only" in places, while the second is used in the review code. Check before
   relying on either.
5. **Configuration reference.** The top-level table of `docs/configuration.md` lacks `[disk]`,
   `[investigations]` and `[serena]`. Worker rotation is described differently in different places.
6. **Checkpoint in the run.** The `Checkpoint` field built for a run is still `None` (`owner/start.rs:443`). The
   briefing reads checkpoints from history instead.
7. **No same-vendor rule for reviewers.** Nothing stops a reviewer from the author's vendor. The provider matters
   only when a refused reviewer is replaced.
8. **Not on main yet:**
   - the conflict investigator pass (item 5.1b: `[merge] investigator_pass`, `merge_conflict_investigated`);
   - what agents learn reaching the next agent (the next task on the roadmap).
