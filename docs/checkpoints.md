# Checkpoints — how work survives a lost session

This is a guide to the harness's checkpoints: what they are, when the daemon takes one, what one holds, who reads it
and what to do with them by hand. It is written for someone who wants to understand how the harness carries work
across a crash, a restart or a change of agent, before changing that code or diagnosing a team that "started over".
It describes `main` at agent 0.144.0 · agent-core 0.141.0 · agent-runtime 0.130.0 (PR #346, 27.11.2026).

The daemon as a whole is described in [`docs/daemon-overview.md`](daemon-overview.md), the history in
[`docs/history.md`](history.md), the keys in [`docs/configuration.md`](configuration.md) and the log events in
[`docs/log.md`](log.md). File references below are to the code, relative to the repository root; they were right at
the time of writing. Section 13 lists the places where other docs and the code disagree.

---

## 1. What a checkpoint is

An agent's run is a conversation with a model, and everything the model knows about its work lives in that
conversation. When the conversation ends before the work does, that knowledge is gone: the daemon was restarted, the
machine went down, the run hung and was stopped, its context grew too large, its provider refused it, or the work
passed to another agent. Without help, the next session would read the task from the start, look at a worktree full
of half-finished changes and either do the work again or undo it.

A **checkpoint** is the harness's answer. It is a short, written account of where one piece of work stands, taken by
the daemon at moments when something happened, together with a copy of the worktree at that moment. It has three
parts:

- **What is to be done** — the task, the item and its numbered requirements (`R1`, `R2`, …).
- **What is done** — the commits and plan steps, requirement by requirement, the findings answered, the review passes.
- **What is left** — the steps and findings still open, and one `next:` line that says what to do now.

A checkpoint looks like this (`agent checkpoints --show` prints it the same way, and so does a briefing):

```text
## Checkpoint: commit at 2026-11-27T04:00:00Z, head 0123abc

### What is to be done
Task: Build the thing.
Item 1.3: Say what is meant
R1. the first thing works

### What is done
Totals: 1 commits, 1 steps
R1 met — steps 1.1 (confirmed); commits abc1234

### What is left
Totals: 0 steps, 0 findings
next: commit what is finished, run the gate and hand the round over
```

Whoever goes on with the work — a new session of the same agent, another agent, a new reviewer, a second worker, the
same agent after a daemon restart — gets the **last checkpoint** at the top of its first prompt, with the words "Go on
from it: do what is left, do not do again what is done."

Three rules shape everything below:

1. **The daemon writes checkpoints, never the agent.** Models are not trusted to follow a protocol (see
   `docs/daemon-overview.md`, section 1), so the account is built from what the harness can see: git, the history,
   the plan file `.agent/plan.md` and the review findings. An agent keeps its own notes in `.agent/plan.md`; the daemon
   reads them into the checkpoint.
2. **A checkpoint never stops work.** When one cannot be taken, the daemon logs `checkpoint_failed` and the round goes
   on (`crates/agent/src/daemon/owner/checkpoint.rs:1-6`).
3. **Every takeover starts from the last one** — the owner's decision D20 (26.11.2026): a change of agent, session or
   team keeps the branch, the pull request and its review, and goes on from the last checkpoint, never from scratch.

## 2. Two things with one name

| | Daemon checkpoints (this guide) | The bus tool `checkpoint` (legacy) |
|---|---|---|
| Written by | the daemon | an agent, through the MCP tool `checkpoint` |
| Stored | `history.db`, record kind `checkpoint`, plus a git snapshot | `<bus dir>/checkpoints/<team-…|task-…>.md` |
| Read by | briefings, `agent checkpoints` | nobody in the daemon |
| Status | the live mechanism | still offered to every run; its retirement is planned |

Section 12 describes the bus tool. Everywhere else, "checkpoint" means the daemon's.

## 3. What a checkpoint holds

### The record

A checkpoint is one record of kind `checkpoint` in the history's `history_records` table, a JSON payload with
`"event": "checkpoint"` (`HistoryEvent::Checkpoint`, `crates/agent-core/src/history/event.rs:614-653`; migration
`crates/agent-runtime/src/history/034-checkpoints.sql`). Its fields:

| Field | What it is |
|---|---|
| `round` | the round the work belongs to |
| `item` | the item of the round (absent for a job checkpoint) |
| `task` | the holder: the task whose worktree the snapshot keeps |
| `job` | `item` (the default), `worker_request`, `review` or `investigation` |
| `tier` | the holder's tier |
| `cause` | why it was taken (section 4) |
| `pull_request`, `draft`, `head` | the round's pull request, whether it is a draft, the worktree's `HEAD` |
| `phase` | where the round stands (in progress, in review, …) |
| `review_pass`, `fix_pass` | the review and fix pass numbers, when there are any |
| `snapshot` | the git reference of the worktree copy, absent when the snapshot failed |
| `content` | `{ meant, done, left }` — the three parts; absent when they could not be built |

Optional fields are left out when empty. Checkpoints are never de-duplicated as history events: every one is kept
(`event.rs:741`). They are not in `state.json`.

### The snapshot

With each checkpoint the daemon copies the holder's worktree into a commit under

```text
refs/agents/snapshots/<round>/<task>/c<n>
```

in the repository (`crates/agent-runtime/src/git/snapshot.rs:20-26`). The copy holds everything a `git status` would
show — modified, staged, deleted, untracked and binary files; ignored files are left out — on top of `HEAD` as its
parent. It is built in a scratch index, so the worktree, its index and its branch are not touched (`:147-157`). The
commit's author is `team-agents <team-agents@team-agents.invalid>` (`crates/agent-runtime/src/snapshot.rs:12-16`).

`<n>` is one more than the highest checkpoint number of that task, so numbers are never reused (`snapshot.rs:192-193`,
`:352-355`). When the worktree has not changed since the last checkpoint, the new checkpoint names the same reference
instead of taking another copy.

The `c` sets these apart from the older **cut-run snapshots**, `refs/agents/snapshots/<round>/<task>/<n>` with a plain
number, which the daemon takes on start for every run that did not end and for a briefing whose worktree changed
(README.long.md, "Snapshots"). Both kinds are restored the same way (section 7).

### The content

For a round's item (`checkpoint_text`, `crates/agent/src/daemon/owner/checkpoint.rs:586-667`; the pure part in
`crates/agent-core/src/history/checkpoint/state.rs`):

