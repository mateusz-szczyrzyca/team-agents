# Item 8.3 — rounds as an entity (`inv-claude-sonnet`)

Branch `agents/inv-claude-sonnet/20261113-224235`, base `16f50edd`. No PR was opened. Phase 1 of an investigation team,
worked alone: no bus, no helpers.

`make gate` passes on the committed tree (`5363933`).

## What was done

| Commit | What |
|---|---|
| `50d85d18` | Mechanical rename: the round of a questions file is its `number` (code, wire key, prompts, docs). |
| `c79a147b` | Mechanical rename: the reviews asked for one head are a review pass; a look at a team is a `Look`. |
| `751e3e21` **RED** / `cfbed7e0` **GREEN** | `RoundId` (`round-YYYYMMDD-HHMMSS-xxxx`) and `RoundId::derive` for the ID of a round of a version 1 state. |
| `37d6dae8` **GREEN** | Rounds: state v2 and its invariants, the v1 migration, `rounds_of_run`, protocol v2, the daemon, the CLI. |
| `0f0ab839` | `docs/state.md` and `docs/architecture.md`. |
| `5363933` | A clippy finding in a new test. |

- **Core, state v2** (`agent-core`): `Round { id, item?, opened_at, state: open | closed { at, how } }`, `RoundEnd` is
  `merged | approved | abandoned`. A team holds `rounds`, a run names `rounds`. `State::{open_round, close_round,
  begin_run}` and `TeamMut::{open_round, close_round, advance_item, begin_run}` check the invariants. `check_rounds`
  checks them again on read.
- **Migration** (`state/migrate.rs`, `state/team/legacy.rs`): a v1 file is read as a typed v1 state and becomes a v2
  state. Every item past `pending` gets a round with a derived ID (collisions are probed), `opened_at` is the team's
  `created_at`, the round is closed `merged` at the merge, `approved` at the last review when the team does not merge
  by itself, otherwise open. Runs name no round. A question's `round` becomes `number`. Saving writes v2.
- **Rounds of a run** (`decide/round/runs.rs`): a pure function of role and start reason, the table of the task.
- **Daemon**: a round is opened in `assign_items` before the item is recorded as given, and in `team/form.rs` for the
  first assignments. It is closed `merged` in the merge path and `approved` when `--no-merge` hands the PR to the human.
  Open rounds are abandoned when the team ends or is stopped with no task left that can run. Events `round_opened` and
  `round_closed` are new, and `round` is on the events about an item, a PR, a review and a run.
- **Protocol v2**: `Target::Round`, `Reply::Round` with `RoundDetail`, rounds in `TeamSummary`.
- **CLI**: `agent run` prints a round line per round after the tasks. `agent status <team>` lists the rounds under the
  team. `agent status <round>` shows the item, the tasks (role, agent, worktree, branch, runs), the PR and the review
  passes. `agent status` takes a team, a task or a round ID.

## Acceptance checks

- `agent run` of a two-item list prints two round IDs:
  `agent_run_prints_the_round_of_each_item_and_agent_status_of_a_round_shows_its_item_tasks_and_pull_request` and
  `the_first_items_of_a_team_get_a_round_each_when_the_team_starts_and_every_run_names_the_rounds_of_its_role`.
- `agent status <round>` shows item, tasks and PR: the first test above and
  `a_round_is_shown_with_its_item_its_tasks_with_where_they_work_and_their_runs`.
- A v1 state in a fresh `AGENTS_HOME` is read and saved as v2: the store test in `agent-runtime/src/store.rs` (fixture
  `state-v1.json` in, `state-v1-as-v2.json` out) and `a_migrated_state_is_saved_as_version_2_and_loads_again`.
