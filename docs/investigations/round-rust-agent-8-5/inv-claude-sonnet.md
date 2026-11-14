# Item 8.5 — finishing report (inv-claude-sonnet)

The run that wrote most of item 8.5 stopped at the turn limit of its CLI; nothing had failed. I did not start again. I read
`git log main..HEAD` and the plan note, compared the branch with the item, and finished what was missing.

## Compared with the item

Present before this hand-over (27 commits, RED/GREEN pairs):

- the records of gates, checks (each issue and what followed), pull request (opened, pushed, ready, review asked and submitted, fix pass, approved, merged, handed over, closed), questions, answers, controls, notifications, the end of a round and of a team (`crates/agent-core/src/history/event.rs`, the ledger in `crates/agent-runtime/src/history/`)
- lost records: counted under their cause, shown by `agent status`, and noted in the history (`crates/agent-runtime/src/history/notes.rs`); the story says what is missing and why
- the story as a pure function (`crates/agent-core/src/story.rs`, `story/text.rs`, `story/json.rs`)
- `agent log --round|--team [--prompts] [--no-steps] [--json]`, exit `3` for an unknown ID, no daemon (`crates/agent/src/commands/log.rs`)
- seven golden stories made by daemon runs (solo merged, team with one fix pass, checks, `--no-merge`, abandoned, question answered, non-ASCII/long lines)

Missing, now done:

- `docs/log.md` — linked from `docs/history.md` but absent
- `docs/architecture.md` — the events and the story (an expected file of the item)
- a test that a secret in a review report stays out of the events and the database bytes (the ground rule on secrets): `history_events_redaction.rs`, checked by disabling the redaction of event texts — the test then fails

## Checks

- `make gate` — `PASS at b9a53b0` (fmt, clippy `-D warnings`, tests, docs), run once on the commit before this report
- size: `3,720` changed lines outside tests, goldens and the earlier investigation report (limit `5,000`)
- not run, by the rules: the old implementation's suites, real CLIs

## Tests changed after RED

All four are `TEST-FIX:` commits that touch tests only; the first two were approved by the human in the round's questions.