- **meant** — `Task: …`, `Item <id>: <title>`, and one `R<n>. …` line per requirement of the item.
- **done** — `Totals: N commits, M steps`, then per requirement which plan steps and commits serve it
  (`R1 met — steps 1.1 (confirmed); commits abc1234`). A step's word is one of `confirmed`, `declared`, `mismatch`,
  `in hand`, `not done`; a commit is tied to a step by its `Step:` trailer. Then the findings answered and the review
  passes.
- **left** — `Totals: N steps, M findings`, the steps not done, the findings still open, and a `next:` line:

| `next:` | When |
|---|---|
| `commit what is finished, run the gate and hand the round over` | the work is not handed over yet |
| `wait for the reviews of pass N` | the pull request is in review |
| `fix the N open findings in pull request #N and answer each one` | a fix pass |
| `wait for the human's instruction; read the human's messages` | a question to the human is open (always after `question_asked` and `waits_for_human`) |
| `wait for the merge` | approved |
| `nothing; the pull request is merged` | merged |

It is built from the branch's commits against `origin/<base>`, the latest judgements of the steps, the plan file
`.agent/plan.md` and the review findings. Each list stops at 40 lines and ends with "N more left out"
(`state.rs:22`). When any source cannot be read, `content` is left out and the failure is logged.

The content of a job checkpoint is described in section 4.4.

## 4. When the daemon takes one

### 4.1 At every step of a round

Every history event of a round goes through one function (`checkpoints_after`, `checkpoint.rs:239-252`, called from
`crates/agent/src/daemon/owner/history.rs:70`). When the event marks a step, a checkpoint of that cause is queued
(`HistoryEvent::checkpoint_cause`, `crates/agent-core/src/history/checkpoint.rs:159-193`):

