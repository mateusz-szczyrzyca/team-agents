# State, socket protocol and questions files

`agent daemon` is the single owner of the state and of every model run. It persists its state as one JSON document,
`state.json`, and serves every CLI command (`agent run`, `status`, `show`, `answer`, `stop`, `pause`, `unpause`,
`resume`, `say`, `gate`) and the web dashboard (`agent-web`, via `subscribe`) over its Unix domain socket.

`agent-core` defines the three pure contracts without performing file or socket I/O:

- [`crates/agent-core/src/state.rs`](../crates/agent-core/src/state.rs) — the persisted state (`State`) and its schema
  version (`SCHEMA_VERSION = 1`).
- [`crates/agent-core/src/protocol.rs`](../crates/agent-core/src/protocol.rs) — the JSON-line socket messages
  (`Request`, `Response`, `Event`) and their protocol version (`PROTOCOL_VERSION = 1`).
- [`crates/agent-core/src/questions.rs`](../crates/agent-core/src/questions.rs) — the Markdown questions-file parser,
  renderer, answer extractor and template instruction injector (`QuestionsFile`).

## Persisted state (`state.json`)

### Schema version and bounds

```text
SCHEMA_VERSION  = 1
MAX_STATE_BYTES = 16 MiB (16,777,216 bytes)
```

Every `state.json` document carries `"schema_version": 1`. `State::from_json_str` checks the byte size and inspects a
minimal `{ "schema_version": u32 }` header before deserializing `teams` and `tasks`, so a document written by another
schema version is always refused with `StateError::UnsupportedVersion { found, supported: 1 }` even if future versions
change inner fields. Direct `serde_json::from_str::<State>` enforces the same version and cross-record invariant checks
through `#[serde(try_from = "RawState", into = "RawState")]`.

### Records

| Type | Role |
|---|---|
| `State` | Top-level snapshot: `schema_version`, `teams: BTreeMap<TeamId, Team>` and `tasks: BTreeMap<TaskId, Member>` (serialized in JSON as sorted `teams` and `tasks` arrays). |
| `Team` | A coordinated team working through a [`TaskList`](task-list.md): `team_id`, `repo`, `topic`, `status` (`TeamStatus`: `working`, `paused`, `stopped`, `failed`, `done`), `merge_mode` (`MergeMode`: `autopilot` or `no_merge`), `coordinator` (`Participant`), `created_at`, `updated_at`, `pause`, `features` (`Vec<FeatureSpec>`), `items` (`BTreeMap<ItemId, ItemProgress>`, serialized as an `items` array), `member_tasks` (`Vec<TaskId>`). |
| `Member` | One agent task (solo or in a team): `task_id`, `team_id` (`Option<TeamId>`), `repo`, `topic`, `participant` (`Participant`: `Normal(AgentName)`, `Worker(AgentName)`, `Investigator(InvestigatorName)`), `cli_kind` (`CliKind`), `model` (`Option<ModelName>`), `role` (`Role`), `status` (`TaskStatus`), `branch` (`BranchName`), `base_commit` (`CommitSha`), `head_commit` (`Option<CommitSha>`), `worktree_path` (`AbsolutePath`), `item_id` (`Option<ItemId>`), `created_at`, `updated_at`, `pause` (`Option<PauseState>`), `pull_request` (`Option<PullRequest>`), `runs` (`Vec<Run>`), `questions` (`Vec<Question>`), `reviews` (`Vec<Review>`), `last_gate` (`Option<GateResult>`). |
| `ItemProgress` | Progress of one plan item (`ItemId`): `item_id`, `title`, `feature_number`, `status` (`ItemStatus`), `round`, `assigned_task`, `pull_request`, `gate_result`, `reviews`. |
| `Run` | One model CLI invocation within a task: `run_id` (`RunId`), `run_number` (`NonZeroU32`), `started_at`, and `state` (`RunState`: `Running`, `WaitingOnGate`, or `Finished(RunFinish)` with `RunEnd::{StatusLine(RunOutcome), Signal(i32), WatchdogTimeout, SpawnFailure(String)}`, `exit_code`, `finished_at`, `commit`). |
| `Question` | One clarification round (`questions-<round>.md`): `round` (`NonZeroU32`), `asked_at`, `questions` (`Vec<questions::Question>`), and `answer` (`Option<RecordedAnswer>` with `verdicts: BTreeMap<NonZeroU32, Verdict>`, `source: AnswerSource::{Cli, FileWatch, Web}`, `answered_at`). |
| `Review` | One investigator review verdict on a pull request: `reviewer` (`InvestigatorName`), `task_id`, `verdict` (`ReviewVerdict`: `APPROVE` or `REQUEST_CHANGES`), `commit` (`CommitSha`), `recorded_at`, `summary`. |
| `GateResult` | Outcome of running a repository's configured gate command: `outcome` (`GateOutcome`: `passed`, `failed`, `timed_out`), `exit_code`, `commit` (`CommitSha`), `tree_state` (`TreeState`: `clean`, `dirty`, `merge_in_progress`), `started_at`, `finished_at`, `failed_step`, `summary`. `GateResult::passed()` requires `outcome == Passed` and `tree_state == Clean`. |
| `Timestamp` | Whole-second UTC timestamp formatted as `YYYY-MM-DDTHH:MM:SSZ`. |
| `CommitSha` | 7 to 64 lowercase ASCII hexadecimal characters (`0-9`, `a-f`). |

