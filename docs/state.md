# State, control protocol and questions files of the Rust `agent`

Three contracts of the Rust `agent` (see [`architecture.md`](architecture.md)) that every other part is built on:

| Contract | What it is | Code |
|---|---|---|
| **State** | what the daemon remembers: teams, their members and roles, the task list with the progress of each item, rounds, runs, questions, pull requests, reviews, fix passes, gate results | [`state`](../crates/agent-core/src/state.rs) |
| **Control protocol** | what the CLI and the dashboard say to the daemon and what it says back: JSON lines over its Unix socket | [`protocol`](../crates/agent-core/src/protocol.rs) |
| **Questions file** | the Markdown file in which an agent asks the human and the human answers | [`questions`](../crates/agent-core/src/questions.rs) |

They are one model, shared by the daemon, the CLI and the dashboard, so that none of them keeps its own idea of a team
or of an answer. This page is the reference for all three and, in the last section, lists the decisions in them that
are expensive to change once an installation has state files (the checkpoint after feature 2 of the plan: read this
page and [`configuration.md`](configuration.md)). It describes `agent-core`, which checks and (de)serializes values;
it does not read or write a file or a socket — that is the runtime's job (items 4.1 and 6.1).

Nothing here is the old Bash harness's `bus/state.json`, `meta.env` or `activity.jsonl`: those are not read, converted
or kept compatible.

## State

### The file

The state is one JSON document, written indented with a newline at the end, the schema version first:

```json
{
  "schema_version": 2,
  "teams": [ { "id": "team-20261111-193000-a1b2", "...": "..." } ]
}
```

A complete example, written by this format, is
[`state-v2.json`](../crates/agent-core/tests/fixtures/state-v2.json): three teams — one of three roles and two
reviewers with a merged round and an open one, one solo that does not merge by itself with an approved round, one on a
list without items — with the runs that name the rounds. A test loads it and writes it back to the same text; **it must
keep loading** — a later build that cannot read it would lose the teams of an installation.

[`state-v1.json`](../crates/agent-core/tests/fixtures/state-v1.json) is a state of version 1, which had no rounds. It
**must keep loading**: it is read through the migration (below) as exactly
[`state-v1-as-v2.json`](../crates/agent-core/tests/fixtures/state-v1-as-v2.json), every time.

### Reading and its rules

- **Strict.** A field that is not on this page is an error, in every object (`deny_unknown_fields`); so is a missing
  one, except the optional ones that are marked. A typo does not fall back to a default.
- **Typed.** Every value is parsed into its type when the file is read: IDs by their form, times as RFC 3339, enums by
  their texts. A value its type refuses is an error at its line and column.
- **Checked.** What the types cannot say, reading checks too (the invariants below). A state that is read is as valid
  as one that was built through the API. There is one way to read a value, the `Deserialize` trait: the field-by-field
  reading that serde derives lives on a private mirror type (`remote`), so no public `T::deserialize` skips the checks.
- **Versioned.** `State::from_json` reads `schema_version` **first**. A state of another version is refused with
  `StateError::UnsupportedSchema { found }` — never as "malformed", whatever else it holds. This build writes version 2,
  reads version 2 and migrates version 1 as it reads it (see "Version 1" below); any other version, 3 included, is
  refused. A change that old files do not satisfy raises the version and comes with a migration, a fixture of the old
  format and tests (the same rule as `AGENTS.md` rule 9 sets for the old harness). Stricter validation of an old value
  is such a change too.
- **Bounded.** The text may have 64 MiB (`MAX_STATE_BYTES`).
- **Errors.** `StateError`: `TooLarge`, `UnsupportedSchema`, `Malformed` (JSON or value errors, with the parser's
  reason as the source), `Invalid` (two teams, two tasks or two rounds of different teams with one ID, or no free ID for
  a round of a state of version 1).

### What it holds

```text
State
└── teams[]  Team            id, repo, created_at, merge, task_list (text), items[], rounds[], crew, reviewers[]
    ├── items[]  ItemProgress    id, phase, fix_passes[], gate        one per item of the task list
    ├── rounds[] Round           id, item?, opened_at, state (open | closed: at, how)   the work on each item that has started
    ├── crew     Crew            solo (one member) or team (coordinator, members[], workers[])
    └── reviewers[] Member       investigators outside the crew, one task each
        Member                   task, agent (tier + name), status, runs[], questions[]
        ├── runs[]      Run          id, started_at, session, rounds[], state (running | ended: at, end)
        └── questions[] Question     number, file, asked_at, state (open | answered: at, via)
```