| Cause | Taken when |
|---|---|
| `round_opened` | a round opens, before its first run can touch the worktree |
| `commit` | the holder's `HEAD` moved (see 4.2) |
| `pull_opened`, `pull_pushed`, `pull_ready` | the pull request is opened, pushed, marked ready |
| `checked` | the checks before the review judged the round |
| `split_decided` | a split of the item was decided |
| `review_submitted` | a reviewer submitted its verdict |
| `fix_pass_started` | a fix pass begins |
| `finding_answered` | the team explained, accepted, rejected or raised a finding |
| `finding_judged` | a finding was confirmed or not confirmed |
| `question_asked`, `question_answered` | a question to the human was asked or answered |
| `waits_for_human` | the round stops for the human (publication or a delivered part) |
| `pull_approved`, `pull_merged`, `pull_handed_over`, `pull_closed` | the pull request's end states |
| `merge_refused` | a merge was refused for a conflict |
| `round_overdue` | the round passed `[limits] round_minutes` |
| `run_ended` | a run of the round ended |

The gate, a review being asked, a second opinion, a Serena reminder, a notification and the round's own close leave
no checkpoint.

### 4.2 After every local commit

Once a minute (`LOOK_SECONDS`, `checkpoint.rs:35`) the daemon looks at the `HEAD` of every task with a watched run.
When it differs from the head of the task's last checkpoint, it queues one `commit` checkpoint (`look_for_commits`,
`checkpoint.rs:875-940`). So a commit that is never pushed still leaves a checkpoint, and several commits within one
minute leave one. The first look at a task only notes its head. A failed look is logged `git_call_failed` with the
operation `checkpoint_look`.

### 4.3 When the daemon starts, and when the holder changes

- **`daemon_started`** — when the daemon starts (`recover`, `crates/agent/src/daemon/owner.rs:952`), right after
  the cut runs' snapshots, it takes one for every task of every open round, and for every going job (4.4). A task
  whose worktree has not changed since its last checkpoint gets none.
- **`holder_changed`** — when the work passes to a new session or a new agent: `agent renew`
  (`controls.rs:381`), the lead taking over a member's task (`takeover.rs:132`), a new agent's own session after a
  hand-over (`takeover.rs:277`), a renewal because the context grew past `max_context` (`bound.rs:123`, `:136`), a
  task given to a new holder on assignment (`teams.rs:1037`), and a new session after bad runs (`owner.rs:2938`).

A `holder_changed` checkpoint marks the change; a briefing does not tell it (section 5), because it says nothing new
about the work.

### 4.4 Checkpoints of workers, reviewers and investigators

A pool worker's request, a review and an investigation are **jobs**: they work in their own task and worktree, inside
a round that belongs to someone else. Since PR #338 they keep checkpoints too, with the same record and the same three
parts (`crates/agent/src/daemon/owner/job_checkpoint.rs`; content in
`crates/agent-core/src/history/checkpoint/job.rs`). `job` says which, and `item` is absent.

| Job | Causes | meant | done | left |
|---|---|---|---|---|
| `worker_request` | `request_started`, `commit`, `request_checked`, `run_ended` | `R1. <the request>`, `K1. <its check>` | the commits, the changed files, `Check: passed — …`, `failed — …` or `not run yet` | `next: report the result` / `make the check pass and commit` / `do what the request asks and commit` |
| `review` | `review_started`, `findings_written`, `commit`, `review_submitted`, `run_ended` | `R1. Review pull request #N at head H, pass P, …` plus the item's requirements | `Findings written so far: N` and the lines of the findings file | `next: finish the review of pass P and submit it with a verdict` |
| `investigation` | `investigation_started`, `commit`, `investigation_answered`, `run_ended` | `R1. Answer the question: …` | the commits and changed files | `next: answer the question` |

A reviewer's findings file is `review-pr<n>-pass<p>-findings.md` in its task directory
(`crates/agent-runtime/src/store.rs:205-216`); the minute look records `findings_written` when it changed and is not
empty.