### State invariants and errors (`StateError`)

| Variant | When |
|---|---|
| `TooLarge { size, max }` | Input exceeds `MAX_STATE_BYTES` (16 MiB). |
| `UnsupportedVersion { found, supported }` | `schema_version` is not `SCHEMA_VERSION` (`1`). |
| `Parse(serde_json::Error)` | Malformed JSON or an invalid field value (`Timestamp`, `CommitSha`, ID, keyword enum, ...). |
| `DuplicateTeam(TeamId)` | Two teams share the same `TeamId`. |
| `DuplicateTask(TaskId)` | Two member tasks share the same `TaskId`. |
| `DuplicateItem { team_id, item_id }` | Two items inside a team share the same `ItemId`. |
| `UnknownMemberTask { team_id, task_id }` | `team.member_tasks` names a `TaskId` that is not in `state.tasks`. |
| `InvalidMemberRole { task_id, tier, role }` | A `Member`'s `Participant` tier does not allow its `Role` (`Normal` allows `solo`, `coordinator`, `member`; `Worker` allows `worker`; `Investigator` allows `coordinator`, `reviewer`). |

## Control socket protocol

### Wire framing and version

```text
PROTOCOL_VERSION  = 1
MAX_MESSAGE_BYTES = 4 MiB (4,194,304 bytes)
```

Every socket frame is one UTF-8 JSON object on a single line terminated by `\n`, with `"version": 1`.
`Request::from_json_line`, `Response::from_json_line` and `Event::from_json_line` check the byte limit, reject empty
lines and embedded newlines, and inspect `{ "version": u32 }` before deserializing the tagged payload. Any version
other than `1` is refused with `ProtocolError::UnsupportedVersion { found, supported: 1 }`.

`TargetId` addresses either a team (`team-YYYYMMDD-HHMMSS-xxxx`) or a task (`task-YYYYMMDD-HHMMSS-xxxx`) and serializes
as its string form.

### Requests (`"command": "<name>"`)