- `git grep -nwi round -- crates docs/state.md` gives about 1100 lines. I read the lines, not each one: the ones
  that remain mean the new entity (one team's work on one item) or, in older text, the work of one item; the questions file is
  a `number` and the reviews of one head are a `review pass`. `docs/state.md` mentions the v1 key `round` of a question on purpose,
  in the migration section.

## Tests

Behaviour → test (the full list is in the files; `core` is `crates/agent-core`).

| Behaviour | Test |
|---|---|
| ID of a round has the shape and parses | `ids.rs` tests (RED `751e3e21`) |
| Derived ID is stable and differs per position | `ids.rs` tests (RED `751e3e21`) |
| A round opens open, with its item and time | `a_new_round_is_open_with_its_item_and_the_time_it_was_opened` |
| A round closes once, not before it opened | `a_round_is_closed_once_with_the_time_and_the_way_it_ended`, `a_round_is_not_closed_before_it_was_opened` |
| An item has at most one round, even if the first is closed | `an_item_has_one_round_even_when_the_first_is_closed` |
| A closed round never opens again | `a_closed_round_is_never_opened_again` |
| An item leaves `pending` only with a round | `an_item_leaves_pending_only_with_a_round` |
| A round is refused for a done item | `a_round_is_refused_for_an_item_that_is_done` |
| `merged` needs an item done as merged, `approved` an approved one, `abandoned` anything | `a_round_ends_as_merged_only_when_its_item_is_done_as_merged`, `a_round_ends_as_approved_when_its_item_is_approved_or_done_as_merged_and_not_earlier`, `a_round_is_abandoned_whatever_its_item_is_doing` |
| A list without items has one round for its whole work | `a_list_without_items_has_one_round_for_its_whole_work_and_no_round_for_an_item` |
| A run names only rounds of its team, none twice, nothing is recorded on refusal | `a_run_naming_a_round_the_team_does_not_have_is_refused_and_nothing_is_recorded`, `a_run_naming_a_round_twice_is_refused_and_nothing_is_recorded` |
| The same invariants hold on read | `rounds::*_is_refused_when_read` (nine tests, one per rule) |
| A round ID is unique in the whole state | `a_round_id_is_used_once_in_the_whole_state_not_only_in_one_team`, `a_round_id_in_two_teams_is_refused_when_read_with_the_team_that_repeats_it` |
| Any sequence of operations saves and loads to the same text (seeded, 300 seeds × 60 ops) | `any_sequence_of_operations_leaves_a_state_that_saves_and_loads_to_the_same_text`; `the_sequences_open_close_and_name_rounds_often_enough_to_mean_something` guards that the sequences are not vacuous |
| Rounds of a run by role and reason | `the_rounds_of_a_run_follow_its_role_and_why_it_starts`, `every_role_is_in_the_table`, `a_continuation_is_the_previous_rounds_whatever_the_role_and_the_state_say_and_the_rest_is_drawn_from_the_state` |
| v1 reads as exactly the v2 it stands for, saves as v2 | `a_state_of_version_1_loads_as_exactly_the_state_of_version_2_it_stands_for`, golden `state-v1-as-v2.json` |
| Migration is deterministic and the IDs are derived | `reading_the_same_state_of_version_1_twice_gives_the_same_state`, `each_item_past_pending_gets_a_round_whose_id_is_derived_from_its_team_and_its_place` |
| Migration closes merged / approved rounds and leaves the others open | `an_item_that_was_merged_gets_a_round_closed_as_merged_at_the_time_of_the_merge`, `an_approved_item_of_a_team_that_does_not_merge_by_itself_gets_a_round_closed_as_approved_at_its_last_review`, `an_approved_item_of_a_team_that_merges_by_itself_keeps_its_round_open_until_the_merge` |
| Runs of v1 name no round | `the_runs_of_version_1_name_no_round_and_its_questions_are_numbered` |
| Schema 3 is refused, v1 rules stay | `a_state_of_schema_version_3_is_refused_as_an_unsupported_version_whatever_else_it_holds`, `a_state_of_version_1_that_breaks_a_rule_of_version_1_is_still_refused` |
| Protocol v2 lines read and write back (goldens) | `protocol.rs` tests on `protocol-v2-*.jsonl`, `protocol_rounds.rs` on `protocol-v2-round-*.jsonl` |
| Protocol v1 is refused | `every_line_of_version_1_is_refused_as_an_unsupported_version` |
| `show` of a round | `the_round_a_show_gives_has_its_item_its_state_its_tasks_its_pull_request_and_its_review_passes`; `tests/round_detail.rs` (PR, review passes oldest first, members of a round) |
| Events name the round | `the_events_that_are_about_a_round_name_it`; daemon `the_events_about_an_item_a_pull_request_a_review_or_a_run_name_the_round` |
| Daemon: two items, two rounds, in order | `a_solo_agent_on_a_two_item_list_opens_and_closes_its_two_rounds_in_order` |
| Daemon: coordinator, member, reviewers, worker | `a_coordinator_a_member_and_the_reviewers_name_the_rounds_of_their_roles`, `a_coordinator_a_member_and_a_worker_on_a_two_item_list_open_and_close_two_rounds_in_order_and_each_run_names_its_rounds` |
| A fix pass stays in its round | `a_fix_pass_stays_in_its_round` |
| `--no-merge` closes `approved` and stays so after a human merge | `with_no_merge_a_round_is_closed_as_approved_when_the_reviews_approve_and_stays_so_when_the_human_merges` |
| A list without items is one round that ends with the team | `a_list_without_items_is_one_round_that_its_solo_agent_works_for_and_that_ends_with_the_team` |
| A stop abandons the rounds only when nobody is left to run | `a_stop_that_leaves_the_team_nobody_to_work_abandons_its_open_rounds_and_a_stop_of_one_task_does_not` |
| Two teams never share a round | `two_teams_started_at_once_never_share_a_round` |
| A restart goes on with the same round | `a_daemon_stopped_in_the_middle_of_a_round_goes_on_with_the_same_round`, `a_daemon_stopped_in_the_middle_of_a_round_goes_on_with_the_same_round_id` |
| `status` / `show` of a round, unknown round, round not controllable | `a_round_is_found_by_its_id_in_status_and_show_an_unknown_one_is_not_found_and_a_round_is_not_controlled` |
| Human-read text (golden-style, exact lines) | `crates/agent/src/commands/status/tests.rs`, `commands/run/tests.rs` |

The daemon tests run the real `agentbus` and the fake CLI script of `crates/agent/tests/fixtures/team-cli.sh` with the
fake GitHub of the rig. No real CLI was run.

## Existing tests changed

Rule 1 forbids it unless the task changes the behaviour. All of these are forced by the task:

- **Renames of the mechanical commits** (`50d85d18`, `c79a147b`): `approval_round.rs`, `checks_record.rs`,
  `item_lifecycle.rs`, `pull_request_kept.rs`, `review_requests.rs`, the `agent-runtime` tests `checks_files.rs`,
  `home_layout.rs`, `questions_prompts.rs`, `run_compose.rs`, and in `agent`: `daemon/{checks,merge,notice}/tests.rs`,
  `owner/answers/tests.rs`, `owner/rounds/tests.rs`, `owner/rounds/tests/checks.rs`, `questions/tests.rs`,
  `questions/watch/tests.rs`, `daemon/tests.rs`, `tests/steer_commands.rs`, `tests/steering_process.rs`. Only identifiers
  and wire keys changed (`round` → `number` / `review pass`); no expectation changed.
- **Protocol 1 → 2** (`37d6dae8`): `protocol.rs` reads the `protocol-v2-*` fixtures (same lines with `"protocol":2`) and
  expects `UnsupportedVersion` at 3 and 7; `public_client.rs` builds a v2 request; `agent/src/client/tests.rs` expects
  the unsupported version 3. The v1 fixtures are untouched, and `protocol_rounds.rs` checks that v1 lines are refused.
- **Schema 1 → 2**: `state.rs` (`agent-core/tests`) now round-trips `state-v1-as-v2.json`, because reading
  `state-v1.json` and writing it gives v2; the `"round": 1` key of a question is `"number"`; the unsupported version
  is 3. `agent-runtime/src/store.rs` inline tests use the same fixture and version. `state-v1.json` itself is unchanged.
- `public_deserialize.rs`: protocol number and question key as above.
- `agent/src/tests/fixtures.rs`, `owner/teams/tests.rs`, `commands/*/tests.rs`: helpers (`pr`, `review`) and imports
  added, existing tests untouched. In-flight items in a few existing tests open a round first, because a state
  with an item past `pending` and no round is now refused on read.

## Lint suppressions

- `crates/agent-core/src/ids.rs`: `split_team_id` and `suffix_text`, `#[expect(clippy::expect_used, reason = ...)]` on
  values that were checked by the parse before (3 attributes).
- `crates/agent-core/src/state/team/rounds.rs`, `describe_item`: `#[expect(clippy::ref_option,
  clippy::trivially_copy_pass_by_ref, reason = ...)]` for the signature `&Option<ItemId>` that the error type stores.

## Assumptions and decisions

1. **Stop and resume.** The text says a stopped team's rounds are abandoned, but `agent resume` goes on. I abandon the
   open rounds only when no task of the crew can run any more (all stopped, failed or done). A resumed team continues in
   the round it had if it is still open; a round that was abandoned stays closed, because a closed round is never opened
   again, and the item goes on without a new round (the invariant "an item has at most one round" wins).
   Needs the human's decision if a resumed item should get a second round.
2. **Item-less list**: its round closes `abandoned` when the team ends (it has no item to merge).
3. **Invariant dropped**: "an open round has an item that is not done" is not checked on read. The state can be saved
   between the item's merge and the close of its round by a crash, and such a state must still load. I did not add a repair of it on the next look. The checks
   on read are the four in the task.
4. **`approved` for the human merge.** With `--no-merge` a round closes `approved` and stays so when the human merges
   later; a `merged` round needs an item done as merged.
5. **Migration time**: `opened_at` is the team's `created_at`, as v1 holds no start time per item; a merge dated before it
   closes the round at `opened_at`.
6. **Reviewer's first run** has no reason; I read the round from the item with the review pending for the task.
7. **Worker rounds**: the rounds of the latest run of the asking task when the worker starts.
8. The clippy `assert_is_empty` lint came in with the toolchain used by the gate; fixed in `5363933`.

## Process notes

- **Test-first.** Only the ID of a round has real RED/GREEN commits. The rest was written as one change across the
  state, the protocol and the daemon, which only build together, and is committed as one GREEN with
  `Test-First: not possible`. I do not claim a RED failing output for it. A reviewer may want it split.
- **Size.** `agent round-size`: +8145 −737 over 108 files, of which about 4960 lines are tests and fixtures; docs
  included. The Rust plan limit is 5000 lines without tests.
- **Flaky.** One `owner::teams` test timed out once in a full workspace run, and passed in more than five reruns and in
  the gate. I did not find the cause; it runs the real `agentbus` and the fake CLI under load.

## Suggested changes to `AGENTS.md`

- In "Rust", say that a state with an item past `pending` and no round is refused on read, so a test that builds such a
  state by hand must open a round first.
- In "Workflow", say that `make gate` refuses a tree with uncommitted changes in a worktree (commit first); it cost me a
  run.

## Verification run (task `20261114-063114-inv-claude-sonnet`)

The first run reached the CLI's turn limit after its last commit. This run checked the work against the item and changed
only what was wrong.

- **Gate:** `agent gate` on `64a1a9d`, before any change: `9/9` steps passed (`unit-bats` 223s, `rust` 55s).
- **Size:** `3,606` changed lines outside tests, fixtures and `tests/`, against the Rust plan's `5,000` (measured with the
  PR-1 command over `16f50edd...HEAD`, report included).