A job ends with `run_ended`, `review_submitted`, `investigation_answered` or `holder_changed`. When a job passes to a
new holder — a request back in the queue for another worker, a replaced reviewer, a second try of an investigation —
the daemon copies the old holder's last checkpoint as a `holder_changed` checkpoint of the new task
(`job_handed_over`, `job_checkpoint.rs:367-427`). The new holder is briefed from it: the commits are in its worktree,
the findings so far are a new reviewer's leads, and the review pass keeps its number.

### 4.5 How a checkpoint is taken

1. A cause **queues** a checkpoint (`queue_checkpoint`, `checkpoint.rs:256`).
2. At the top of its next loop turn the daemon **flushes** the queue (`flush_checkpoints`, `:447`; `owner.rs:884`).
   It also flushes before a briefing, a publication and a merge, so that they read the newest state.
3. For each: the **snapshot** first, then the **record**, then a short wait for the history writer — at most 2 s, or
   20 ms once the writer has been seen stalled (`take_checkpoint`, `:507-580`).

The round of a checkpoint is the round of the current run, else of the task's assigned item, else of its last run,
else the team's only open round (`checkpoint_round_of`, `:728`). A task with no round gets none.

## 5. How checkpoints are used: the briefing

A new session of a task that was already started does not get the task's prompt alone. Its first prompt is a
**briefing** (`prompts/harness/run/briefing.md`, "# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED"): why there is a
new session, who worked on the task and how each run ended, the commits, the files left uncommitted, the snapshot, the
plan, the answers of the human, unread messages, the pull request and its review — and, near the top, **the last
checkpoint**.

### Who is briefed

- `agent renew <task>` — the human asks for a fresh session.
- A session cut by a daemon stop or crash, or a machine restart (`[restart] session = "new"`, reason `daemon_down`).
- A session that hung, hit its turn limit, ended three times without a status, or failed.
- An agent replaced because it was busy elsewhere or its provider refused it.
- A round cut by `[limits] round_minutes` and given to another agent.
- A session renewed because its context grew past `max_context`.
- A lead that takes over a member's work: the member's checkpoint comes in its own section
  (`prompts/harness/run/lead-briefing-checkpoint.md`, "## Where the member's work stands").
- A worker, reviewer or investigator that takes over a job (4.4), with its own short text:

| Text | Opens with |
|---|---|
| `prompts/harness/run/briefing-worker.md` | "# THIS REQUEST WAS STARTED BEFORE YOU" |
| `prompts/harness/run/briefing-review.md` | "# THIS REVIEW WAS STARTED BEFORE YOU" |
| `prompts/harness/run/briefing-investigation.md` | "# THIS INVESTIGATION WAS TRIED BEFORE YOU" |

Every one of them renders the checkpoint with `prompts/harness/run/checkpoint-state.md` (variables `cause`, `time`,
`head`, `meant`, `done`, `left`), the same text `agent checkpoints --show` prints.

### Which checkpoint is told

The briefing tells the **last checkpoint that says something about the work** (`last_told_checkpoint_of_round`,
`crates/agent-runtime/src/history/reader.rs:770`): the task's own newest item checkpoint in the round, else the
round's newest. `holder_changed` checkpoints and `run_ended` checkpoints that changed nothing are skipped. A job
briefing reads the job's own last checkpoint (`job_briefing`, `job_checkpoint.rs:816-881`).

### Which snapshot is named

The briefing names one snapshot for the worktree (`crates/agent/src/daemon/owner/briefing.rs:457-487`):

1. after a daemon stop or crash, the snapshot taken of the cut run;
2. otherwise the last checkpoint's snapshot, when the worktree still matches it (same head and tree);
3. otherwise a new snapshot.

### When there is none

- No checkpoint yet: the briefing says "No checkpoint yet." and goes on with the rest
  (`crates/agent-runtime/src/run/compose/briefing.rs:236`).
- A checkpoint without content: "(this checkpoint kept no account of the work)"
  (`crates/agent-runtime/src/run/compose/checkpoint.rs:23`).
- A task with no round is not briefed; `history_record_incomplete` is logged.

## 6. A restart of the daemon

What happens to the work when the daemon stops or is killed and starts again:

1. **On start**, before any run: a cut-run snapshot of every task whose run did not end, then a `daemon_started`
   checkpoint of every task of every open round and of every going job (4.3).
2. Every going job is registered again from its checkpoints (`jobs_daemon_started`, `job_checkpoint.rs:327`): its
   start commit, what it was meant to do and the last check result. A job whose checkpoints lack a start head or a
   meant part is logged `checkpoint_failed`.
3. **The next run** of each cut task is, by default, a new session briefed with the reason `daemon_down`
   (`[restart] session = "new"`); a cut review, worker request or investigation is briefed from its own last
   checkpoint.

Nothing of the worktree is lost: what the run had not committed is in the snapshot and still in the worktree.

## 7. Snapshots by hand

List a task's snapshots:

```sh
git for-each-ref refs/agents/snapshots/<round>/<task>/
```

Restore one into a worktree (the files of the snapshot over the current ones; nothing is deleted):

```sh
git checkout refs/agents/snapshots/<round>/<task>/c7 -- .
```

**Pruning.** The daemon removes a round's `c<n>` references when its pull request is merged or closed, or when the
round closes with any end other than approved (`event_prunes`, `checkpoint.rs:196-204`; `prune_round`, `:670-722`).
The history records stay. The cut-run snapshots (plain `<n>`) are never removed by the daemon; remove one with
`git update-ref -d <ref>`. Once no reference names a snapshot, `git gc` can free its space.

## 8. Reading them

**`agent checkpoints <round-…|task-…> [--show [N]] [--json]`** (`crates/agent/src/commands/checkpoints.rs`) reads
`history.db` directly; it needs no running daemon.

- Without options: one line per checkpoint, oldest first —

  ```text
  <at>  <cause>  PR <#N draft|#N ready|none>  head <abc1234|none>  pass <n>  done <C commits, S steps|n/a>  left <S steps, F findings|n/a>  snapshot <ref|none>[  job <job>]
  ```

- `--show N` prints checkpoint `N` in full (1 is the oldest); `--show` alone prints the last.
- `--json` prints the records; it cannot be combined with `--show`.
- A task's target lists the checkpoints of its team's rounds that name it.
- Nothing found: "No checkpoints for X." and exit 0 (with `--json`, the notice goes to stderr and stdout is empty).
  Exit 3 for an unknown round or task, 1 for other errors (for example `--show` with a number past the last).

**`agent log`** does not print checkpoints in the told story of a round (`crates/agent-core/src/story/text.rs:1148-1150`).

**The history itself**:

```sh
sqlite3 "$AGENTS_HOME/history.db" \
  "select at, json_extract(payload,'$.cause'), json_extract(payload,'$.task') from history_records
   where kind = 'checkpoint' and json_extract(payload,'$.round') = '<round>' order by at"
```

## 9. Configuration

No key turns checkpoints on or off; they are always taken while the history is enabled. These keys decide when they
are taken or used:

| Key | Default | What it does here |
|---|---|---|
| `[snapshot] warn_mb` | `200` | a snapshot above this size is still taken and logged `snapshot_large` |
| `[restart] session` | `"new"` | after a restart the cut task goes on in a new session briefed from its checkpoint |
| `[session] max_context` (also `agents.NAME.max_context`) | `150000` | a session past it is renewed (`holder_changed`) and briefed |
| `[replacement] new_sessions` | `1` | how many new, briefed sessions bad runs get before another agent takes over |
| `[limits] round_minutes` | `0` (off) | a round past it leaves `round_overdue` and passes to another agent |
| `[commit_reminder] context_percent` | `80` | a session near its context is told to commit; its commits become `commit` checkpoints |

The intervals in code are fixed: the look for commits every 60 s, the wait for the history writer 2 s, lists of 40
lines.

## 10. Failures and edge cases

- **Any failure is logged, never fatal.** `checkpoint_failed` (warn; `round`, `task`, `cause`, `error`) covers a
  failed snapshot (the record is written without `head` and `snapshot`), content that could not be built (written
  without `content`), a stalled history writer (`cause = history_stalled`, logged once, then not waited on until it
  answers), pruning (`prune_list`, `prune_no_worktree`, `prune_remove`) and a job briefing that could not read its
  checkpoint (`briefing_read`).