| Command | Variant | Fields |
|---|---|---|
| `run` | `Request::Run` | `repo: RepoName`, `topic: String`, `target: RunTarget` (`Solo { agent, prompt }` or `Team { coordinator, agents, merge_mode, task_list }`) |
| `status` | `Request::Status` | `filter: Option<TargetId>` |
| `show` | `Request::Show` | `target: TargetId` |
| `answer` | `Request::Answer` | `target: TargetId`, `round: Option<NonZeroU32>`, `payload: AnswerPayload` (`FileContent(String)` or `Verdicts(BTreeMap<NonZeroU32, Verdict>)`) |
| `stop` | `Request::Stop` | `target: TargetId`, `reason: Option<String>` |
| `pause` | `Request::Pause` | `target: TargetId`, `reason: Option<String>` |
| `unpause` | `Request::Unpause` | `target: TargetId` |
| `resume` | `Request::Resume` | `target: TargetId` |
| `say` | `Request::Say` | `target: TargetId`, `message: String` |
| `gate` | `Request::Gate` | `task_id: TaskId` |
| `subscribe` | `Request::Subscribe` | `filter: Option<TargetId>` |

### Responses (`"response": "<name>"`)

| Response | Sent for | Fields |
|---|---|---|
| `run_started` | `run` | `team_id: Option<TeamId>`, `tasks: Vec<TaskId>` |
| `status` | `status` | `state: State` |
| `show_team` | `show` (team) | `team: Box<Team>`, `members: Vec<Member>` |
| `show_task` | `show` (task) | `member: Box<Member>` |
| `answered` | `answer` | `task_id: TaskId`, `round: NonZeroU32`, `task_status: TaskStatus` |
| `stopped` | `stop` | `target: TargetId` |
| `paused` | `pause` | `target: TargetId`, `pause: PauseState` |
| `unpaused` | `unpause` | `target: TargetId` |
| `resumed` | `resume` | `target: TargetId` |
| `said` | `say` | `target: TargetId` |
| `gate` | `gate` | `task_id: TaskId`, `result: GateResult` |
| `subscribed` | `subscribe` | `snapshot: State` (followed by streamed `Event` lines on the same connection) |
| `error` | any failed request | `code: ErrorCode` (`invalid_request`, `not_found`, `invalid_transition`, `conflict`, `busy`, `gate_failed`, `internal`), `message: String` |

### Streamed events (`"event": "<name>"`)

After `Response::Subscribed`, the daemon streams `Event` JSON lines to the subscriber (`agent-web`):

- `team_created`, `team_status_changed`
- `task_created`, `task_status_changed`
- `item_status_changed`
- `run_started`, `run_finished`
- `questions_asked`, `questions_answered`
- `gate_recorded`, `pull_request_updated`, `review_recorded`
- `message_delivered`

## Questions files (`questions-<round>.md`)

When an agent ends a run with `STATUS: QUESTIONS`, its questions file is parsed, enriched with answering instructions
from a prompt template, and presented to the human (in the file on disk, through `agent answer`, or in `agent-web`).

### Structure

```markdown
# Questions — task-20260406-171000-k3x9

> Answering instructions from prompts/harness/... (rendered as a blockquote)

## 1. Which wire framing should the control socket use?
a) Newline-delimited JSON — one UTF-8 JSON object per line
b) Length-prefixed binary frames

**Recommended:** a) — human-readable and line-oriented (coordinator)

**Verdict:** ok

## 2. Where should the socket live?
a) Under $XDG_RUNTIME_DIR
b) Under $AGENTS_HOME/run

**Recommended:** a) — cleaned up by systemd

**Verdict:** b) — keeps every artifact under AGENTS_HOME

**ANSWERED**
```

