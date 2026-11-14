# Investigation — round `rust-agent/8.5` — `inv-kimi-deepinfra`

Base: [`869901c0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/869901c028fa7f7104d5637e12928acf63edc88f) (`origin/main`). Branch `agents/inv-kimi-deepinfra/20261114-144134`. Phase 1, independently; no PR was opened.

This is an implementation investigation, not an implementation. No production or test code was changed. Serena was used for symbol discovery and targeted bodies; shell reads were used for documentation, schema text and test fixtures.

## Executive findings

1. **The current schema cannot satisfy same-time write ordering by itself.** [`crates/agent-runtime/src/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-runtime/src/history/schema.rs) has independent keys/rowids per table and no cross-table sequence. Two records in different tables with the same timestamp have no recoverable insertion order.
2. **Use migration 3 to add one logical-record ledger plus a round-link table.** Keep the existing normalized tables and readers. The ledger gives every logical [`Record`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-core/src/history.rs#L781-L800) one sequence, time, team and ordered round links. Existing record kinds refer to their normalized rows; new 8.5 kinds store typed JSON in the ledger. This bounds the schema work while giving `--json` exactly one line per record and a direct SQL count.
3. **A completely unavailable database makes the literal loss-reporting requirement impossible to guarantee.** Today a loss updates only an atomic aggregate and an in-memory reported-cause set in [`crates/agent-runtime/src/history/handle.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-runtime/src/history/handle.rs#L111-L127). Nothing survives for a direct `Reader`, let alone after a restart. Persist best-effort loss tombstones when storage recovers, and print explicit “missing; no record was stored / cause unknown” at an absence. Do not claim knowledge of losses that occurred while no storage was available. A stronger requirement needs a human-approved separate loss journal, which still cannot guarantee writes during disk-full corruption.
4. **The frozen schema test needs human authorization.** Any new table fails `the_pinned_file_is_the_version_of_the_last_migration_and_every_table_of_it_is_strict`, whose table list is hard-coded in [`crates/agent-runtime/tests/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-runtime/tests/history/schema.rs#L31-L65). Repository rule 1 requires an approved dedicated `TEST-FIX:` for that list and [`docs/history.schema.sql`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/docs/history.schema.sql), as round 8.4 needed for `run_sessions`.
5. **Recording must happen at causal external actions, not polling reads.** In particular, record pushes after successful Git pushes, PR lifecycle after its GitHub call, review requests after `record_reviews`, and fix passes only at transitions into `Fixing`. Recording in `deliver_fix_passes`, `try_merge` polling, or `deliver_reviews` without a natural idempotence key would duplicate events.
6. **The CLI must not open the socket.** Extend [`crates/agent/src/main.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent/src/main.rs) with a separate `LogArgs`/`Work::Log` path, as was done for `gate`, rather than modifying the frozen `Command` contract. Read `history.db` directly, inject `Write`, and map unknown round/team IDs to a distinct tested exit code (recommend `3`; usage remains `2`, operational/read/output failures `1`).
7. **JSON must remain lossless when presentation flags are combined.** Recommended semantics: `--prompts` and `--no-steps` affect human text only; `--json` always emits the complete selected record set. JSON prompts and bodies are stored strings already, so there is no useful JSON equivalent of prompt preview or step elision.
8. **Plan the PR stack before coding.** Records + ledger + pure story + CLI + all daemon hooks + documentation are likely near the `4,000` non-test-line planning threshold. Use the staged TDD sequence below and measure after each vertical slice. No dependency is needed beyond those already approved/present.

## Evidence from the current implementation

- [`Record`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-core/src/history.rs#L781-L800) currently covers team/round openings, run start/launch/steps/end/changes and sessions. A logical record already spans multiple physical rows (`TeamStart` + plan, `RunChanges` + commits/files), so `--json | wc -l` must count **logical records**, not SQLite rows.
- [`crates/agent-runtime/src/history/write.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-runtime/src/history/write.rs) inserts parent records `ON CONFLICT DO NOTHING`, validates child uniqueness, and preserves the first copy. New records need equivalent natural idempotence keys; append-only events such as a control or actual notification need a fresh event key rather than accidental deduplication.
- [`Reader::open`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-runtime/src/history/reader.rs#L180-L209) is read-only and accepts older supported schemas. A version-1/2 story reader must synthesize a deterministic legacy sequence because it cannot create the new ledger read-only.
- [`Stored<T>`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-runtime/src/history/reader.rs#L36-L43) carries `Cut` separately. Text and JSON stories must expose those cut markers; `--prompts` can only mean the whole **stored** prompt, not text truncated at the configured history limit.
- Existing secret handling allows redacted stored text to differ from CLI input, explicitly documented in [`docs/history.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/docs/history.md#L455-L471). Golden prompt-equality tests must use scenarios without secret markers. The secret test separately requires redaction, not byte equality with the fake CLI.
- [`RunChanges`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-core/src/history.rs#L764-L779) has no capture timestamp. New writes need an additive `History::record_at`/captured-at path in the run engine; the legacy reader/backfill may use its run-end time (or run-start when no end exists), clearly documented as an approximation. Do not add a required field to `RunChanges`, which would break frozen struct literals.
- [`GateRecord`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-core/src/state/gate.rs#L68-L103) already has head, finish time, outcome, tree and log. Gate duration needs the request time captured in `gate_request` and an execution start captured in `Service::execute`; measure the total request-to-finish duration with the injected clock (the execution interval may be recorded additionally), so queue wait is not lost. Represent a backward clock as an explicit capture loss, not a fabricated non-negative duration.
- Existing fake CLI scenarios commonly write `gate.json` directly, as [`crates/agent/tests/fixtures/team-cli.sh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent/tests/fixtures/team-cli.sh#L98-L108) shows. Those are not daemon gate executions. Golden tests must instead issue `Command::Gate` through the socket after an ungated fake commit/integration and before publication can proceed.
- The existing [`Rig`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent/src/daemon/owner/rounds/tests/rig.rs), fake GitHub, prompt capture and argument capture are reusable. New test-only actions such as `integrate_ungated` may be added to the stub; existing actions and helper assertions must remain unchanged.

## Proposed record and storage model

### One sequence per logical record

Add migration 3 with two STRICT tables (names illustrative, not a required API):

- `history_records`: `sequence INTEGER PRIMARY KEY`, `kind`, `natural_key`, `at`, `team`, optional `payload` plus `cut`, and a uniqueness rule over `(kind, natural_key)`.
- `history_record_rounds`: `(sequence, round)` with an index from `round` to `sequence`.

`payload`/`cut` are `NULL` for pre-8.5 record kinds, whose values continue to come from the validated normalized readers. They are populated for new 8.5 kinds using typed, tagged JSON serde. SQL checks that known kinds have the correct payload-nullness and JSON validity; the reader again deserializes into closed enums and validates cross-record rules. All values are bound parameters, never SQL interpolation.

Migration 3 backfills index rows and round links for existing logical records; it does not duplicate their large texts. Backfill timestamps for changes use the nearest existing run times. For a database opened read-only at version 1/2, build equivalent entries from normalized rows with a deterministic virtual sequence.

**Compatibility caveat to document and test:** original cross-table order at equal old timestamps was never stored. Migration/backfill chooses a documented deterministic legacy order; only records written after migration 3 guarantee actual same-time insertion order. Never claim that migration reconstructed information that did not exist.

The writer inserts each normalized record and its ledger entry atomically. Queue order determines `sequence`; a failed batch followed by per-record retry preserves the accepted records' sequence order without exposing rolled-back gaps as data. Run-linked records inherit their run's rounds through the existing `run_rounds` mapping. A round selector joins through the link table, while team selection includes all linked records plus the one team-start record.

For counts, an independent test query counts distinct parent ledger sequences selected by the round plus its team-start record. It must not sum child rows or repeat a coordinator run linked to multiple rounds.

### New typed record kinds

Use `strum` for every new keyword text. Keep pass numbering explicit in docs: stored review `pass = 0` means the first review pass; human text displays `1`. Human fix passes and check-back passes start at `1`.

| Record | Required content |
|---|---|
| `Gate` | request time, execution start, finish, team, task, existing `GateRecord`, total request-to-finish duration (or explicit unavailable duration) |
| `CheckAssessment` | time, team, PR/items, head, every structured `Issue`, current human-allowed subjects, and one outcome: reviews started, sent back `{pass, of, text}`, asked human `{question number, text}` |
| `PullRequestOpened` | time, team/items, number, branch, actual remote head, whole title/body |
| `PullRequestPushed` | time, team/items, PR, new head; natural idempotence by PR/head |
| `PullRequestReady` | time, PR, head |
| `ReviewRequested` | time, PR, pass, head, reviewer, reviewer task |
| `ReviewSubmitted` | time, PR, pass, reviewer/task, verdict, whole report |
| `FixPassStarted` | time, PR, `{pass, of}`, whole findings sent to the team |
| `PullRequestApproved` | time, PR/head (second approval, for both merge modes) |
| `PullRequestMerged` | time, PR, merge commit, whether observed from the merge response or a later remote read |
| `PullRequestHandedToHuman` | time, PR; specifically the `--no-merge` approval |
| `PullRequestClosed` | time, PR and observed head where available; unmerged closure only |
| `QuestionAsked` | time, team/task/number, whole questions text as asked |
| `QuestionAnswered` | time, team/task/number, whole applied questions text (every verdict whole), `AnsweredVia` |
| `ControlUsed` | time, team, action, target, client/login, tasks actually reached |
| `NotificationSent` | actual time, team/task/context, whole rendered title/body, delivered file if useful |
| `RoundClosed` | time, team, round, `RoundEnd` |
| `TeamEnded` | time, team; the one team-close transition |
| `RecordLost` | time/scope where known, expected record kind, typed cause, redacted safe detail, and what content is absent |

A new core `SequencedRecord` should hold the ledger metadata, `Record` and `Cut`; the story never depends on rusqlite. If `Issue` is embedded directly, serde derives are also needed on its nested types in [`crates/agent-core/src/decide/checks.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-core/src/decide/checks.rs). List that file in the PR as an expected-file deviation, or use a fully structured history-local issue record rather than flattening issues to prose.

### One text-processing path

Do not create a second hand-maintained table of text fields for JSON storage. Introduce one exhaustive per-variant text transformation used for:

1. exact-value secret redaction before persistence;
2. configured UTF-8-boundary truncation and `Cut` collection;
3. normalized rows and JSON payloads.

Otherwise a new PR body, answer, notification or loss detail will eventually be redacted in one representation but leaked in the other. Structured identities containing a credential keep the existing omit-and-count behavior. Add generated tests that put distinct oversized/secret text in **every new free-text field**, including `RecordLost.detail` and check/fix findings.

### Losses: what is actually knowable

Add queued, bounded, best-effort `RecordLost` tombstones for queue overflow, individual write failures, secret-identity omission and failed captures. The writer drains pending tombstones on a later successful batch. The existing aggregate `HistoryHealth.lost` and once-per-cause stderr reporting remain unchanged, satisfying status without a protocol-shape change. New capture hooks supply team/run/round/task scope when known.

Rendering rules:

- A surviving tombstone appears at the missing content's position with expected kind and cause.
- A present partial record (for example a launch with `head = NULL`) shows the absent field and its capture cause.
- A missing record with no tombstone says “missing; no stored record / cause unknown”, not a guessed cause.
- An 8.4-only absence of PR records says “not recorded by schema version 2”, not “lost”.
- Cut text displays its `Cut` field names; truncation is not silently presented as whole content.

If the human rejects the best-effort limitation, stop and ask rather than weakening the loss definition. The current aggregate alone cannot support a daemonless, per-round, per-cause story.

## Pure story and CLI

### The pure story module

Keep all formatting deterministic and I/O-free in the new `story` module of [`crates/agent-core/src/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-core/src). Suggested pure inputs are a stored team/plan, one or more round openings and their `SequencedRecord`s; outputs are human lines/fragments and JSON-ready record references.

Human layout:

1. round/team and the complete `ItemText` (or complete brief), plus clear missing warnings;
2. plan participants by role, including `HowChosen` and draw candidates;
3. runs in `(time, sequence)` order, with prompt preview (`N` first lines, byte and line counts), full command arguments, worktree/branch, injected-clock duration, numbered steps unless `--no-steps`, and end;
4. each run's launch/end/changes grouped immediately with that run, including commits, added/deleted/modified/renamed files, line counts, binary files and uncommitted files;
5. gates, checks and complete PR/review/fix activity followed by questions/answers, controls, notifications and round/team closure, ordered by `(time, sequence)`.

Within a run, reuse stored sequence rather than inferring missing steps or inventing ends. Show “no run-end stored” separately from an ended run. A coordinator run linked to two rounds may appear in both human round stories, but must appear once in `--team --json`.

For `--prompts`, write a delimiter/newline, then the entire stored prompt as a raw block. Do not prefix each prompt line: acceptance should be able to find the exact captured reviewer prompt as a contiguous byte string. The same applies to complete PR bodies, review reports and questions. Never wrap long lines or slice by bytes.

`StoryOptions` should have named fields rather than booleans (TYP-4): prompt presentation and step presentation. JSON serialization is independent and uses `serde_json::to_writer` plus exactly one newline per record, retaining sequence/team/rounds/cut metadata. Both output modes consume the same selected record collection, preventing text/JSON divergence.

### Selectors, errors and output

Recommended command shape:

```text
agent log (--round ROUND | --team TEAM) [--prompts] [--no-steps] [--json]
```

- `--round`: one team-start context and everything linked to that round.
- `--team`: team start once, then its rounds by `(opening time, original order)` and every linked record once globally.
- Valid syntax with an unknown ID: typed `LogError::UnknownRound` / `UnknownTeam`, printed with its source context, exit `3`.
- Parse/selector conflicts/invalid IDs: clap exit `2`.
- Missing/corrupt/newer database, invalid stored row, output error, unknown schema: distinct `LogError` sources, exit `1`.
- No daemon is contacted. The path from `AGENTS_HOME` is sufficient; prompts, configuration, state and socket are not needed by `log`.
- For an empty 8.4 round, render team/item/runs then an explicit schema-version note where PR activity would be.

The new log command module under [`crates/agent/src/commands/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent/src/commands) should expose narrow `LogError` and an injected `Write` API. Since direct SQLite is blocking, execute the synchronous CLI command outside an async runtime (or collect owned data in `spawn_blocking`) rather than blocking the daemon/current Tokio worker. All writes are `write_all` and a final `flush` with errors preserved, including a closed pipe.

Required new wiring/documentation outside the item's narrow list, each to name in the PR:

- [`crates/agent/src/main.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent/src/main.rs) and [`crates/agent/src/commands.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent/src/commands.rs) for CLI registration and exit mapping;
- [`crates/agent-runtime/src/github.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-runtime/src/github.rs) if the current remote shape must retain `merge_commit_sha` for merges observed after their response;
- [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/README.md), required by the repository for a user-visible command;
- preferably a new log page under [`docs/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/869901c028fa7f7104d5637e12928acf63edc88f/docs), because the target layout names one; otherwise deliberately fold all CLI semantics into [`docs/history.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/docs/history.md) and explain why;
- new story tests/goldens and process tests, which are fixtures/test files and do not count toward PR-1.

## Daemon recording points

| Behaviour | Exact insertion point and caution |
|---|---|
| gate | `gate_request`: capture request time and rounds through additive metadata; `Service::execute`: capture execution start before inspection, then record only after the gate record is successfully saved, with actual finish/log and total request-to-finish duration. Existing `GateJob` literals are frozen; use additive history/service/queue-scope APIs rather than adding a required field. Associate scope from the task's latest run at gate start when possible. |
| check issues/outcome | `check_before_review`, after `round_issues` and `Checking::conclude`, separately for each outcome branch. Record after the corresponding checks file/questions file is safely persisted; include allowed/refused subjects from an answered earlier question. |
| PR push | immediately after each successful `ready.git.push`, before subsequent operations can hide the event; key repeated retries of the same head idempotently. |
| PR opened | only on the actual create path, not every `find_open_pull_request` retry. Use the returned remote title/body/head. If checks hold it as a draft, the opened record still exists. |
| ready | after successful `mark_ready`, including its head. |
| review asked | after `record_reviews` succeeds, using the `head` already read in `try_ask`, one record per prepared review. Never infer from the later reviewer run alone. |
| review submitted | in `take_verdict`, after `report_text`/save succeeds and before `apply_next`; carry the exact whole report, verdict, pass and matching requested head. Late valid reports are records too but must not advance the current pass. |
| fix pass started | `apply_next` and `start_pass_after_answer`, once per PR/pass when moving to `Fixing`; pass the actual full `reports_of` findings. Do not record every poll in `deliver_fix_passes`. |
| approved / hand to human | when `Next::Merge` or `Next::HandToHuman` is applied. No-merge produces both an approval and the explicit handover record. |
| merged | use `Merged.commit` returned by `merge_pull_request` rather than discarding it. For `PullState::Merged` observed later, retain/read `merge_commit_sha`; a missing merge hash is a capture loss, not an invented ID. |
| closed | when an observed remote pull is closed without merge; do not emit for an already merged PR. Existing polling limitations should not be broadened into new autopilot behavior in this item. |
| question asked | `Owner::ask`, with the actual readable questions content; distinguish the agent's original file from successfully applied instructions. If exact “what the human saw” is intended, record the successfully written prepared content, not a fallback that was never written. |
| answer | extend `record_answer` to receive the `answered` value already available in `answer_request` and `file_saved`; persist `answered.render()` and `via`. |
| control | once in `record_control` when at least one task was reached, with original target/client and actual reached tasks. Existing active-run control steps remain separate records. |
| notification | in [`crates/agent/src/daemon/notifier.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent/src/daemon/notifier.rs) after successful render+delivery, recording the actual rendered title/body. Pass history and preserved team/round context into the delivery. Failed render/delivery needs a loss marker, not a false “sent” record. Queue drops likewise. |
| round closed | in the sole `close_round` transition after successful state transition. |
| team ended | in `close_if_finished` once when the close is accepted, independently of notification delivery. |

The additive API rule matters: current [`GateJob`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent/src/daemon/gate.rs#L46-L67) literals occur in existing tests, while no external `RemotePullRequest` struct literals were found. Prefer `.with_history(...)` and new scoped wrappers over changing existing required constructor fields. Do not add special test behavior to production.

## Behavior-to-test map for the merger

### Storage, pure story and CLI tests

| Behavior | New proof |
|---|---|
| every new record read back equal | Seeded runtime round trips for every variant, batches split at every point; daemon goldens additionally prove recording, because hand-inserted rows prove only queries. Both success and nearest invalid case, in SQL and again when read. |
| natural idempotence / real duplicates | Writing the same keyed record twice preserves the first once; two actual same-time controls/notifications with different event IDs both survive; one PR/head push is not repeated on publication retry. |
| same timestamp keeps write order | Seeded records of different kinds/tables shuffled after sequence assignment; story and JSON order by `(at, sequence)` exactly. Include a clock-back case separately, without sleeping. |
| every selected JSON record exactly once | Parse each JSON line back to its typed value and compare multisets/sequences, not just line totals. Direct SQL independently counts the selected parent sequences. |
| two concurrent teams | Interleaved seeded histories with equal timestamps: no shared record sequence, union equals input, each team's records complete. Use a daemon two-team acceptance case too, not only manual rows. |
| flags | Preview truncates at the documented first-line count and reports sizes; `--prompts` preserves whole stored text; `--no-steps` changes human text only; `--json --no-steps` still has every step. |
| unknown ID / no daemon | Real binary with a stopped/absent daemon: success for existing IDs, exit `3` for each unknown ID, no socket access; malformed scopes exit `2`; malformed/newer DB exits `1`. |
| version 1/2 database | Preserved fixtures retain normalized reads, story renders with documented virtual order, no migration on read, and an 8.4 round without PR data is useful rather than an error. Writer upgrades add only migration 3 and preserve old rows. |
| UTF-8/long lines | Non-ASCII, empty text, exact limit, limit minus/plus one, multibyte cut, one very long line in PR/report/question/notification; raw JSON remains one physical line and no human line is wrapped. |
| secrets | Daemon round with unique marker per text channel, redaction before both representations/truncation, no marker in DB or live WAL or `agent log` output; real CLI input remains unchanged. Secret in an identity is omitted, counted, reported and no secret is printed. |
| loss rendering | Surviving capture tombstones appear in place with causes; missing untracked content is explicit; cut fields are named; an unavailable whole DB produces a read error rather than a fictional story. |

### Seven required golden stories

Each golden comes from a real daemon with fake CLIs + fake GitHub, stopped before the real `agent log` command runs. IDs, commit hashes, timestamps, temp paths and generated sequence numbers are normalized **after** semantic assertions, never by sorting output to hide ordering defects. Before normalization, every persisted reviewer prompt of every pass equals the corresponding `.fake-prompt-N` byte for byte, and launch args equal `.fake-args-N` (excluding the already documented prompt-in-file representation handling).

1. **Solo, first-pass merge:** complete item, plan, actual gate, PR open/push/ready, two approvals, merge commit, team end.
2. **Team + worker, one fix pass:** coordinator/member/worker runs and reasons, actual gate, two first-pass verdicts including changes requested, fix pass with findings, pushed new head, second-pass approvals and merge. This is the primary acceptance check.
3. **Checks send back once, then human:** `check_passes = 1`, no-test issue persists, exact first back-pass and subsequent ask-human records; no reviews start early. Answer `allow` to continue to approvals, and independently test refusal in a focused case.
4. **No merge:** approval plus explicit hand-to-human and round closed `approved`; no fabricated merge. A later human merge is observed with its real merge commit while the round remains approved.
5. **Abandoned:** control stops the team's work; open round closes `abandoned`, human control and notifications are in the story, and resume does not silently reopen it.
6. **Question answered:** the actual asked file, complete verdicts and channel, resumed answers run, and no secret/content loss.
7. **Unicode and very long fields:** non-ASCII item/PR/review/question/notification text and a long single line, including second-pass complete reviewer prompt equality and full stored bodies.

Add focused daemon/process cases for actual gate failure/timeouts/not-clean, question channels (`command`, `saved_file`, `dashboard`), reviewer late reports, closed PR, merge hash observed only on a later GET, killed daemon/recovery, a newer schema, unavailable history, full queue, rejected one-record write, capture-head absent and snapshot failure. Each shows work continuing or the documented refusal and an accurate loss/reporting path.

### Acceptance commands to preserve

After the team + worker + one-fix golden run and `rig.stop()`:

```bash
agent log --round <id>
agent log --round <id> --prompts
agent log --round <id> --no-steps
agent log --round <id> --json | wc -l
```

Assertions must be content-level: item in full, plan/how chosen, every run and its reason/branch/worktree/duration/prompt preview/args/steps/end, files+uncommitted changes per run, actual gate, two review passes with verdicts/full reports, fix findings, new head and merge commit. The second reviewers' whole prompts are compared with fake captures. `wc -l` equals an independent SQL parent-sequence count, not a call to the same selection helper alone.

## TDD/PR sequence and size control

Plan the stack before writing, per PR-2:

1. **Record vocabulary and text preparation:** RED variants, serde, pass/kind parsing, structured issues, redaction/truncation visitor; GREEN normalized + payload text equivalence.
2. **Migration/read model:** RED fresh/preserved schema, ledger round trips, idempotence, same-time order, old-version fallback; GREEN migration 3, backfill, writer/index and reader. Stop here for the required schema-test authorization if it is not already given.
3. **Pure story + CLI:** RED property/order/team-isolation/golden-format/unit CLI failures; GREEN the pure story module, log command, injected writer and exit `3`.
4. **Gates/checks/PR/review/merge:** one RED/GREEN vertical slice at a time through the daemon, with real fake-GitHub outcomes and merge hash.
5. **Questions/answers/controls/notifications/closure:** individual RED/GREEN slices including channels and actual rendered delivery. Gate queue-wait duration is covered by two queued gates under the injected clock, never sleeping.
6. **Seven end-to-end goldens + secret/failure acceptance + docs:** new tests first; fixtures normalized after exact prompt/report/body checks. Rerun the one allowed final gate once.

Likely hotspots are record serde/validation, ledger backfill/read fallback, and daemon attribution. Measure the PR-1 command after each slice. If the estimate crosses `5,000` non-test lines, split before further implementation rather than reducing tests. No new dependency is justified; seeded generation already uses `rand`, JSON uses `serde_json`, and golden normalization needs no regex crate.

## Decisions needed before implementation

1. **Approve the frozen-schema `TEST-FIX:` (recommended).** Permit only the expected table-list addition in [`crates/agent-runtime/tests/history/schema.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/crates/agent-runtime/tests/history/schema.rs) and the matching pinned [`docs/history.schema.sql`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/docs/history.schema.sql), with every existing assertion preserved. Without this, any real migration fails the gate and repository rule 1 blocks editing it.
2. **Accept bounded, best-effort loss tombstones (recommended), or specify another loss contract.** Recommended: preserve every loss that can later be written and honestly mark unknown absences; retain aggregate status. Stronger per-loss recovery across total DB unavailability/restart cannot be truthfully implemented with today's files and “history never blocks work”.
3. **Adopt the flag semantics above (recommended).** Human-only `--prompts`/`--no-steps`, always-complete JSON. Rejecting the combinations instead would also be defensible, but silently filtering JSON would violate the exact-record acceptance property.

No live answer is assumed by this report. These are handoff decisions for the merger/human, not implementations made under an assumption.

## Checks performed for this investigation

- Read the binding [`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/AGENTS.md), [`prompts/agent/rust-developer.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/prompts/agent/rust-developer.md), [`rounds/round-8.5.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/rounds/round-8.5.md), [`docs/architecture.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/docs/architecture.md) and [`docs/history.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/869901c028fa7f7104d5637e12928acf63edc88f/docs/history.md).
- Traced `Record`, writer, reader/migrations, redaction/loss accounting, CLI dispatch, gate execution, checks, publication, review/fix/merge, questions, controls, notifications and closure with Serena definitions/references and targeted reads.
- Confirmed `GateJob` literals and the exact table-list test as compatibility hazards; confirmed no existing `RemotePullRequest` literals outside its own runtime construction.
- No real model CLI, live `~/agents`, hardware, other agent worktree, bus tool or integration suite was used. No new tool/package was needed.

## Not done

- No item 8.5 production code or tests were written, so no behavioral implementation is claimed.
- No schema-test exception was granted and no loss-semantics exception is assumed.
- No old-implementation Go/Bats suites were run; they are explicitly outside this round.
- No physical disk was filled. Existing actual `SQLITE_FULL` techniques should be reused for write-failure proofs rather than filling the host.
- The final `make gate` is intentionally run after committing this report, on that exact commit. Its result accompanies the final handoff rather than being backfilled by a second result-only commit.