- **The worktree is gone.** The checkpoint is recorded without a snapshot.
- **Large worktrees.** A snapshot is never refused for its size; above `[snapshot] warn_mb` it is logged
  `snapshot_large`.
- **No change.** An unchanged worktree reuses the last snapshot; an unchanged `daemon_started` is not taken at all.
- **A run with no round** (for example a coordinator between items) leaves no checkpoint.

## 11. Compatibility

- The record kind and the `checkpoint_failed` log event came with history migration 034; older history files migrate
  and keep their records (fixture `crates/agent-runtime/tests/fixtures/history-before-checkpoints.sql`).
- A record without `job` reads as an item checkpoint, so the job checkpoints of PR #338 needed no migration.
- A record without `content` (from before content existed, or one whose content failed) is told with the "kept no
  account" line.
- An unknown `cause` or an unknown field of `content` is refused when read: adding a cause is a change of the format,
  with tests that old records still load (AGENTS.md, rule 9).

## 12. The bus tool `checkpoint` (legacy)

Every run's `agentbus` still offers an MCP tool `checkpoint` (`summary`, optional `next_steps`). Its description asks
the agent to save a checkpoint "after each completed stage and about every 30 min of work" and says the team resumes
"from the coordinator's last checkpoint" (`agentbus/collab.go:1188-1191`). The tool appends the text to
`<bus dir>/checkpoints/team-<id>.md` (a coordinator) or `task-<id>.md` (anyone else), and a non-member's call also
posts a `CHECKPOINT:` message to everyone (`collab.go:1398-1456`). `agentbus checkpoint-get --dir --key` prints a file.

**The daemon reads none of it**: no code outside the tests calls `checkpoint-get`
(`crates/agent-runtime/src/agentbus.rs:1611`), and briefings are built only from the daemon's checkpoints. A note
saved with the tool reaches nobody. The planned change (`task-checkpoint-tool-retired.md`, decision D4) keeps the tool
callable but has it answer that the checkpoint is the plan file `.agent/plan.md`, and takes it out of the agents'
texts. Until then, what an agent wants the next session to know belongs in `.agent/plan.md` and in commits.

## 13. Where other docs disagree with the code

1. `docs/configuration.md` (`[snapshot]`) says nothing prunes snapshots. The daemon prunes the `c<n>` references
   (section 7); the cut-run snapshots are indeed never pruned.
2. `docs/configuration.md` (`[restart] session`) says a cut review or worker request starts again as before. Since
   PR #338 each is briefed from its own last checkpoint (section 6).
3. `docs/architecture.md` says a resumed agent's checkpoint is not read from the bus yet, and the compose order lists
   a "checkpoint" section. That section (`prompts/harness/run/checkpoint.md`) is never filled
   (`owner/start.rs:443`); the checkpoint reaches a run through the briefing.
4. `docs/daemon-overview.md` (section 13) lists `checkpoint-get` among the subcommands the daemon calls; it has no
   caller outside the tests.
5. `README.long.md` names the history schema 34 at its checkpoints paragraph; that is the migration that brought
   checkpoints. The newest migration is 036.

## 14. Where to read more

- `README.long.md`: "Snapshots", "Checkpoints", "Checkpoints of workers, reviewers and investigators",
  `agent checkpoints`, the briefing, the MCP tool `checkpoint`.
- `docs/history.md`: the `checkpoint` record and its jobs.
- `docs/log.md`: `checkpoint_failed`, `snapshot_large`.
- `docs/daemon-overview.md`, section 10: restart and recovery.
- Code: `crates/agent/src/daemon/owner/checkpoint.rs` (item checkpoints), `job_checkpoint.rs` (jobs),
  `briefing.rs` (briefings); `crates/agent-core/src/history/checkpoint.rs` and `checkpoint/` (the record, the content);
  `crates/agent-runtime/src/git/snapshot.rs` (snapshots); `crates/agent/src/commands/checkpoints.rs` (the command).