- **Title (`# ...`)** — optional top-level heading before the first question.
- **Instructions (`> ...`)** — optional blockquote lines before the first question. `QuestionsFile::with_instructions_from_template(template, &[("task_id", ...)])` renders `{{key}}` placeholders and attaches the text; no instruction prose lives in `.rs` files.
- **Question (`## N. Title`)** — `N` is a positive integer (`1..=u32::MAX`) without leading zeros; numbers are unique within the file.
- **Options** — lettered lines in the body (`a) ...`, `- a) ...`, `* a. ...`, `(a) ...`) are extracted into `QuestionOption { letter, text }` for the dashboard while preserving the full `body` text.
- **Recommendation (`**Recommended:** <text>`)** — required once per question, non-empty; continuation lines before `**Verdict:**` are preserved.
- **Verdict (`**Verdict:** [text]`)** — required label once per question:
  - empty (`**Verdict:**`) while the file is still a draft (`AnswerStatus::Draft`);
  - `ok` (case-insensitive, trimmed) -> `Verdict::AcceptRecommendation`, which resolves to the question's recommendation text;
  - any other non-empty text (including multi-line continuation lines) -> `Verdict::Own(OwnAnswer)`.
- **`**ANSWERED**` marker** — on the last non-empty line of the file sets `AnswerStatus::Answered`.
  `QuestionsFile::answered_verdicts()` and `QuestionsFile::render_for_agent(lead_in)` require `AnswerStatus::Answered`
  and a verdict on every question, failing with `QuestionsError::NotMarkedAnswered` or
  `QuestionsError::MissingVerdict { number }`.

## Decisions to review

These are choices left open by the plan and fixed in `agent-core` for the checkpoint after feature 2:

1. **Arrays on wire, `BTreeMap` in memory.** In `state.json`, `State.teams`, `State.tasks` and `Team.items` serialize as
   arrays of records (each record already carries its `team_id`, `task_id` or `item_id`) and deserialize into
   `BTreeMap`s. This avoids duplicate keys between map keys and inner fields, keeps serialization order deterministic,
   and turns duplicate IDs in JSON into typed `DuplicateTeam`, `DuplicateTask` and `DuplicateItem` errors.
2. **Whole-second UTC timestamps (`YYYY-MM-DDTHH:MM:SSZ`).** `Timestamp` accepts only UTC in fixed 20-byte RFC 3339
   form, matching the UTC clock used by `TeamId` / `TaskId` / `RunId`.
3. **Lowercase hex `CommitSha` (7–64 chars).** Accepts both abbreviated (7+) and full SHA-1 (40) or SHA-256 (64)
   commits in lowercase hex.
4. **Single `TargetId` for team-or-task commands.** Because `TeamId` (`team-...`) and `TaskId` (`task-...`) have
   disjoint prefixes, commands that can target either (`show`, `answer`, `stop`, `pause`, `unpause`, `resume`, `say`,
   `subscribe`, `status`) take a `TargetId` instead of separate `--team` / `--task` flags.
5. **Two ways to submit answers in `Request::Answer`.** `AnswerPayload` supports both `FileContent(String)` (when the
   human edits `questions-<round>.md` in `$EDITOR`) and `Verdicts(BTreeMap<NonZeroU32, Verdict>)` (when answering
   through the web dashboard or CLI flags).
6. **`subscribe` sends an initial `State` snapshot before `Event` frames.** `Response::Subscribed { snapshot }` gives
   `agent-web` a race-free starting state on the same socket connection before incremental `Event` lines arrive.

## From the Bash harness

| Old (`bin/agent`) | New (`agent-core::state` / `protocol`) |
|---|---|
| `~/agents/tasks/<id>/meta.env` (sourced Bash variables per task) | `Member` inside `state.json` (`State::tasks`) |
| `bus/state.json` team fields + task directories | `Team` and `ItemProgress` inside `state.json` (`State::teams`) |
| Direct file edits and `systemctl --user` from CLI commands | `Request` / `Response` over the daemon's Unix socket |
| Polling `tasks/*/meta.env` and `activity.jsonl` in `live/` | `Request::Subscribe` -> `Response::Subscribed` + streamed `Event` lines |
| `questions_note`, `validate_answers`, `expand_answers` in `bin/agent` and `live/web_questions.go` | `QuestionsFile::parse`, `with_instructions_from_template`, `answered_verdicts`, `render_for_agent` |