**A solo task is a team of one.** Every unit of work is a `Team`; the solo agent is its only member, with the role
`solo`. One code path serves both.

#### `Team`

| Field | Type | Meaning |
|---|---|---|
| `id` | `team-YYYYMMDD-HHMMSS-xxxx` | generated from the clock and the random source |
| `repo` | repository name | the `[repos.NAME]` of the configuration the team works on |
| `created_at` | time | when the team was started |
| `merge` | `autopilot` \| `no_merge` | what happens to an approved pull request (`--no-merge`) |
| `task_list` | text | the task list **as given**; reading parses it again, so a list that no longer parses is an error |
| `items` | list | the progress of each item: exactly one entry per item of the list, in its order |
| `rounds` | list | the team's rounds, oldest first; absent = none (see "Rounds" below) |
| `crew` | object | see below |
| `reviewers` | list | the investigators that review the team's pull requests, each with its task; absent = none |

#### `Crew` and roles

| `kind` | Fields | Rules |
|---|---|---|
| `solo` | `member` | one normal agent or investigator; role `solo` |
| `team` | `coordinator`, `members`, `workers` | the coordinator is a normal agent or an investigator; `members` (at least one) take items and are normal agents or investigators — a second investigator in a team is a member; `workers` serve `delegate` requests, in the configured order, and are workers |

A member's **role** is its place in the crew — it is not stored twice. Roles are `solo`, `coordinator`, `member`,
`worker` and, for the reviewers of a team, `reviewer` (an investigator outside the crew). A participant is written
`{"tier": "normal" | "worker" | "investigator", "name": "codex-1"}`; the names of the three tiers share one namespace,
as in the configuration.

Invariants: a role gets only a tier that fits it (`worker` role ⇔ `worker` tier; a `reviewer` is an `investigator`); a
team has at least one member besides its coordinator; no name and no task ID twice in a crew; a reviewer is not in the
crew, not twice a reviewer, and its task is not the crew's. Reading checks the same.

#### `Member`, `Run`, `Question`

| Field | Meaning |
|---|---|
| `task` | the task's ID, `task-YYYYMMDD-HHMMSS-xxxx`, unique in the **whole** state |
| `agent` | the participant |
| `status` | `queued`, `running`, `waiting`, `questions`, `paused`, `stopped`, `failed`, `done`; it changes only along the table of [`TaskStatus`](../crates/agent-core/src/status.rs) |
| `runs` | oldest first; absent = none. Only the last may be going; no ID twice; a run ends at or after its start |
| `questions` | oldest first; absent = none. Numbered 1, 2, 3, …; only the last may be open; an answer is at or after its question |

A `Run` is `{id, started_at, session?, rounds?, state}`; `rounds` are the rounds it works for (below), oldest first, absent =
none; its state is `running` or `ended` with a time and **how**:

| `end.kind` | Fields | Meaning |
|---|---|---|
| `reported` | `outcome` | the agent's final message ended with `STATUS: DONE`, `QUESTIONS`, `FAILED`, `SLEEPING` or `EXCLUDED` |
| `no_status` | `exit_code?` | the CLI ended without a `STATUS:` line (no code: a signal) |
| `quota_exhausted` | `resets_at?` | the quota is used up; the time the CLI gave, if it gave one |
| `rate_limited` | | the provider limited the rate of requests |
| `hung` | | no activity for too long; the watchdog stopped it |
| `stopped` | | the human stopped it |

The `session` is the model CLI's session ID, which a later run resumes (the latest run that has one).

A `Question` is `{number, file, asked_at, state}`: the questions file `questions-<number>.md` of the task, and
`open` or `answered` with the time and the channel (`command` = `agent answer`, `saved_file` = the file was saved with
the `**ANSWERED**` marker, `dashboard`). **The questions and the answers are in the file, not in the state.**

#### Items

An item's progress is `{id, phase, fix_passes?, gate?}`. The `phase` carries exactly the data of that phase:

| `phase` | Data | Item status | Moves to |
|---|---|---|---|
| `pending` | | `pending` | `in_progress` |
| `in_progress` | `assignee`, `branch` | `in_progress` | `pending` (the member left), `in_review` |
| `in_review` | `pr`, `reviews[]` | `in_review` | `fixing` (changes requested), `approved` |
| `fixing` | `assignee`, `branch`, `pr` | `in_progress` | `in_review` |
| `approved` | `pr`, `reviews[]` | `approved` | `done` |
| `done` | `completion`: `listed` (the list marked it `[x]`) or `merged` (`pr`, `at`) | `done` | — |

The table is `PhaseKind::successors`; every change in it is a change `ItemStatus` allows, and every change `ItemStatus`
allows is in it (two tests). The phase `fixing` exists so that the pull request survives a fix pass. Leaving
`in_review` for `fixing` records the reviews of the review pass as a **fix pass** (`fix_passes[]`, oldest first); their number is
what `[review] fix_passes` of the configuration is compared with. A member who leaves while an item is `in_progress` or
`fixing` is replaced with `reassign`, which keeps the pull request.

`pr` is `{number, branch, opened_at}`. A `review` is `{reviewer, task, requested_at, outcome}`; its outcome is `pending`
or `submitted` with the `APPROVE` / `REQUEST_CHANGES` verdict and a time. A `gate` result is
`{head, finished_at, outcome, log}`: the commit it ran on, `passed`, `failed` (`exit_code`, never 0: a zero is a pass
and is refused when read) or `timed_out`, and the file with the gate's output.

Invariants: one progress entry per item of the task list, in its order; an item the list marks `[x]` is `done` as
`listed` from the start, and only such an item is; an item `fixing` records at least one fix pass.

The reviews of a **review pass** — the reviews asked for one head of a pull request (`in_review`, `approved`, a fix pass) —
name no reviewer and no task twice. An `approved` item carries at least one review and every one is a submitted `APPROVE`. A fix
pass records at least one review, is not a review pass that every reviewer approved, and contains a submitted `REQUEST_CHANGES`.
`advance` checks the reviews of the new phase and of the review pass it leaves, and reading checks the same (the phase, and every
fix pass), so a state that `advance` could not have made is not read. Other reviews in the review pass of a fix pass may still be
pending.

The review pass of an item that has just entered `in_review` is empty; it is filled once with the reviews asked of the reviewers
(`ItemMut::ask_reviews`: all `pending`, none twice, and not again for the same review pass), so a pull request can be open while it
waits for two free investigators.

A review that waits (`pending`) receives its outcome through `submit_review` (`ItemMut::submit_review`): by the
reviewer's task, only while the item is `in_review`, and once — a second outcome for the same review is an error, and
so is a task that was not asked for a review. The review pass is then `advance`d: to `fixing` once a `REQUEST_CHANGES` is in,
to `approved` with every review approving. `done` as `listed` is only the starting state of an item the list marks
`[x]`: `advance` refuses it (`ListedCompletion`), because the state would save but not load again; an item a team
finishes is `done` as `merged`.

#### Values

| Value | Form |
|---|---|
| time | RFC 3339 in UTC, years 0000–9999: `2026-11-11T19:15:54Z` (fractions only when there are some). Any offset is read and converted |
| session ID | 1–200 bytes, no whitespace or control characters |
| commit ID | 40 or 64 lowercase hex digits |
| pull request number | a positive number |
| paths | absolute |

#### Rounds

A **round** is the work of one team on one item of its task list, from the item's start to its merge (with `--no-merge`: to
its approval) — or the whole work of a team whose list has no items. One round, one branch of the lead, one pull request.
"Round" means this and nothing else here: the numbered questions of a task are its `number`, and the reviews asked for one head
of a pull request are a review pass.

A round is `{id, item?, opened_at, state}`:

