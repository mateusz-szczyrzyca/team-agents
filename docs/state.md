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

The Rust daemon no longer writes a task's `activity.jsonl`. Run steps live in `history.db` and are read through
`Reader::steps_of_task`; the state document remains the authority for current task and round state. Existing
activity files are left alone. See [history retention](history.md#retention-and-task-activity).

## State

A member may carry optional `holder_after_run`, the last run of the previous task holder. A handover preserves all
runs but resumes only sessions after this boundary and counts only those runs for the new holder's failure ladder.
Older states omit the field; they load without migration and acquire a boundary on their next handover. The
pre-renewal state fixture and handover reload tests exercise both formats.

### Recovery foundation

Version 0.76.0 adds optional top-level `recoveries` to schema 14. It is an ordered list of reservations:
`source` (the old team snapshot), `replacement` (the exact fresh team), `ordinal` (a positive restart count),
and `phase` (`pending` or `installed`). Absence means no prior reservations; an empty list is omitted on save.
The [pre-recovery fixture](../crates/agent-core/src/state/recovery/legacy.json) proves that an older documentation
team loads with an unused allowance. All existing state migrations remain supported. Older binaries reject a
nonempty ledger's unknown field; retain a backup before using a caller that writes recovery records. The daemon
in this release never creates a reservation, so ordinary daemon saves omit the new field.

Preparation fixes the replacement team/task identities and creation time before any external effects. Retrying
preparation returns the original plan even if the proposed destination or configured budget changes. The ordinal
starts at one and follows an installed predecessor; it survives removal of either team. A zero allowance,
a used-up allowance or an overflowing count creates no reservation. Source snapshots and plans remain in the
ledger so retries cannot reuse old identities or reset the budget. There is no ledger pruning in this release.

The replacement retains its repository, task list, merge policy, completed items, approved splits and merged
part rounds. Unmerged work returns to pending with no open round. A merged whole item whose old round was still
open keeps that evidence in a closed historical round. New crew identities keep their selected agents but start
with fresh sessions; old reviews, questions and runs remain only in the source snapshot. The loader validates
each replacement against that projection, plus fresh identities and ordered, installed restart predecessors.

Installation requires the source to be absent and the replacement identities to be free. It inserts the exact
saved team and changes the intent to `installed` together; repeated calls leave current progress untouched.
Once installed, an intent cannot recreate a replacement even after that replacement is removed. Preparation and
installation failures leave state unchanged. Callers must persist preparation before cleanup or external setup,
reconcile those operations after interruption, and persist installation before launching runs. This API performs
no disk, GitHub, worktree, bus or notification operations. The daemon remains removal-only until a separate
activation task wires those operations and tests their crash boundaries.

### Approved split definitions

Schema 13 adds optional per-team `splits`, a list of whole validated plans in approval order. Each plan has
`parent` (the original two-number item ID) and `parts`; each part has `title`, `delivers`, `checks` and `depends_on`.
The [verdict format](task-list.md#approved-split-plans-core-format) uses the same fields. A team's plans must name
its original items and cannot repeat a parent. Part keys are stable lowercase letters assigned by position;
`1.1a` is a separate part identity, not a newly accepted task-list ID. Plans cannot be replaced in place.

A missing `splits` field means no plans and an empty list is omitted when saving. Schema 12 migrates without
changing existing items, branch names, rounds, activities, runs or wake-up history. The pre-change schema-12 fixture
is [state-before-splits.json](../crates/agent-core/tests/fixtures/state-before-splits.json); its test compares the
whole saved document with the fixture after adjusting only the schema number. Versions 1–11 retain their previous
migrations and default to no split plans. Older binaries refuse schema 13, so keep a backup before upgrading.

Split execution adds an optional round `part`: `{"status":"working","key":"a"}` while active, or
`{"status":"merged","key":"a","pr":…}` after merge, with the ordinary pull-request record in `pr`.
Schema stays 13; states written by 0.29.0 omit this field and retain their meaning. The pre-execution fixture is
[state-v0-29-before-part-rounds.json](../crates/agent-core/tests/fixtures/state-v0-29-before-part-rounds.json).
Loading a stored definition alone activates nothing. An explicit coordinator answer records the whole plan and
binds its working round to part a. Later rounds retain the parent item ID and carry their distinct letter.
An intermediate merge records its pull request permanently and resets the parent to pending for the next
ready part; only the last merge completes the parent. Split rounds remain open on review approval in `--no-merge`.
Deserialization checks part membership, uniqueness and previously merged dependencies.

### Split proposal preparation

Schema remains 14. Approved parts may additionally retain `estimate` (`amount`, a JSON number, and `unit`, text)
and `tier` (`investigator`, `normal`, `worker`). Absent estimates remain absent; an absent tier means `normal`.
Default fields are omitted on saving, preserving the old split representation.

An approved split plan may also carry `approved_by`: `"harness"` or `"human"`. The optional field records the
approval authority without changing the parts or their requirements. Absence means the authority was not recorded;
reading and saving such a plan leaves the field absent. Existing plans load without a migration. The daemon
records `harness` for automatic approval and `human` for an explicit approving verdict. The pre-change plan fixture is
`crates/agent-core/tests/fixtures/split-plan-before-approval.json`.

Teams may have an optional `split_proposals` object, keyed by original item ID. Each entry stores bounded raw
`text`, the `corrections` count and `stage`: `"pending"`, `{"correction":{"fault":…}}`, or
`{"asked":{"question":N}}` with a nonzero questions number. Raw text deliberately includes unreadable proposals
so a restart before the decision loses no input. Resubmission preserves the correction count; a send-back counts
once and retains its fault; the asked stage retains the existing question. The daemon saves pending input before deciding. On restart it decides only pending entries; a saved correction
is included in the next coordinator prompt without incrementing its count, and a saved question is reused.

Only known, unsplit parents may have proposals. Recording or activating a complete approved split removes its
pending proposal atomically. Missing maps mean no proposals; empty maps are omitted. Existing state files and
human verdicts still load. The pre-change-format fixture is
[state-v0-36-before-proposals.json](../crates/agent-core/tests/fixtures/state-v0-36-before-proposals.json);
its test compares the complete saved document and checks legacy part requirements.
Older binaries reject the new fields, so retain a backup before saving new proposal or requirement data.

### Delivery claims

Schema 14 adds optional round `delivery`, an ordinary pull-request record identifying another round's PR in
that team. An absent marker preserves the previous round lifecycle. Schema 13 loads without changing its teams,
split plans, round parts, run references or history; the pre-change fixture is
[state-before-delivery.json](../crates/agent-core/tests/fixtures/state-before-delivery.json).
Older binaries cannot read schema 14; keep a state backup before upgrading.

Only an open round of an `in_progress` item may receive this marker. Acceptance preserves the item's phase;
completion records `done` with the delivering PR and closes the round as `merged`. Both transitions are checked,
and a refusal leaves the team unchanged. Split-part completion preserves the existing rule: intermediate parts
retain their PR and leave the parent pending until its final part merges. The marker survives completion and restart.
Loading verifies that its complete PR record belongs to another round of the same team and agrees with the
waiting or completed item. Git evidence and remote merge observation are the daemon's responsibility.

### The file

The state is one JSON document, written indented with a newline at the end, the schema version first:

```json
{
  "schema_version": 15,
  "teams": [ { "id": "team-20261111-193000-a1b2", "...": "..." } ]
}
```

A state of version 2, as `main` wrote it before the findings, is
[`state-before-findings.json`](../crates/agent-core/tests/fixtures/state-before-findings.json) (and
[`state-v2.json`](../crates/agent-core/tests/fixtures/state-v2.json)): three teams — one of three roles and two
reviewers with a merged round and an open one, one solo that does not merge by itself with an approved round, one on a
list without items — with the runs that name the rounds. A test loads it; **it must keep loading** — a later build
that cannot read it would lose the teams of an installation. A state of version 3 with findings is written and read
back by the tests of the `findings` module. The previous daemon's state is also kept in
[`state-before-independent-reviews.json`](../crates/agent-core/tests/fixtures/state-before-independent-reviews.json).
Version 3 migrates without enabling independent reviews: its active PRs keep two reviewers, the bus and immediate
publication. New PRs freeze the configured `review_count` in their ledger.
Version 4, before rotating verification, is preserved in
[`state-before-rotating-verification.json`](../crates/agent-core/tests/fixtures/state-before-rotating-verification.json).
Its active PRs keep the same reviewers, full-review requests and single-pass renewal. New PRs add optional
`verification` context to their findings ledger: `reviewed_head`, `lead_message` and `extra_fix_passes`.
This preserves the previous reviewed commit, the latest fix message and the human's renewed allowance across restarts.
Version 5, before one-time explanations, is preserved in
[`state-before-disputes.json`](../crates/agent-core/tests/fixtures/state-before-disputes.json).
Its active ledgers have no `disputes` marker and retain their previous lead and verifier protocol. New ledgers
retain each explanation's reason, pending/accepted/rejected status and verifier votes, the accepted commit and
changed-file eligibility, upward severity decisions, the answered fix-pass number, and the checked run, correction
text and bounce count. These fields preserve decisions and bounded corrections across daemon restarts.

Version 6, before second opinions, is preserved in
[`state-before-second-opinions.json`](../crates/agent-core/tests/fixtures/state-before-second-opinions.json).
Migration to version 7 preserves the active round and its previous protocol markers; an absent `opinions`
map means no finding has used an opinion.
Version 8 retains accepted FIXED answers by pass and one-time human-notice claims alongside opinions.
Version 8 before bounded assignments is preserved in
[`state-before-opinion-retries.json`](../crates/agent-core/tests/fixtures/state-before-opinion-retries.json).
Migration to version 9 retains active fix passes, their protocol markers, FIXED answers and completed opinions.
The migrated round uses the current opinion protocol without incrementing its fix-pass count.

The optional top-level `wake_deliveries` map (added in 0.17.0, schema version unchanged) keeps the wake-up budget of each
task that the bus woke for an unread message, by task ID. Each value is `{"state": "pending", "message": 12, "count": 1}` (the
number of wake-ups spent for the newest unread message, with its number on the bus; `message` is null when no message
could be named) or `{"state": "delivered", "message": 12, "count": 2}` (`limits.wake_ups_per_message` reached: the task is
not woken for that message again). A state without
the map loads as before and an empty map is not written; a daemon older than 0.17.0 refuses a state that holds it
(`deny_unknown_fields`), so going back after a wake-up stop needs the field removed by hand. The loader of the new field is
exercised by `crates/agent-core/tests/wake_delivery.rs` over
[`state-before-wake-delivery.json`](../crates/agent-core/tests/fixtures/state-before-wake-delivery.json).

The findings ledger's optional `opinions` map uses permanent finding numbers as keys. Each value retains
`request` (`pass`, exact `head`, nonempty `reason`) and a tagged `stage`: `waiting`, `assigned`
(`investigator`, `task`), or `reported` (the same owner and a `report` with `outcome` and nonempty `evidence`).
Only an open BLOCKER can request once. Pending tasks can be replaced; only the assigned task may report,
once. Confirmation leaves the finding open; non-confirmation closes it as `not_confirmed` without changing
its number. The loader rejects unknown/non-blocker opinion owners and inconsistent statuses. A confirmed
finding can later become `resolved`; a not-confirmed finding retains its report and stays closed.


The optional `opinion_attempts` map uses the same finding numbers as `opinions`. Each value is a nonempty
list of at most three assignments, oldest first, with `investigator`, `task`, `asked_at` (RFC 3339 or null for
an assignment migrated from version 8), and `failure` (null while active or after a successful report).
Failure values are `failed`, `no_status`, `missing_report`, `silent`, `interrupted`. Investigators and tasks
are distinct within a request; every earlier assignment has failed. A waiting request has fewer than three
failed attempts; an assigned request owns the last attempt; a reported request's last attempt succeeded.
After the third failure the assigned stage remains for the human notice and rejects late reports.
Absent attempt history means no retained assignments, not a fabricated count: a legacy assigned task is
recorded as its first attempt if it fails. Only its owning task may end an assignment; stale reports cannot
close a replacement. Notice claims persist to avoid duplicate notifications after restart.


[`state-v1.json`](../crates/agent-core/tests/fixtures/state-v1.json) is a state of version 1, which had no rounds. It
**must keep loading**: it is read through the migration (below) as exactly
[`state-v1-as-v3.json`](../crates/agent-core/tests/fixtures/state-v1-as-v3.json), with its schema advanced and reconstructed investigator activities, every time.

### Investigator activities

Schema 11 adds each team's optional `activities` list (omitted when empty). Each entry has `investigator`,
`task`, `rounds`, `request`, `asked_at` and `outcome`. Request `kind` values are `team_member`, `first_review`,
`verification` (with one-based `pass`), `second_opinion` (with `pass` and permanent `finding` number),
`investigation` and `task_allocation`. These map to activity kinds `team-member`, `reviewer`, `investigator`
and `task-allocator`; a task's role remains separate. No new investigation/allocation workflow starts yet.

An outcome has `status: working`, or `status: finished` with `at` and `end`. End `kind` is `reported`,
`stopped`, `failed` (with the last structured run `reason`, or null if unavailable), or `opinion_failed`
(with the opinion attempt's structured failure `reason`). All times are RFC 3339. A finish cannot precede
assignment and an entry cannot repeat a round. Accepted reports are retained across later stops or failures.
Shared PR assignments merge their round references rather than counting once per item.

Reading schemas 1–10 rebuilds records from investigator crew membership, each item's review and fix-pass
history, and retained opinion attempts. Review assignment and submission timestamps remain exact. Opinion
attempts without assignment times use their first task run, or team creation if none exists. An opinion's
ending time uses its last ended run, or the team's latest retained run timestamp, bounded below by assignment.
History discarded before this schema cannot be recovered. Current task permissions and all other legacy data
remain intact. The previous-version fixture is
[`state-before-activities.json`](../crates/agent-core/tests/fixtures/state-before-activities.json), written by
main before this change with an open second fix pass and an assigned second opinion. Socket protocol and
status output are unchanged; builds that write schema 10 cannot read a schema-11 state after it is saved.

### Unpublished base commits

Schema 12 adds each team's optional `unpushed_base` evidence: the base `branch`, nonzero `count`, full
`first` commit ID and its `subject`. It is recorded only when `agent run --allow-unpushed-base` permits a
start with commits absent from origin. The harness retains and prints this evidence in every pull request.
Teams without the field grant no permission and have no evidence to report. Schema 11 migrates without
changing retained investigator activities; its fixture is
[`state-before-unpushed-base.json`](../crates/agent-core/tests/fixtures/state-before-unpushed-base.json).
Older daemons cannot read a state saved with schema 12.

### Reading and its rules

- **Strict.** A field that is not on this page is an error, in every object (`deny_unknown_fields`); so is a missing
  one, except the optional ones that are marked. A typo does not fall back to a default.
- **Typed.** Every value is parsed into its type when the file is read: IDs by their form, times as RFC 3339, enums by
  their texts. A value its type refuses is an error at its line and column.
- **Checked.** What the types cannot say, reading checks too (the invariants below). A state that is read is as valid
  as one that was built through the API. There is one way to read a value, the `Deserialize` trait: the field-by-field
  reading that serde derives lives on a private mirror type (`remote`), so no public `T::deserialize` skips the checks.
- **Versioned.** `State::from_json` reads `schema_version` **first**. A state of another version is refused with
  `StateError::UnsupportedSchema { found }` — never as "malformed", whatever else it holds. This build writes version 15,
  migrates versions 1 through 14 as it reads them (see "Version 1" below); any other version is
  refused. Version 2 is read as it is: its items have no `findings`, so a round that was open when the daemon was
  updated ends by the rules it started with (verdicts only), and the numbered findings hold for pull requests opened
  after the update. A change that old files do not satisfy raises the version and comes with a migration, a fixture of the old
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

`idle_watch` is optional in schema 14. When present it holds `last_activity` (a timestamp), `heads` (local branch
and worktree keys mapped to commit IDs), and nullable `refusal` (the already reported removal failure). Old
states omit it and still load. Their first scan reconstructs activity from team creation, run starts/ends and
commit dates; subsequent new or changed tips count at observation time even when the commit has an old date.
New activity clears the refusal marker. A restart preserves activity and notification suppression, but removal
counts idle time from the later of recorded activity and current daemon startup. Downtime does not count; the
startup lower bound stays in memory and is not written as activity.
The pre-change fixture is `crates/agent-core/src/state/team/idle/legacy.json`. Older binaries reject the added
field; keep a pre-upgrade state backup if a downgrade is needed.


| Field | Type | Meaning |
|---|---|---|
| `id` | `team-YYYYMMDD-HHMMSS-xxxx` | generated from the clock and the random source |
| `repo` | repository name | the `[repos.NAME]` of the configuration the team works on |
| `created_at` | time | when the team was started |
| `merge` | `autopilot` \| `no_merge` | what happens to an approved pull request (`--no-merge`) |
| `task_list` | text | the task list **as given**; reading parses it again, so a list that no longer parses is an error. Its optional header line `Short desc:` (0.51.0) is part of the text, so the description is kept with the team and survives a restart without a new schema number; a team without the line has one derived from its brief wherever one is shown. A text this version wrote **with** such a line is stored as `{"text": …, "allow_lines": "permissions", "short_desc_header": true}`; every other stored text — every text written before 0.51.0 — is read as it always was, a `Short desc:` line being brief text, so an old brief that starts with such a line, and the `Allow:` lines around it, keep their meaning ([`state-before-short-desc.json`](../crates/agent-core/tests/fixtures/state-before-short-desc.json)). A marked text that the header rules refuse is a fault of the state. An older binary reads the marked form as a plain text and takes the line for brief |
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
| `plan` | what the task keeps of its plan ([`plan.md`](plan.md#the-daemon-and-the-plan)); **optional**: absent on a member written before 0.35.0, whose runs are never sent back for their plan, and present on every task made since (`plan_judging`: optional boolean on `Member`, `true` on tasks created under step judging and omitted on tasks created between 0.35.0 and `#163`, which are never sent back for a step check mismatch). `sent_back` (times its runs went back for the plan), `granted` (send-backs the human added), `stage` (`"open"`, `{"asked": {"question": N}}` while the human's questions number N about the plan wait, `"released"` once the human said to go on as it is), and, absent when none: `pending` (the reason of a send-back the next run is told: `{"kind": "missing"}`, `{"kind": "unreadable", "line": N, "detail": "…"}`, `{"kind": "unfinished", "steps": ["1.2"]}`, `{"kind": "lost_step", "steps": ["1.3"]}`, `{"kind": "mismatch", "step": "1.1", "title": "…", "check": "…", "reason": "…", "tail": "…"}`, `{"kind": "fault_and_mismatch", "fault": <PlanFault>, "mismatch": <StepMismatch>}`), `refused_gate` (list of `["<step>", "<command>"]` notices waiting to be told once; also kept as an optional `refused_gate` field on `Member` itself when `plan` is absent), and `unread_answers` (the number of the answered questions about the plan the next run is given). The schema's number does not change: an older state loads, every member without a `plan`; 0.34.0 and older refuse a state with the field, and 0.65.0 and older refuse a state with `fault_and_mismatch`, `refused_gate` or `plan_judging` (a member's fields and `PlanReason` variants are strict), so going back needs the state of before the upgrade |
| `running_check` | the finished run whose command checks or plan settlement are in progress ([`plan.md`](plan.md#judging-done-steps)); **optional**, absent when none is: `{"run": "run-…", "end": <RunEnd>, "pause": <Pause>, "made_progress": true|false, "final_message": "…", "ended_at": <time>}` (`pause`, `final_message` and `ended_at` omitted when absent), kept on the member itself so both current and legacy tasks settle the finished run once across a restart with its original end time and progress without re-running the CLI |
| `renewals` | how the ladder started the task in a new session ([`[replacement]`](configuration.md#replacement)); **optional**: absent until it did, and on every member of a state written before 0.38.0. `used` (how many new sessions), `at` (when the last one was decided), `reason` (`"no_status"`, `"hang"`, `"turn_limit"`, `"busy"`, `"no_tool_call"`, `"daemon_down"` or `"context_bound"`), `after_runs` (how many runs the task had then: the runs after them are the new session's, and no session before them is resumed). A hand-over to another agent, and the question to the human after the last step, mark the same boundary without raising `used`. `context_tokens` (**optional**, absent unless the reason is `"context_bound"`) is the size in tokens of the context that outgrew the agent's bound and ended the session: the size paragraph of the new session's first prompt is built from it, so that a restart before that run still names it. `asked` (**optional**, absent until then) is the number of the questions the human was asked what becomes of the task after the last step of the ladder; it is asked once. `lead_owed` (**optional**, absent unless owed) is when a member's task stopped with its work given to its lead, until the bus took the notice to the lead: a daemon that restarts with it tells the lead then. `given_to_lead` (**optional**, absent unless true) records that the ladder gave the member's work to its lead; it stays after the lead is told, so that the team's look at a stopped member's work skips only such a member, not one that was merely started in a new session before the human stopped it. `busy_notice_owed` (**optional**, absent unless owed) marks that a busy hand-over's team notice was not accepted for every affected item: the whole team is sent item, branch, task and both holders through the bus outbox, after recovery and after transient delivery failures. A permanent bus refusal or a rendering failure is logged and drops that notice; the obligation is cleared when all notices have either succeeded or been dropped. `busy_notice_owed` is retained independently of a newer hand-over. A renewal before the task's first run, or counting more runs than the task has, is refused. The one exception is a busy hand-over to another agent before the task's first run: the new holder still needs the briefing, so the boundary is recorded with `after_runs` zero and a new session waits. The schema's number does not change: an older state loads; 0.37.0 and older refuse a state with the field, and 0.38.0 to 0.43.0 one with `asked` or `lead_owed` (the fields of `renewals` are strict), so going back needs the state of before the upgrade |
| `waiting_since` | **optional**: the moment a queued task began to wait for its agent, which runs another task at its limit (`[limits] runs_per_agent`). Absent when the task does not wait, and from a state written before waits existed; a run of the task, a hand-over, pausing or stopping end it. Resuming starts a fresh wait interval. The schema's number does not change: an older state loads; a version that does not know the field refuses a state that has it |
| `past_holders` | the agents that held the task before the one that holds it now, oldest first; **optional**: absent until it was handed over, and on every member of a state written before hand-overs existed. Each entry is `{"agent": <participant>, "until": <time>}`: the agent the task was taken from, and when it was given to the next one. The agent that holds it now stays in `agent`. The schema's number does not change: an older state loads; a version that does not know the field refuses a state that has it |

Publication escalation may move a `waiting` or `failed` task directly to `questions`. It creates no run and
records no intermediate `queued` or `running` status. Older state and history remain readable: the persisted
status names are unchanged, and history/log readers store status changes without rejecting these pairs.
`publication_history_compat.rs` loads a pre-change history database, preserves its old log and reads both new
pairs; `grouped_publish_state.rs` loads the former flat publication question identity. New publication questions
store one optional identity with task, number, timestamp and branch; old complete identities retain their task
and number, while incomplete timestamp-only markers supply no fabricated identity.

A `Run` is `{id, started_at, session?, rounds?, used_tool?, state}`; `rounds` are the rounds it works for (below), oldest first, absent =
none; its state is `running` or `ended` with a time and **how**:

| `end.kind` | Fields | Meaning |
|---|---|---|
| `reported` | `outcome` | the agent's final message ended with `STATUS: DONE`, `QUESTIONS`, `FAILED`, `SLEEPING` or `EXCLUDED` |
| `no_status` | `exit_code?` | the CLI ended without a `STATUS:` line (no code: a signal) |
| `quota_exhausted` | `resets_at?` | the quota is used up; the time the CLI gave, if it gave one |
| `rate_limited` | | the provider limited the rate of requests |
| `turn_limit` | | the CLI exhausted its per-run turn budget; queued for bounded same-session continuation |
| `hung` | | no activity for too long; the watchdog stopped it |
| `stopped` | | the human stopped it |

The `session` is the model CLI's session ID, which a later run resumes (the latest run that has one).

`used_tool` records whether the run called a tool step before it reported `STATUS: FAILED` (absent = false); the no-tool
retry is only the retry of a run that ended without one. Schema 15 adds it; a schema-14 state loads through `from_v14`
with every run read as having called no tool step, and the pre-change fixture is
[state-before-no-tool-call.json](../crates/agent-core/tests/fixtures/state-before-no-tool-call.json).

A `Question` is `{number, file, asked_at, state}`: the questions file `questions-<number>.md` of the task, and
`open` or `answered` with the time and the channel (`command` = `agent answer`, `saved_file` = the file was saved with
the `**ANSWERED**` marker, `dashboard`). **The questions and the answers are in the file, not in the state.**

#### Items

An item's progress is `{id, phase, fix_passes?, gate?, findings?, merge_passes?, conflict_verifications?, conflict_approved_head?, conflict_refunded_head?, conflict_accounted_head?, conflict_provisional_head?, merge_conflict?}`. `merge_conflict` is
`{status: "merging_base", approved_head, at}` while the lead merges the base after a refused merge, or `{status: "asking_human", at}` once the human is to be asked; it is absent otherwise, and an approved item without it is merged as before. `merge_passes` counts the lead passes
spent on a merge that GitHub refused for a conflict, compared with `[merge] conflict_passes`; it is absent while none is spent,
so states written before it read as none spent and keep the schema version. `conflict_verifications` counts how many times an approved item was reopened for a verification review after a conflict pass or an answered conflict question (absent while zero), `conflict_approved_head` keeps the approved head GitHub refused while a conflict pass or question is active (absent when none), `conflict_refunded_head` keeps the head whose provisional conflict pass was refunded on publication failure (absent when none), `conflict_accounted_head` keeps the head whose conflict pass was accounted as spent upon confirmed publication (absent when none), and `conflict_provisional_head` tracks the head for which a provisional conflict pass was reserved (absent when none or unreserved). The `phase` carries exactly the data of that phase:

An item's optional `rejections` lists the returns of its lead, oldest first, each `{at, reason, recorded_in?, part?, of_run?}`: when the lead's
run ended with `RETURNED: <item>`, what the lead told the member directly during the run that returned the work as the reason, the number
of the member's run that was started again after it (`recorded_in`, absent until that run starts), the part of a split item
the work belonged to (`part`, a letter) and the run of the member that ended `DONE` and was sent back (`of_run`); the last
two are absent in states written before and for an item that is not split. Only the returns with a `recorded_in` count
against `[replacement] rejections`. The field is absent while the lead returned nothing, so a state written before it
loads with a count of zero and the schema version stays 14 (the existing `state-before-renewals.json` fixture is read by
the tests of the count).

| `phase` | Data | Item status | Moves to |
|---|---|---|---|
| `pending` | | `pending` | `in_progress` |
| `in_progress` | `assignee`, `branch` | `in_progress` | `pending` (the member left), `in_review` |
| `in_review` | `pr`, `reviews[]` | `in_review` | `fixing` (changes requested), `approved` |
| `fixing` | `assignee`, `branch`, `pr` | `in_progress` | `in_review` |
| `approved` | `pr`, `reviews[]` | `approved` | `done` |
| `done` | `completion`: `listed` (the list marked it `[x]`) or `merged` (`pr`, `at`) | `done` | — |

**Findings.** An item of a pull request opened by this build carries `findings`: the permanent record of the review's
findings, shared by the items of one pull request. Each entry has a `number` (given by the harness, counting up over the
pull request in the order the reviews were asked and then the order of the report; it never changes), the `severity`
(`BLOCKER`, `MAJOR` or `MINOR`), the `rule` ID, the `place`, the `text`, the `check`, the `reporter` (the investigator),
the review `pass` and the `status` (`open` or `resolved`). `comment` is the id of the harness's one comment on the pull
request, with the list of the findings, which is edited in place; reports that wait for the rest of their pass are
kept there until the pass is complete. A finding is resolved when every reviewer of a later pass says so. The numbers are exactly 1, 2, … in order; a state whose ledger has a duplicate, a gap or another order is refused when read. An item
without `findings` belongs to a round opened by an earlier daemon. The optional `review_count` is at least two:
when present it freezes the number of reviews per pass and enables independent first-pass reviews. When absent the
PR retains the preceding protocol. Pending first-pass reports survive daemon restarts and wait for all verdicts
before publication.

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

A round is `{id, item?, opened_at, state, part?, delivery?, delivery_closed_at?}`:

| Field | Meaning |
|---|---|
| `id` | `round-YYYYMMDD-HHMMSS-xxxx`, generated like the other IDs; unique in the whole state |
| `item` | the item the round is for; absent for the whole work of a team whose task list has no items |
| `delivery` | optional PR record belonging to another round of this team; retained while waiting, after completion and after an observed unmerged closure |
| `delivery_closed_at` | optional timestamp of an observed unmerged close; requires a delivery and an abandoned round; absent in 0.34.0, whose open claims still load unchanged |
| `opened_at` | when the daemon opened it: when the item was first given to a member, or when the team started |
| `state` | `{"status":"open"}`, or `{"status":"closed","at":…,"how":…}` with `how` one of `merged`, `approved` (`--no-merge`: both reviews approved, the pull request is the human's) and `abandoned` (the team was stopped or ended without a merge or an approval) |

A run names the rounds it works for (`rounds`, above). Invariants, checked by the operations (`Team::open_round`, `close_round`,
`advance_item`, `begin_run`, and `State::open_round`, `close_round`, `begin_run` for what needs the whole state) and again when the
state is read:

- an ordinary item has at most one round excluding abandoned rounds with an ended delivery claim; an activated split has one per started part, and a merged part or an ended claim permits its next round;
  an item has a round as soon as it leaves `pending`; an item the task list marks `[x]` has none, and a
  round is not opened for an item that is done;
- a round without an item exists only in a team whose task list has no items, and there is one at most;
- a round ID is used once in the whole state;
- a closed round is never opened again: closing it again is refused, and so is a second ordinary round for its item unless the old round has an ended delivery claim. An ordinary round closed as
  `merged` belongs to an item `done` as merged; a merged part retains its own pull request while its parent advances; one closed as `approved` to an item that is `approved` or `done` as merged;
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
  a `clean` tree is a pass of `head`. The previous record, in the same format, is retained in optional
  `tasks/<task>/gate-previous.json` before replacing the latest record. This lets descriptions disclose a pass
  after a failure at the same commit. An older task directory without it reads as before.
- **The try of a request** — optional `tasks/<worker task>/worker-try-<n>.json`, `{"number": 2, "earlier": ["why the first result did not serve"]}`, written when a request is given to a worker for the second time and removed when the request is answered; a request without it is on its first try. An unknown field is refused. The settlement `again` of a request's file (`{"settled": "again", "reasons": [...]}`) keeps the first worker's file until the bus has the request with another worker.
- **The request of a worker whose run has ended** — optional `tasks/<worker task>/worker-request-<n>.json`, written by the
  daemon when a worker's run ends `STATUS: DONE` and replaced, through a temporary file, after each stage of settling its
  result: `{"request": n, "team", "requester", "requester_task", "worker", "worker_task", "summary", "run_seconds", "base",
  "stage": {"stage": "ended" | "checked" | "settled", ...}}`. `checked` holds what the request asked, the worker's commit, the
  files outside the request's `files` and what the check found; `settled` adds what became of the result (`report`, `brought`
  with the commit on the requester's branch, `conflict` with the files, `returned` with the reasons). The daemon removes the
  file when the requester is told. A daemon that starts finds the files of the team's workers and goes on from their
  stages; an unknown field is refused, and a file that is refused, cut short or cannot be read is set aside as
  `worker-request-<n>.json.damaged` (kept, never read again), logged (`worker_request_damaged`, with its name) and its request is
  answered as a wrong result, while the other files are read. `base` is the commit the worker's worktree was at when the request was given to it. A task directory without such files reads as before.
- **Lead contribution to the PR description** — optional `tasks/<lead>/description.md`, outside the repository,
  read as UTF-8 with a 1 MiB limit. The harness combines it with its own header, measured Size, recorded Gate
  and TEST-FIX disclosure. Missing or empty text does not block a round. It is not part of `state.json`.
- **The checks before a review** — `tasks/<lead>/checks.json`, the **checks record** of a team's lead, written by the daemon only:
  `{"items": {"<item id>": {"passes": N, "allowed": [<subject>, ...]}}, "asked": {...}, "sent": {...}}`. For each item the passes the
  checks sent its round back and what the human allowed (a subject is `{"path": "tests/a.rs"}`, `"no_tests"` or, since 0.54.0, a commit — see `refused`). Five optional
  keys, left out when empty and absent from a record an earlier daemon wrote (it loads, and its allowances hold as before):
  `refused`, the subjects the human refused for the item, kept until it is closed (a subject can be `{"commit": {"commit": "<id>", "path": "tests/a.rs"}}`, a commit that took lines out of a frozen test, kept by the commit and the path; one in `allowed` is never revoked, because a commit never changes; a record without `refused` loads as it did);
  `human_only`, the subjects of `allowed` that the human allowed in answer to a question about the issues they alone may allow,
  which does not cover an issue of the team's about the same file; `covered_at`, a map from a file the human allowed or refused before commits had subjects to the head up to which that answer covers its commits (a commit added after it is asked on its own);
  `allowed_at`, a map from a file the human allowed to the head
  the checks looked at then — the allowance ends when the round changes that file again, and the file is asked about again; and,
  beside `asked`, `basis` (`{"head": ..., "covers": "everything" | "human_part"}`), what the question waited on was put about: a
  question about the human's part alone, put while the team still has passes, leaves the count of passes as it is when answered.
  `asked` is the
  question the round waits on (the `number` of the lead's questions, when it was written, the items, what each question is about, how
  often the lead was woken to put it); `sent` is what the lead is to be told on its next run. Empty parts are left out and an empty
  record is `{}`. A task the checks have not seen has no file. See "Round checks before the review" in
  [`architecture.md`](architecture.md).
- **Worktree and branch paths, the unit names** — derived from the IDs and the layout of `AGENTS_HOME`.
- **An idle round's next action** — derived from its durable phase, questions and opinions together with the owner's
  queued gates and publication corrections. The optional `RoundDetail.waiting` field is absent from older socket
  replies and omitted for active or closed rounds; it changes neither `state.json` nor its schema. The publication
  correction budget and one-time scheduling-error log set are transient and restart with the daemon.
- **A team's overall status** — derived from its members and items (`TeamSummary`).
- **Tokens and cost** — out of scope of the plan.

## Control protocol

### Framing

JSON lines over the daemon's Unix socket: one message per line, ended by `\n`, no embedded newline (JSON escapes it).
A client sends a **request**; the daemon answers with a **response** carrying the request's `id`. A client that sent
`subscribe` also receives **events** on the same connection. The daemon's lines are told apart by `type`.

| Direction | Line | Limit |
|---|---|---|
| client → daemon | `{"protocol":3,"id":N,"client":{…},"request":{"command":"…",…}}` | 2 MiB (a task list is at most 1 MiB) |
| daemon → client | `{"type":"response","protocol":3,"id":N,"result":{…}}` | 32 MiB |
| daemon → client | `{"type":"event","protocol":3,"at":"…","event":{"kind":"…",…}}` | 32 MiB |

The `id` is the client's number for the request, any `u64`, echoed so that a client can match answers. The `client`
says who asks — `{"kind":"cli"}` or `{"kind":"dashboard","login":"you@example.com"}` — and the daemon names it in the
activity of a control (who paused, who answered).

### Versions

Legacy APIs and clients use `"protocol":3`. Split-aware clients send `"protocol":4`; the daemon answers in the
request's version and retains it for subscription events. Version 4 adds split acknowledgements (`Response.split`),
plans in team summaries (`TeamSummary.splits`) and part identities in rounds (`Round.part`). All are omitted from
version-3 replies and events. A version-3 `show` of a full split team returns an `invalid` refusal explaining that
protocol 4 is required: removing part identities would make historical rounds invalid. Ordinary version-3 frames
retain their existing encoding. State schema 13 is unchanged.

Version 5 adds `task_plans` to status snapshots. Versions 3 and 4 omit this field.
Version 6 adds `part_assignments` and `tier_waits`; versions 3, 4 and 5 omit both fields from replies and events,
so strict older clients remain compatible. Status and show first probe version 6, then fall back to versions 5,
4 and 3 when a daemon refuses or closes a probe. The selected format is cached on the connection. Status uses
cached summaries; it performs no filesystem or history reads.

The CLI negotiates before sending other split-aware commands: it sends a read-only version-4 `status` probe, then sends
the intended command in version 4. If an older daemon rejects the probe or closes the connection, it reconnects
once and uses version 3 instead; no mutating command is retried. The choice is cached on that connection. Older
clients continue to send version 3 directly.

A reader checks the version before decoding the message body. Strict legacy `Request::from_line` and
`Frame::from_line` retain version-3-only behavior; their negotiated counterparts accept 3, 4 and 5. Other versions
return `ProtocolError::UnsupportedVersion`. A line without a usable version, or invalid JSON, is `Malformed`;
a line over the direction's limit is `TooLong`. A message change that old peers cannot read raises the version.

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
| `remove` | `team`, `force?` (absent means false) | `removed` (`team`, `stopped`, `tasks`, `bundle?`, `bundled`, `branches`, `closed_pull_requests`, `dirty_worktrees`, `freed_bytes`, `left_behind`) |
| `gate` | `task` | `gate_recorded` |
| `subscribe` | `team?` | `subscribed`, then events |

A `target` is `{"team": "team-…"}`, `{"task": "task-…"}` or `{"round": "round-…"}`; the controls (`stop`, `pause`, `unpause`,
`resume`, `say`) take a team or a task and refuse a round with `invalid`. A `verdict` is a string: `ok` (any case) accepts the
recommendation, any other text is the human's own answer. The daemon checks `answers` against the questions file of the questions
with that `number` (every question answered, `ok` only where there is a recommendation).

The `remove` command adds no persisted team fields. It saves state without the team's tasks, rounds and wake-up
budgets before deleting local files. A failed save leaves the in-memory team and local files intact. Reloading the
saved state cannot recreate a removed team; its history remains in `history.db`. The reply's `dirty_worktrees`
names tracked changes regardless of cleanup success; `left_behind` reports cleanup failures and the bus records
not released by this command. `freed_bytes` counts only successfully removed directories.

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
| `teams` | `teams`: summaries — `id`, `repo`, `created_at`, `merge`, `items` (counts by status), `rounds` (the team's rounds, as in the state; absent = none), `members` (task, agent, role, status, `open_question?`: the number of the questions that wait, `new_sessions?`: how many new sessions the ladder started the task in, `held_by?`, `returned?`, `waits_for?`: `{team, task}`, the run that holds the agent of a queued task at its limit of runs; absent when none) |
| `overview` | what `status` answers instead of `teams` when the history ([`history.md`](history.md)) lost records or cannot be written: `teams`, as above, and `history` — `state` (`{"kind":"writing"}`, `{"kind":"disabled"}` or `{"kind":"unavailable","reason":"…"}`) and `lost`, the number of records handed to the history that are not in it. A history that is written and whole, or off, is not mentioned: the answer is `teams` |
| `team` | `team`: the whole team record, as in the state |
| `round` | `round`: `team`, `round` (as in the state), `item?` (`id`, `title`, `status`), `tasks` (`task`, `role`, `agent`, `worktree`, `branch`, `runs`: the runs that name the round), `pr?` and `waiting?` (the idle round's next action) and `review_passes` (each fix pass, then the pass that is going: `reviews`; the reviews of the pass that was approved are not kept once the item is done) |
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

The lines of version 3 are pinned by [`protocol-v3-requests.jsonl`](../crates/agent-core/tests/fixtures/protocol-v3-requests.jsonl)
and [`protocol-v3-frames.jsonl`](../crates/agent-core/tests/fixtures/protocol-v3-frames.jsonl), and the lines about rounds by
[`protocol-v3-round-requests.jsonl`](../crates/agent-core/tests/fixtures/protocol-v3-round-requests.jsonl) and
[`protocol-v3-round-frames.jsonl`](../crates/agent-core/tests/fixtures/protocol-v3-round-frames.jsonl): a test reads each line
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
8. **`RunEnd` lists what ends a run** (reported status, no status, quota, rate limit, turn limit, hung, stopped). Item 3.1
   classifies a final message and an exit status into it. Adding a variant without changing existing variants or fields keeps
   old state files valid under the current `schema_version` ("Reading and its rules" above: `schema_version` rises only for a
   change that old files do not satisfy), while raising the protocol version and migrating the history schema.
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
16. **A round is a record of its team, found by the item.** It is `{id, item?, opened_at, state}` with optional split and delivery metadata; an item has one current round, and a run
    names the rounds it works for (a list, because a coordinator works for every round that is open). The ID of a team and the ID
    of a round are the two keys the history of item 8.4 is found by.
17. **A closed round is never opened again, also when its team is resumed.** A `stop` that leaves a team with nobody to work
    abandons its open rounds; a `resume` goes on with the item in its round, which stays `abandoned`, and the runs that go on name
    it. Such an item can merge in a round that says `abandoned`. An observed unmerged closure of a delivery PR is
    different: it ends that claim permanently and permits a fresh round for the same item or split part. The old
    round stays abandoned with its delivery and closure timestamp.
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

### Turn-limit compatibility (item 8.8)

State schema `2` is unchanged: old state files still satisfy the contract, including runs without a turn-limit
ending. No counter field is added. The count is the consecutive suffix of `turn_limit` endings of the task;
any other ending resets it. A queued task with such a last end resumes once in that run's session after restart.
Older binaries cannot read a state containing the new end and must not be used to downgrade a running installation.

Protocol `3` adds the `turn_limit` end in run events and status details. Both peers must use version `3`;
version `1` and `2` fixtures are retained unchanged and are rejected before their payload is parsed. This follows
the protocol rule for messages old peers cannot read. History schema `7` migrates the closed keyword constraints
for run endings and reasons while preserving existing rows and references. A newer history is refused untouched.

An observed delivery closure is saved on the abandoned round before recovery. While that round remains the latest
for its in-progress item, the daemon still owes the member a fresh round. After opening it and queuing the member,
that new round makes recovery complete. Restarting at either boundary preserves the obligation and never revives
the ended claim, even if GitHub reopens its PR. The marker timestamp cannot precede the opening of the round.

### Capability waits

Teams and status summaries may contain `tier_waits`, an object mapping original item IDs to minimum tier
keywords. Each key must name an item in the team’s task list. Empty maps are omitted; older records without this field load with no retained waits. The fixture
`crates/agent-core/tests/fixtures/team-before-tier-waits.json` preserves a team written before this field.
The owner recomputes waits from ready pending items and review fixes awaiting a qualified assignee: only
absence of qualified crew is retained, not temporary busyness. The map and assignment changes are saved together. A newly entered wait is logged once; restarting
with the same wait does not announce it again. A split uses the original item ID while its active round keeps
the part identity. No schema migration is needed for this optional field.

### Plan-step status snapshots

Step judgements live in history schema 17, not `state.json`; state schema 14 is unchanged.

### Worker requests in the bus state

A request for a worker is not kept in `state.json`: the bus keeps it in its own state, under `helpers`. Next to `step` a
request the harness accepted for a step carries `files` (the paths the request names, as a list), `check` (the command or
`exists PATH` that shows the result right) and `doing` (what to do, in the requester's words). The three are absent on a
request in the old form, or one written before this version — the harness will read them as none and treat such a request
as a look-up — so a bus state of an earlier version loads unchanged (`agentbus/testdata/state-before-worker-result.json`).
The daemon caches plan-step summaries for task and round status and rebuilds them on startup.
A summary includes the mark, step ID and title, plus a done step's settlement and optional reason.
The optional `task_plans` map in team and round replies is absent when there is no readable plan.

### Split executors and capability questions

Teams optionally store `part_assignments`, an object mapping split round IDs to agent names. Activation and the
executor's run record the assignment; later coordinator integration or review does not replace it. This leaves
the round representation unchanged and keeps the historical executor available after an item changes phase.
Missing legacy assignments are displayed as `not recorded`, without guessing an executor from current progress.
The optional map also appears in protocol-6 team status replies and events, alongside `tier_waits`. Older
protocols omit both maps and the private `tier_questions` marker in full team replies; newer clients can fall
back to older daemons without executor metadata.

The optional `tier_questions` object maps original item IDs to the tiers already asked about in the current
`tier_waits`. The question and markers are saved together. Answering queues the lead through the usual answer
flow and leaves the marker intact, so a restart or an unhelpful answer cannot generate another question for the
same wait. Refreshing waits removes markers once staffing resolves them. A later wait can then ask again.
Older teams omit both maps and load with empty maps; the pre-change fixture
`crates/agent-core/tests/fixtures/team-before-tier-waits.json` remains readable. No schema migration is needed.

### Per-agent run limits

The run limit (`[limits] runs_per_agent` and `runs_per_worker`, default `1`) keeps state schema `14` unchanged. When
queued tasks enter the queue in an order that differs from team-creation order (for example, when an older team's task
is resumed while a newer team's task is already waiting), `state.json` carries the optional top-level `queued` array of
task IDs in queue-entry order; when the order matches team-creation order, `queued` is omitted. A state written before
the limit with an agent that has runs going in two teams loads unchanged: both tasks finish, and the limit holds from
the next start. On the socket, `MemberSummary` carries the optional `waits_for` (`{team, task}`) when a queued task
waits for its agent's run in another task, omitted when absent.