- **Read in full:** `state/round.rs`, `state/team/rounds.rs` (operations, their errors, the checks on read) and the daemon's
  `owner/round_state.rs`. The invariants of the item are all there, in the API and again in `check_rounds`.
- **Search for the PR-1 hits** (`unwrap(`, `expect(`, `let _ =`, `.ok()`, `unwrap_or`, `as `, `#[allow`) over the added
  lines outside tests: the `expect`s in `ids.rs` are the three listed under "Lint suppressions"; the `unwrap_or_default`
  calls are specified fallbacks (a task with no earlier run has no rounds; an item-less round has no review passes), except
  one.

### Fixed

- `Owner::abandon_open_rounds` read a team that is not in the state as a team with no open rounds
  (`state.team(team).map(..).unwrap_or_default()`): a lost lookup that vanished (ERR-2). Both callers check the team
  first, so no behaviour was wrong today, but the function said nothing when the state was inconsistent. The lookup is now
  `open_rounds_of`, which returns `OwnerError::LostTeam`.
  - $\color{red}{\textbf{RED}}$ `371fd5ab`: three tests in `owner/round_state/tests.rs`, two fail on their assertions.
  - $\color{green}{\textbf{GREEN}}$ `46d1fc63`: `3/3` pass.

### Not changed

Everything else in the first run's report stands, including its assumptions (stop and resume, the dropped
"open round has an item that is not done" check, `approved` after a human merge) and its note that only the ID of a round
has real RED and GREEN commits. These are for the human to decide.