- [`5d88ce7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5d88ce76) — the table list of `tests/history/schema.rs` gains the two ledger tables, `user_version` 2→3 in `sessions.rs` (migration 3)
- `9cc791da` — `story_records.rs`: checks the records of the team that owns the first two-round record, not of the whole history
- `372a960a` — `history_events.rs`: the gate's tree is not clean, because the stand-in CLI leaves its files in the worktree
- `337d736f` — `history_events.rs` tables: the nineteenth event kind, `lost`

## Skipped tests, suppressions, dependencies

none, none, none.

## Not verified by me

- The `Tests` table below maps the item's sentences to tests by reading the test names; I ran the whole gate, not each test one by one.
- The manual checkpoint after feature 8 (a real run with cheap models) is the human's.

## Compatibility (rule 9)

- The history schema goes from version 2 to 3 (migration 3, forward-only). A version 1 or 2 file is read through a temporary view and is never written by `agent log`; `a_history_of_item_8_4_is_told_by_the_binary_and_left_as_it_was` proves it. `docs/history.md` and `docs/history.schema.sql` are updated.
- No change to the state file, the control protocol or the configuration.

## Task / Tests (behaviour → test)

| Behaviour | Test |
|---|---|
| each new record written and read back equal | `crates/agent-core/tests/history_events.rs`, `crates/agent-runtime/tests/history/ledger.rs`, `notes.rs` |
| gate, checks, pull request, reviews, fix pass, merge, hand-over, close recorded by a daemon run | `daemon/owner/rounds/tests/history_events.rs` |
| questions, answers (with channel), controls, notifications, team end | same file |
| lost records counted under a cause, shown by `agent status`, said in the story | `history_records.rs`, `history_failures.rs`, `crates/agent/tests/history_process.rs`, `story_text.rs` |
| no secret in the events or the database bytes | `history_redaction.rs`, `history_events_redaction.rs` |
| the story, golden files of the seven rounds | `daemon/owner/rounds/tests/history_stories.rs`, `crates/agent/tests/golden/` |
| every record exactly once in `--json`, order of writing for equal times, two teams share no record | `crates/agent-core/tests/story_records.rs` (seeded) |
| `agent log` with no daemon, unknown ID → exit `3`, a history of 8.4 only | `crates/agent/src/tests/log_command.rs`, `crates/agent/tests/log_process.rs` |
| `--json` line count equals the records of the round in the database | `json_prints_every_record_of_the_round_once_and_the_flags_do_not_change_it` |

## Commits

- [`b9a53b0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b9a53b06fbc1b0af7fc76a594cb61d39b9a62dd0) Document agent log and the events of the history
- [`3d1da56`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3d1da56c00b6dd9e0a4db413313ad0639fabf9f8) Test that a secret in a review report stays out of the events
- [`e09d40e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e09d40eee563cb92ec21a0dab806b44b138167c0) Pin the stories of seven real rounds as golden files
- [`979dd7b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/979dd7b27881c6225e9303ea6c55d438b6ea4f6d) GREEN: note what the database refused and say what is missing in a story
- [`b232bb8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b232bb88eaef6852d9ed85f258388b73cd240279) RED: what the database refused is noted and a story says what is missing
- [`337d736`](https://github.com/mateusz-szczyrzyca/team-agents/commit/337d736f322cadb51b69ac56797fb051e9b1d916) TEST-FIX: let the tables of the event vocabulary know the lost-record event
- [`2505f43`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2505f432f50edbfeefb7a478f624f359f4007039) GREEN: record questions, controls, notifications and the end of a team
- [`07221a2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/07221a26de2dd23a95ebcfb52f19403694626dbb) RED: the daemon records questions, controls, notifications and the end of a team
- [`08c1f7b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/08c1f7bb8b3379106352c3d5b833a83fd5bca971) GREEN: record what the checks before a review found and what followed
- [`be16674`](https://github.com/mateusz-szczyrzyca/team-agents/commit/be16674e7a8bd406652c043eefb60b9d81956d1b) RED: the checks before a review are recorded with each issue and what followed
- [`f20f8e0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f20f8e0c23e2c1b1033a0dd8e6e03a7e5587d582) GREEN: record the gates the daemon runs and link them to their rounds
- [`372a960`](https://github.com/mateusz-szczyrzyca/team-agents/commit/372a960a1bcce99feeb0432acbb732e4451b9e2d) TEST-FIX: expect the tree the stand-in CLI leaves, which is not clean
- [`147b899`](https://github.com/mateusz-szczyrzyca/team-agents/commit/147b8999f99d5082666fbc9b94a518faee3f049f) RED: the gate a daemon ran is recorded and linked to the round of its task
- [`7ba92d7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7ba92d774dd26efd059c0ea595f1b78dad2e25f8) GREEN: record a pull request, its reviews and the end of its round
- [`f7ef159`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f7ef1590ba71ab75030d8a16cbbd3049ab97df58) RED: the daemon records a pull request, its reviews and the end of its round
- [`dc4ca62`](https://github.com/mateusz-szczyrzyca/team-agents/commit/dc4ca62316d8115aa2a2c3715092b4b8e8b4f15f) GREEN: agent log reads the history file and prints the story
- [`56cc08d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/56cc08ddf45250689a4f08ecdc24e3ee2295aa60) RED: agent log tells a round or a team from the history file alone
- [`1d52cce`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1d52ccecff21298e349620b733742a4ec7274e0a) GREEN: tell a round from its records, in time order
- [`9cc791d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9cc791da471dbc2e62017c6056a8113d24e3875e) TEST-FIX: take the records of one team when a record of two rounds is checked
- [`1fe99b1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1fe99b17f3ab022b8e731125be4a2987c9d48b19) RED: the story of a round is told in time order from the records alone
- [`eeb6975`](https://github.com/mateusz-szczyrzyca/team-agents/commit/eeb6975e7848e1461f4291e2478468c33eaba3ad) GREEN: keep every record in a ledger and read a round's records from it
- [`5d88ce7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5d88ce76ca6d06f85036acfb39cc274fe6cb338c) TEST-FIX: Permit the ledger tables and the version of migration 3
- [`af98819`](https://github.com/mateusz-szczyrzyca/team-agents/commit/af98819ad5f716511cf05726683e9c06e5ec253f) RED: records have a place in write order and events are kept in a ledger
- [`fe5a2c8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/fe5a2c81386cf12249d826e07d68274071beb0ae) GREEN: name, key, redact and guard the events of a round
- [`1c997b7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1c997b72e742ac3da2b625212003c6da0900fe89) RED: history events of a round have kinds, keys, texts and identities