| Field | Meaning |
|---|---|
| `id` | `round-YYYYMMDD-HHMMSS-xxxx`, generated like the other IDs; unique in the whole state |
| `item` | the item the round is for; absent for the whole work of a team whose task list has no items |
| `opened_at` | when the daemon opened it: when the item was first given to a member, or when the team started |
| `state` | `{"status":"open"}`, or `{"status":"closed","at":…,"how":…}` with `how` one of `merged`, `approved` (`--no-merge`: both reviews approved, the pull request is the human's) and `abandoned` (the team was stopped or ended without a merge or an approval) |

A run names the rounds it works for (`rounds`, above). Invariants, checked by the operations (`Team::open_round`, `close_round`,
`advance_item`, `begin_run`, and `State::open_round`, `close_round`, `begin_run` for what needs the whole state) and again when the
state is read:

- an item has at most one round, and has one as soon as it leaves `pending`; an item the task list marks `[x]` has none, and a
  round is not opened for an item that is done;
- a round without an item exists only in a team whose task list has no items, and there is one at most;
- a round ID is used once in the whole state;
- a closed round is never opened again: closing it again is refused, and so is a second round for its item. A round closed as
  `merged` belongs to an item `done` as merged; one closed as `approved` to an item that is `approved` or `done` as merged;
  `abandoned` fits any item (a round of a whole list can end in no other way, since nothing merges it); it is not closed before it
  was opened;
- a run names only rounds of its own team, none twice.

`Team::advance_item` is the move of an item that checks the first rule; `ItemMut::advance` moves an item without knowing the
rounds, and the state that is built with it alone is not read until the item has its round. A round that is still `open` when its
item is `done` is allowed to be read: the daemon closes it in the same step as it completes the item.

A task that is resumed after a stop goes on with its item in the round it had, which stays `abandoned`; the runs that go on name
it. (The alternatives — a new round for the same item, or a round that opens again — break the invariants above.)

**Version 1.** A state of schema version 1 has no rounds. It is migrated when it is read, by one rule, so that reading the same
file twice gives the same state: each item past `pending` (not the ones the list marks `[x]`) gets a round, whose ID is derived
from its team's ID and its place in the list — the team's date and time, and the team's suffix counted up by the place plus one in
the digits `0-9a-z` (`RoundId::derive`; if another team already has that ID the place a whole list later is tried, and so on, up
to eight times) — opened at the time its team was started (version 1 did not record when an item was given out). Its state: an
item `in_progress`, `in_review` or `fixing` has an open round; an item `approved` has one closed as `approved` at its last
review when its team does not merge by itself, an open one otherwise; an item `done` as merged has one closed as `merged` at the
time of the merge. The runs of version 1 name no round, and the `round` of a question is its `number`. A list without items gets
no round (it had none to open). Version 1 is read only through this migration; the saved file has version 2.

### What is not in the state

Each is derived or lives elsewhere, so that there is one place for it:

- **The configuration** — read from `agents.toml`; the state refers to it by name (`repo`, the names of agents). A
  name that the configuration no longer has is the daemon's problem to report, not a reason to refuse the state.
- **The text of questions and answers** — in `questions-<number>.md`.
- **Activity** — what a run did, in its activity log, and what the human did to the task (`{"kind":"control","action":"pause","by":{"kind":"cli"}}`, and an `info` line for an answer); the state keeps only how a run began and ended.
- **Review reports, gate output** — in files; the state keeps their verdicts and paths. A gate run is in
  `tasks/<task>/gate-<n>.log` (its output) and `tasks/<task>/gate.json`, the task's last **gate record**
  `{"tree":"clean"|"not_clean","result":{head, finished_at, outcome, log}}`: the daemon's own record of the run, which says
  whether the tree was clean at the start and at the end of the command (a changed or untracked file, a moved `HEAD`, or a
  merge waiting to be committed, is not). Only a `passed` on
  a `clean` tree is a pass of `head`.
- **The checks before a review** — `tasks/<lead>/checks.json`, the **checks record** of a team's lead, written by the daemon only:
  `{"items": {"<item id>": {"passes": N, "allowed": [<subject>, ...]}}, "asked": {...}, "sent": {...}}`. For each item the passes the
  checks sent its round back and what the human allowed (a subject is `{"path": "tests/a.rs"}` or `"no_tests"`); `asked` is the
  question the round waits on (the `number` of the lead's questions, when it was written, the items, what each question is about, how
  often the lead was woken to put it); `sent` is what the lead is to be told on its next run. Empty parts are left out and an empty
  record is `{}`. A task the checks have not seen has no file. See "Round checks before the review" in
  [`architecture.md`](architecture.md).
- **Worktree and branch paths, the unit names** — derived from the IDs and the layout of `AGENTS_HOME`.
- **A team's overall status** — derived from its members and items (`TeamSummary`).
- **Tokens and cost** — out of scope of the plan.

## Control protocol

### Framing

JSON lines over the daemon's Unix socket: one message per line, ended by `\n`, no embedded newline (JSON escapes it).
A client sends a **request**; the daemon answers with a **response** carrying the request's `id`. A client that sent
`subscribe` also receives **events** on the same connection. The daemon's lines are told apart by `type`.

| Direction | Line | Limit |
|---|---|---|
| client → daemon | `{"protocol":2,"id":N,"client":{…},"request":{"command":"…",…}}` | 2 MiB (a task list is at most 1 MiB) |
| daemon → client | `{"type":"response","protocol":2,"id":N,"result":{…}}` | 32 MiB |
| daemon → client | `{"type":"event","protocol":2,"at":"…","event":{"kind":"…",…}}` | 32 MiB |

The `id` is the client's number for the request, any `u64`, echoed so that a client can match answers. The `client`
says who asks — `{"kind":"cli"}` or `{"kind":"dashboard","login":"you@example.com"}` — and the daemon names it in the
activity of a control (who paused, who answered).

### Versions

Every line carries `"protocol": 2`. A reader looks at **the version first** and refuses a line of another version with
`ProtocolError::UnsupportedVersion { found }`, whatever else the line says: a client of version 3 that sends a request
this daemon does not know gets that answer and not a parse error, and so does a line of version 1. A line without a usable
version, or not JSON, is `Malformed` (with the parser's reason); a line over the limit is `TooLong`. A change to a message that
old peers cannot read raises the version.

Version 2 added the round — the target `{"round":"round-…"}`, the rounds in a team's summary, the reply `round`, the events
`round_opened` and `round_closed`, and the round in the events about an item or a run — and renamed the `round` of the questions
of a task (`answer`, `question_asked`, `question_answered`) to `number`.

### Requests

| `command` | Fields | Answer (`reply.kind`) |
|---|---|---|
| `run` | `repo`, `checkout` (absolute), `agents` (1–16 names, none twice), `task_list` (text, parsed when read), `coordinator?`, `merge` (`autopilot` \| `no_merge`) | `started` |
| `status` | `target?` | `teams` |
| `show` | `target` | `team`, `task` or `round` |
| `answer` | `task`, `number`, `answers` (1–256 × `{question, verdict}`, none for a question twice) | `applied` |
| `stop`, `pause`, `unpause`, `resume` | `target` | `applied` |
| `say` | `target`, `text` (not blank, at most 64 KiB, no NUL) | `applied` |
| `gate` | `task` | `gate_recorded` |
| `subscribe` | `team?` | `subscribed`, then events |

A `target` is `{"team": "team-…"}`, `{"task": "task-…"}` or `{"round": "round-…"}`; the controls (`stop`, `pause`, `unpause`,
`resume`, `say`) take a team or a task and refuse a round with `invalid`. A `verdict` is a string: `ok` (any case) accepts the
recommendation, any other text is the human's own answer. The daemon checks `answers` against the questions file of the questions
with that `number` (every question answered, `ok` only where there is a recommendation).

### What the daemon serves

Since item 6.1 the daemon serves `status`, `show` and `subscribe`; since item 7.1 also `run`: it forms the team, makes its
worktrees and its team on the bus and answers `started`, or refuses with `invalid` (an unknown agent or repository, a worker
as coordinator, only workers, a list with nothing to do, a base branch the checkout does not have), `refused` (too many
investigators for two outside reviewers) or `internal` (git or the bus failed; what was made is taken back). `status` takes an optional target: a team's summary, the
summary of the team a task or a round belongs to, or every team; an unknown target is `not_found`. `show` gives a whole team, a
task with its role, or a round with its item, its state, the tasks that worked for it (those with a run that names it) with their
role, agent, worktree and branch and the runs, its pull request and its review passes. `subscribe` answers with the summaries and then sends the events described below; the events of one team only
when one was asked for.

Since item 7.2 also `answer`, `stop`, `pause`, `unpause`, `resume` and `say`, each answering `applied` with the tasks it
reached:

- `answer` takes the verdicts for the questions that are open; `not_found` for an unknown task, `conflict` for a task that does
  not wait for answers or questions that are not the open ones, `invalid` (every problem listed) for a question the file does not
  have, a question left without a verdict, or `ok` where there is no recommendation. The channel recorded is `command` for the
  client `cli` and `dashboard` for the client `dashboard`; `saved_file` is for a file the daemon read itself.
- `stop`, `pause`, `unpause`, `resume` decide each task of the target by its status (the table in
  [`architecture.md`](architecture.md)); `conflict` when the control reaches none, with the reason for each task, and
  `not_found` for an unknown target. A control of a team reaches its reviewers too.
- `say` puts the text on the agentbus from `human`, for the task's agent or for the team; `conflict` for a target that is done,
  `internal` when the bus does not take it.

Since item 7.3 also `gate`: the daemon queues the gate of the task (one gate at a time on the machine, in the order asked)
and answers `gate_recorded` **when the gate is over**, so a client waits as long as the queue and the gate take
([`architecture.md`](architecture.md), "The gate"); `not_found` for an unknown task, `refused` for a repository the
configuration lost, a worktree git cannot read, a gate command that cannot be started and a full queue (16 waiting),
`conflict` when the task was stopped or paused, or the daemon shut down while the gate ran, before the gate was over (nothing
is recorded; a gate that waits for its turn when its task is stopped keeps its place in the queue and gets this answer when
its turn comes, without running), and `internal` otherwise: a gate still waiting when the daemon shuts down gets `internal`
("the daemon is shutting down"), and so does a worktree git cannot read after the command ran (nothing is recorded; the
message names the log).

A line that is not a valid request is answered with `invalid` under the request ID `0` (its own ID cannot be
read) and the connection goes on; a request over the limit is answered with `invalid` and the connection is closed.

### Responses

`result` is `{"status":"ok","reply":{…}}` or `{"status":"error","error":{"kind":"…","message":"…"}}`.

| `reply.kind` | Fields |
|---|---|
| `started` | `team`, `tasks` |
| `teams` | `teams`: summaries — `id`, `repo`, `created_at`, `merge`, `items` (counts by status), `rounds` (the team's rounds, as in the state; absent = none), `members` (task, agent, role, status, `open_question?`: the number of the questions that wait) |
| `overview` | what `status` answers instead of `teams` when the history ([`history.md`](history.md)) lost records or cannot be written: `teams`, as above, and `history` — `state` (`{"kind":"writing"}`, `{"kind":"disabled"}` or `{"kind":"unavailable","reason":"…"}`) and `lost`, the number of records handed to the history that are not in it. A history that is written and whole, or off, is not mentioned: the answer is `teams` |
| `team` | `team`: the whole team record, as in the state |
| `round` | `round`: `team`, `round` (as in the state), `item?` (`id`, `title`, `status`), `tasks` (`task`, `role`, `agent`, `worktree`, `branch`, `runs`: the runs that name the round), `pr?` and `review_passes` (each fix pass, then the pass that is going: `reviews`; the reviews of the pass that was approved are not kept once the item is done) |
| `task` | `team`, `role`, `member` |
| `applied` | `tasks`: the tasks the command reached |
| `gate_recorded` | `record`: a gate record — `tree` (`clean` \| `not_clean`) and `result`, a gate result |
| `gate` | `result`: a gate result; read, but the daemon sends `gate_recorded` instead |
| `subscribed` | `teams`: the summaries of the teams subscribed to (all, or the one asked for), as they are when the events start |

`error.kind` is one of `invalid` (well-formed but asks for something that makes no sense: an unknown agent, a worker as
coordinator), `not_found`, `conflict` (the state does not allow it now), `refused` (the harness refuses to start: too
many investigators in the team for two outside reviewers) and `internal`. The `message` is for the human — what went
wrong and what to do — and carries no internals.

### Events

`event.kind`, each with the `team` it is about:

| `kind` | Other fields |
|---|---|
| `team_started` | |
| `team_updated` | `summary`: the team's whole new summary |
| `task_status` | `task`, `status` |
| `round_opened` | `round`, `item?` (absent: the whole work of a list without items) |
| `round_closed` | `round`, `item?`, `how` (`merged`, `approved`, `abandoned`) |
| `item_status` | `item`, `round`, `status` |
| `run_started` | `task`, `run`, `rounds?` (the rounds the run works for; absent = none) |
| `run_ended` | `task`, `run`, `rounds?`, `end` |
| `question_asked` | `task`, `number` |
| `question_answered` | `task`, `number`, `via` |
| `pull_request_opened` | `item`, `round`, `pr` |
| `review_submitted` | `item`, `round`, `reviewer`, `verdict` |
| `gate_finished` | `task`, `outcome` |
| `controlled` | `task?` (absent: the whole team), `action` (`stop`, `pause`, `unpause`, `resume`, `say`), `by` (the client) |

The lines of version 2 are pinned by [`protocol-v2-requests.jsonl`](../crates/agent-core/tests/fixtures/protocol-v2-requests.jsonl)
and [`protocol-v2-frames.jsonl`](../crates/agent-core/tests/fixtures/protocol-v2-frames.jsonl), and the lines about rounds by
[`protocol-v2-round-requests.jsonl`](../crates/agent-core/tests/fixtures/protocol-v2-round-requests.jsonl) and
[`protocol-v2-round-frames.jsonl`](../crates/agent-core/tests/fixtures/protocol-v2-round-frames.jsonl): a test reads each line
and writes it back to the same text. The lines of version 1 ([`protocol-v1-requests.jsonl`](../crates/agent-core/tests/fixtures/protocol-v1-requests.jsonl),
[`protocol-v1-frames.jsonl`](../crates/agent-core/tests/fixtures/protocol-v1-frames.jsonl)) are kept: a test checks that each is
refused as an unsupported version.

## Questions files

An agent (or the harness) asks the human by writing `questions-<number>.md` and ending its run with `STATUS: QUESTIONS`.
The human answers in the file or in the dashboard.

```markdown
# Questions — task-20261111-153601-k3x9

> Answer every question after **Verdict:** — `ok` accepts the recommendation. When you are done, put **ANSWERED** on the
> last line and save: the answers are sent at once (or run `agent answer task-20261111-153601-k3x9`).

## 1. Which store?
a) in memory
b) on disk

**Recommended:** a) — simpler (inv-opus)

**Verdict:** ok

## 2. Żółć or bile?

**Recommended:** bile — the key is taken

**Verdict:** Bile, and please rename the key.

**ANSWERED**
```

### Format

- **Title** — an optional `# ` line at the top.
- **Instructions** — the block quote right under the title: the text that tells the human what to do. It comes from a
  template (`prompts/harness/questions/instructions.md`, variable `{{task_id}}`), which the runtime renders; the code
  has no wording. `set_instructions` puts it under the title, replacing any there are, so it is added once.
- **Free text** — anything else before the first question.
- **A question** — `## N. title`, N a number from 1 to 4294967295, unique in the file. Then its text (the options, the
  context) and its labelled parts, kept in the order written.
- **Labelled parts** — `**Label:** text`, the text on the same line and the lines after it up to the next label. The
  labels are fixed English words whatever language the questions are in; spaces inside the asterisks are tolerated; the
  words are case-sensitive. `**Recommended:**` and `**Verdict:**` are read; any other label is kept as written. A
  question may have each of the two at most once.
- **The marker** — `**ANSWERED**` as the **last non-empty line** says the human is done. Anywhere else it is dropped
  and says nothing.

### Verdicts

The human's answer to a question is its verdict: **`ok`** (any case, nothing else) accepts the recommendation; **any
other text** is the human's own answer, and may span lines; **blank** means not answered yet. `QuestionsFile::answers`
turns a file into the answers or lists **every** question that cannot be answered — without a verdict, or `ok` where
there is no recommendation — so that the human is told all at once.

### What the agent gets

`QuestionsFile::expand` returns the answered file for the agent: the instructions and the marker removed, and a verdict
`ok` replaced by words that tell the agent the human accepted the recommendation, followed by the recommendation. The
words are a template (`prompts/harness/questions/accepted.md`, variable `{{recommendation}}`) that the runtime renders and
passes in. An unanswered file is not expanded, and words that cannot be a verdict (blank, `ok`, a line that reads as the
file's structure) are an error that names the question.

### Reading and writing

`QuestionsFile::parse` reads a file (at most 1 MiB), `render` writes one; rendering a parsed file and parsing the result
gives an equal file. A file written by a model in the format of the prompts parses; a text with no `## N.` question, a
number 0 or too large, two questions with one number and a second `Recommended` or `Verdict` in a question
are errors with their line. `set_verdict` changes only the verdict of one question and `mark_answered` the marker: every
other part of the file stays as it was, which is how the dashboard saves a draft. The text the old harness called
`questions_note` and the old labels `Answer` and `Odpowiedź` are not kept.

## Decisions to review

These choices are in the model and are expensive to change once state files exist. Each is easy to reverse now.

1. **A solo task is a team of one** (role `solo`), not a separate kind of record.
2. **A role is a place in the crew**, not a stored field. Investigators may be coordinators, members (a second
   investigator in a team) and, outside the crew, reviewers. Workers only serve `delegate`. A team needs at least one
   member besides the coordinator.
3. **Reviewers are tasks of the team they review** (`reviewers`), each a `Member` with its own runs, so a review run can
   be stopped, resumed and answered like any other. A review in the item points at its reviewer's task.
4. **The questions are not in the state**, only that they were asked, when, and through which channel; the file is the
   one source of the text, and a state never grows with the answers.
5. **A task list is stored as its text** and parsed when read, so it is the list the human wrote. A task-list format that
   becomes stricter would make an old state unreadable: that is a schema change, with a migration.
6. **The phase `fixing`** keeps the pull request through a fix pass; fix passes are counted by their records, not by a
   counter.
7. **One run at a time per task; questions are numbered 1, 2, 3, …** without gaps, and earlier questions must be
   answered first.
8. **`RunEnd` lists what ends a run** (reported status, no status, quota, rate limit, hung, stopped). Item 3.1
   classifies a final message and an exit status into it; a new kind of ending is a new variant and a schema change.
9. **Time is RFC 3339 in UTC** as text, not Unix seconds: a human can read a state file.
10. **The protocol carries `repo` and `checkout`** in `run`: the daemon creates the worktrees from the client's checkout
    and takes the base branch and the gate from the configuration by the repository's name.
11. **`answer` carries verdicts, not a file**: the client reads the file (`--file`, or the editor) and sends the verdicts;
    a file saved with the marker is read by the daemon itself. Both go through the same code.
12. **`status` returns summaries, `show` the whole record**: a list of teams stays small, however long the task lists.
13. **`subscribe` starts from the state**: its reply carries the summaries of the teams, taken when the events start,
    so a dashboard needs no separate `status` and loses no event between the two. Summaries, not whole teams: one
    line stays small. The events keep them up to date through `team_updated`, which carries the whole new summary
    (`EventKind::apply_to` is the client's reducer): `team_started` and `item_status` say what happened but are too
    little to change a summary (what does a new team look like? which count does an item leaving `pending`
    change when others are pending or in review?). The daemon sends `team_updated` after every change that alters a
    summary, right after the event that says what happened.
14. **Strict in both files and lines**: unknown fields are errors, and the protocol version is checked first. A newer
    client against an older daemon is told so, not misread.
15. **A state is changed through handles, never through `&mut` to a whole value**: `State::team_mut` gives a `TeamMut`,
    which gives an `ItemMut`, a `CrewMut` and `MemberMut`s. Each reads like the value it stands for (`Deref`) and offers
    only that value's checked operations (`advance`, `transition`, `begin_run`, ...), so no caller can overwrite a team,
    an item, the crew or a member and bypass the checks that keep IDs unique and phases in order. The compiler refuses
    the overwrite (compile-fail doctests on `State::team_mut` and `State::find_task_mut`).
16. **A round is a record of its team, found by the item.** It is `{id, item?, opened_at, state}`, an item has one, and a run
    names the rounds it works for (a list, because a coordinator works for every round that is open). The ID of a team and the ID
    of a round are the two keys the history of item 8.4 is found by.
17. **A closed round is never opened again, also when its team is resumed.** A `stop` that leaves a team with nobody to work
    abandons its open rounds; a `resume` goes on with the item in its round, which stays `abandoned`, and the runs that go on name
    it. The other ways — a new round for the same item, or a round that opens again — break "an item has one round" and "a closed
    round is closed". The cost: such an item is merged in a round that says `abandoned`. A human who wants a clean round starts a
    new team.
18. **The round of a list without items ends `abandoned`.** Nothing merges it and nothing approves it; `abandoned` is the only
    ending that fits it, and it is what the team's end says.
19. **The migration of version 1 is by one rule and opens its rounds when the team was started.** Version 1 did not record when
    an item was given out; the derived ID and the time of the team's start make reading the same file twice give the same state.
20. **The item's own move does not know rounds; the team's does.** `ItemMut::advance` is unchanged; `Team::advance_item` refuses to
    take an item out of `pending` without its round, and reading checks it for every state.
21. **`number` for the questions.** The key `round` of a question is `number` in the state, in `answer` and in the two events;
    the record of the checks (`checks.json`) reads the old key as well as the new.
22. **A round is looked at with `status` and `show`, not controlled.** A control of a round is `invalid`: stop its team or one of
    its tasks.
