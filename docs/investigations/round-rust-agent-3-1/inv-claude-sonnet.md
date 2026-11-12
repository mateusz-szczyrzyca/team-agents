# Round rust-agent/3.1 — core decisions — inv-claude-sonnet

Independent Phase 1 result for `20261112-102926-inv-claude-sonnet`, branch `agents/inv-claude-sonnet/20261112-102926`,
based on [`196f2e1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/196f2e13a0f2d80dd5d57e75a827cc2980a34f34).
No pull request is opened; this file is what the PR description would say.

## What and why

Item 3.1 puts every decision of the harness in four pure, fully tested modules of `agent-core`, so that the daemon only
carries them out:

- [`decide/team.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/d0e15146f0ac5785102c2147729399ee48ff19be/crates/agent-core/src/decide/team.rs): `plan_team`, `TeamRequest`, `TeamPlan`, `TeamRoster`, `TeamPlanError`
- [`decide/assign.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/d0e15146f0ac5785102c2147729399ee48ff19be/crates/agent-core/src/decide/assign.rs): `next_assignments`, `first_free_worker`, `AssignmentRequest`, `Assignment`, `AssignError`
- [`decide/round.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/d0e15146f0ac5785102c2147729399ee48ff19be/crates/agent-core/src/decide/round.rs): `RoundState`, `RoundEvent`, `RoundState::transition`, `RoundAction`, `Step`, `RoundPolicy`, `RoundError`
- [`decide/outcome.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/d0e15146f0ac5785102c2147729399ee48ff19be/crates/agent-core/src/decide/outcome.rs): `classify_outcome`, `parse_status_line`, `after_hang`, `Classified`, `Pause`, `LimitContext`, `ClassifyError`
- fixtures of the four CLIs under [`crates/agent-core/tests/fixtures/cli-outputs/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/d0e15146f0ac5785102c2147729399ee48ff19be/crates/agent-core/tests/fixtures/cli-outputs) and their test [`tests/outcome.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/d0e15146f0ac5785102c2147729399ee48ff19be/crates/agent-core/tests/outcome.rs)

Every rule is in the module documentation of its file, with the order in which checks run.

## Task

> 3. Core decisions
> Depends on: 2 (merged before this round)
> [ ] 3.1 Team formation, assignment, the autopilot round and run outcomes
>     Pure functions, each fully tested. Team formation: from the requested agents, the configuration, an optional
>     coordinator and a random source to a team plan — solo or team, the coordinator by the rule of the brief, typed
>     refusals (unknown agent, worker as coordinator or member, too many investigators in the team for two outside
>     reviewers). Work assignment: one item per normal member; items of a feature in order (the next starts when the
>     previous is merged, or approved with `--no-merge`); features in parallel once their "Depends on" features are
>     merged. The round of one item as typed states and events: member work → coordinator integration → gate green at
>     the head → PR open → two reviews → merge (or handed to the human with `--no-merge`) → item done; "changes
>     requested" → fix pass, at most the configured number, then a question to the human. Run outcome
>     classification from the final message, exit status and error tail: done, questions, failed, quota exhausted
>     (with the reset time when the CLI gives one), rate limited, hung — and the pause each implies; fixtures of real
>     CLI outputs. Reference: `bin/agent` limit and quota handling.
>     Expected result:
>     - Files: `crates/agent-core/src/decide/{team,assign,round,outcome}.rs`; fixtures of CLI outputs under
>       `crates/agent-core/tests/`.
>     - Adds: `plan_team`, `next_assignments`, `RoundState` and `RoundEvent` with their transition function,
>       `classify_outcome`.
>     - Tests: table tests per rule — coordinator precedence, each refusal, investigators in the team, assignment
>       order across features and items, every round transition, each outcome kind per CLI.
>     - Why: every decision of the harness in one pure, fully tested place; the daemon only carries them out.
>     Checks:
>     - Each rule of "Behaviour in short" has a named test; the PR lists them.
>     - `git grep -nE 'std::(fs|process|net)|tokio' -- crates/agent-core/src` finds nothing: the decisions do no I/O.

## Files

Expected by the item:

- `crates/agent-core/src/decide/{team,assign,round,outcome}.rs`
- `crates/agent-core/tests/fixtures/cli-outputs/{claude,codex,gemini,opencode}.json` and `crates/agent-core/tests/outcome.rs`

Outside the item's list, each with its reason:

- `crates/agent-core/src/decide.rs`: the module root (`pub mod` of the four files and `REVIEWS_REQUIRED`, shared by `team` and `round`).
- `crates/agent-core/src/lib.rs`: `pub mod decide;` and the crate documentation's "later" turned into a link.
- `docs/architecture.md`: a section for the decisions, next to the vocabulary, configuration and contract sections.
- this report.

No dependency was added; `Cargo.toml` and `Cargo.lock` are unchanged. No existing test, helper or fixture was changed.

## Size

`1,688` changed lines outside tests by the PR-1 command (`git diff -U100000 196f2e1...HEAD` with its awk), limit `5000`;
about `1,900` with `docs/architecture.md` and this report. No split was needed.

## Tests

Test first, as `docs/tdd.md` says: per module one RED commit (tests and a compiling stub that returns a wrong value, the
failing run in the message) and one GREEN commit.

| Module | RED | GREEN |
|---|---|---|
| team | [`7a8649f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7a8649fb44d1567d687ed1a4476cf9540afe50a6) `27` of `30` fail | [`79a03b5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/79a03b59920dfe5451dde2234170ed5264963cce) `30/30` |
| assign | [`0a4fb88`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0a4fb88f837f591cbe3c447748144e8cd3a112a1) `18` of `20` fail | [`d3c1e60`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d3c1e60549ba6da9336e66ab4699b0a185bfb458) `20/20` |
| round | [`bd3adaf`](https://github.com/mateusz-szczyrzyca/team-agents/commit/bd3adaf4cb2adbd73d2ec597a7b41b8ae86a12fe) `25` of `29` fail | [`f0ad5e0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f0ad5e0b5fc7e12a5f6ed70b42a55154fb30b338) `29/29` |
| outcome | [`46b24cc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/46b24ccc58a81d56988d36f3e7f185a0cdd80d1a) `41` of `45` unit and `1` of `4` fixture tests fail | [`cbd9330`](https://github.com/mateusz-szczyrzyca/team-agents/commit/cbd933080413d52190a943db48a0919762041613) `50/50` and `4/4` |

Two RED tests of `team` specified an impossible case (two investigators of three configured leave one outside, which the
rule refuses); the GREEN commit gave them a configuration of four. The tests that pass under a stub are guards (a stub
that returns a plausible value); the mutation check below shows they guard something.

**Mutation check** (TDD-2): `35` one-line changes to the rules of the four modules, one at a time (swap the coordinator
precedence, `>=` to `>`, drop the busy check, drop the head check, no cap, no margin, ...). Every change fails a test.
Two rules found unguarded (a Unix time has ten digits; a month needs three letters) got their test.

### Each rule of "Behaviour in short" and its tests

All names are in the `tests` module of the file named, except the `outcome` fixtures.

- **One agent is a solo task, more is a team** — `team`: `one_agent_is_a_solo_task`, `one_investigator_is_a_solo_task`, `a_solo_agent_named_as_coordinator_stays_solo`, `more_than_one_agent_is_a_team`
- **Coordinator: explicit wins, then an investigator (drawn among two or more), then a drawn normal agent** — `team`: `the_flag_coordinator_wins_over_an_investigator`, `the_listed_coordinator_wins_over_an_investigator`, `the_flag_coordinator_wins_over_the_listed_one`, `an_explicit_coordinator_may_be_an_investigator`, `the_only_investigator_of_the_team_coordinates`, `the_coordinator_is_drawn_among_the_investigators_of_the_team`, `the_same_random_source_draws_the_same_coordinator`, `a_normal_agent_is_drawn_when_no_investigator_is_in_the_team`
- **Workers never coordinate and take no items** — `team`: `a_worker_is_never_drawn_as_coordinator`, `a_worker_as_the_explicit_coordinator_is_refused`, `only_workers_are_refused`, `a_coordinator_with_only_workers_is_refused`, `workers_do_not_count_as_investigators_in_the_team`
- **Refusals** — `team`: `an_unknown_agent_is_refused`, `an_unknown_coordinator_is_refused`, `an_unknown_agent_is_reported_before_a_worker_coordinator`, `a_coordinator_that_is_not_requested_is_refused`, `a_missing_member_is_reported_before_the_reviewer_count`
- **Two reviews from investigators outside the team; too many investigators refused** — `team`: `one_investigator_of_three_leaves_two_outside`, `two_investigators_of_three_are_too_many`, `an_investigator_coordinator_counts_as_in_the_team`, `a_solo_investigator_needs_two_others_outside`, `one_investigator_of_two_is_too_many`, `too_few_configured_investigators_are_refused_even_without_one_in_the_team`; `round`: `the_first_approval_waits_for_the_second`, `two_approvals_are_merged_in_autopilot`, `the_same_reviewer_cannot_approve_twice`
- **Members in the order named; workers in the configured order** — `team`: `members_keep_the_order_they_were_named`, `workers_are_in_the_configured_order`; `assign`: `a_delegation_goes_to_the_first_free_worker_in_the_configured_order`
- **One item to each member at a time** — `assign`: `the_first_item_goes_to_the_first_member`, `independent_features_go_to_different_members_in_the_order_written`, `members_beyond_the_items_get_nothing`, `a_member_with_an_item_in_progress_is_busy`, `a_member_fixing_an_item_is_busy`, `a_member_whose_item_is_in_review_is_free`, `only_the_available_members_take_items`, `a_member_listed_twice_gets_one_item`, `a_missing_progress_is_refused`
- **Items of a feature in order; the next starts when the previous is merged, with `--no-merge` approved** — `assign`: `a_feature_gives_one_item_at_a_time`, `the_next_item_waits_while_the_previous_is_in_review_fixed_or_approved`, `the_next_item_starts_when_the_previous_is_merged`, `with_no_merge_the_next_item_starts_when_the_previous_is_approved`, `with_no_merge_an_item_in_review_still_holds_the_next`, `items_marked_done_in_the_list_are_skipped`, `a_finished_list_assigns_nothing`
- **Features in parallel once their "Depends on" features are merged** — `assign`: `a_feature_waits_for_the_features_it_depends_on`, `a_feature_with_two_dependencies_waits_for_both`, `an_approved_item_does_not_release_a_dependent_feature`
- **The round: member work → integration → gate green at the head → PR → reviews → merge or hand-over → done** — `round`: `a_round_starts_with_the_members_work`, `the_members_done_hands_its_branch_to_the_coordinator`, `a_green_gate_publishes_the_pull_request`, `a_red_gate_goes_back_to_the_coordinator`, `a_published_pull_request_asks_for_reviews`, `a_pull_request_for_another_head_than_the_gated_one_is_refused`, `two_approvals_are_handed_to_the_human_with_no_merge`, `a_merge_completes_the_item`, `the_merge_of_another_pull_request_is_refused`, `an_autopilot_round_runs_from_work_to_done`, `a_no_merge_round_stops_at_the_human`
- **Every round transition and every refused one** — `round`: `only_the_events_that_fit_a_phase_are_taken` (every phase against every event kind), `the_checks_cover_every_phase_and_event_kind`, `every_transition_is_a_change_the_item_phase_allows`, `the_item_phase_follows_the_round`
- **"Changes requested" → a fix pass, at most the configured number, then the human** — `round`: `changes_requested_start_a_fix_pass`, `changes_requested_after_an_approval_start_a_fix_pass`, `the_last_configured_fix_pass_is_started`, `changes_requested_after_the_last_fix_pass_ask_the_human`, `with_no_fix_passes_the_human_is_asked_at_once`, `the_humans_answer_starts_one_more_fix_pass`, `the_human_is_asked_again_after_the_granted_pass`, `an_updated_pull_request_is_reviewed_again`, `a_fix_pass_published_to_another_pull_request_is_refused`, `a_round_with_a_fix_pass_ends_done`, `the_policy_takes_the_fix_passes_of_the_review_configuration`
- **Run outcomes** — `outcome` (`50` unit tests): the status line (`the_status_line_*`, `a_status_word_of_the_old_harness_is_unknown`), the order of the decision (`a_reported_status_ends_a_clean_run`, `the_humans_stop_is_stopped_whatever_the_run_said`, `a_run_the_watchdog_killed_is_hung`, `a_hung_run_is_not_a_limit`, `a_status_reported_before_the_watchdog_killed_the_run_stands`, `a_run_without_a_status_has_no_status_and_its_exit_code`, `an_unknown_status_word_is_no_status`, `a_reported_status_of_a_failed_run_without_a_limit_stands`, `a_successful_result_stands_with_a_failed_exit_code`), the limits (`an_exhausted_quota_pauses_for_the_default`, `the_words_of_an_exhausted_quota`, `a_rate_limit_pauses_for_its_default`, `an_overloaded_model_pauses_like_a_rate_limit`, `a_quota_wins_over_a_rate_limit`, `rate_limit_reached_is_a_rate_limit`, `a_context_limit_is_no_provider_limit`, `a_local_error_is_no_limit`, `a_clean_run_is_not_paused_for_a_stale_429`, `the_agents_own_report_is_not_read_for_limits`), the reset time (`a_unix_reset_time_is_read`, `a_reset_duration_is_read`, `an_iso_reset_time_is_read`, `a_date_with_a_month_name_is_read_in_local_time`, `a_time_of_day_is_the_next_one_in_local_time`, `text_that_is_no_reset_time_gives_the_default_pause`, `a_far_reset_time_is_cut_to_the_longest_pause`, `a_reset_time_in_the_past_gives_the_default_pause`), the pause (`the_default_pauses_come_from_the_configuration`, `the_evidence_is_the_first_limit_line_cut_to_size`, `a_pause_beyond_the_last_year_is_an_error`) and `after_hang` (`a_hung_run_is_resumed_until_the_strikes_are_used`, `a_hung_run_without_a_session_is_marked_hung`, `the_strikes_come_from_the_watchdog_configuration`)
- **Each outcome kind per CLI** — `tests/outcome.rs`: `every_case_of_every_cli_is_classified_as_recorded` (`4` CLIs × `13` to `14` cases: done, questions, failed, sleeping, hung, stopped, no status after a question, a crash, a quota in two forms, a rate limit, an overloaded model, a retried 429 in a clean run), `every_cli_has_a_case_of_each_outcome_kind`, `every_cli_has_a_quota_case_with_a_reset_time`, `there_is_a_fixture_for_every_cli_and_it_says_where_its_texts_come_from`

## Checks

- `git grep -nE 'std::(fs|process|net)|tokio' -- crates/agent-core/src` finds **one line**, which is not mine and is not I/O: a comment in
  [`config/values.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/196f2e13a0f2d80dd5d57e75a827cc2980a34f34/crates/agent-core/src/config/values.rs#L7)
  that says it uses `core::net`, "not `std::net`". The decisions' files have no hit. The check as worded cannot pass until that comment
  is reworded; I left it (out of scope, see below).
- `make rust`: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` (`528` unit tests (`129` of them in `decide`) and `23` doctests in `agent-core`, plus the integration tests), `cargo doc -D warnings`: all pass, `0` findings.
- `make gate` at `d0e1514`: `gate: PASS`, recorded (it runs the Rust scope only for a change under `crates/`).

## Deviations

- Lint suppressions: none. No `unsafe`, no new dependency, no `#[expect]`.
- Hits of the ERR-2 search that I judged specified behaviour, each with a test:
  - `outcome.rs`: `parse().ok()?`, `split_at_checked(..)?`, `Date::from_calendar_date(..).ok()?` and similar in the readers of the reset time (`ResetReader`): text that is
    no time is no time, and the limit then has the configured default pause (`text_that_is_no_reset_time_gives_the_default_pause`).
  - `strip_prefix(..).unwrap_or(..)` for an optional prefix (the `s` of `resets`, the `:` of `unix:`, the space before a unit).
  - `unwrap_or(len)` on `find` for "the rest of the text".
- Wildcard-free `match` on `RoundEvent` in every state of `RoundState::transition` (TYP-6): each state lists the events it refuses. It is long on purpose.
- `Participant` carries the coordinator in `TeamPlanError::NoMember` because it can be an investigator; names in other errors are `AgentName`, which the three tiers share in form.

## Decisions to review

These are readings of the brief where it was silent; each is a typed, tested rule that is cheap to change now.

1. **The explicit coordinator must be one of the agents named** (`TeamPlanError::CoordinatorNotRequested`) rather than being added to the team.
2. **Fewer than two configured investigators refuses every team**, even one with no investigator in it (`TooFewInvestigators`), by the literal reading "leaves fewer than two outside". A solo task is a team of one and is checked the same. The message "too many investigators in the team" is `TooManyInvestigators`, used when the team has investigators.
3. **Workers are in a team only when named**, ordered as the configuration orders them; a worker alone, workers alone, or a coordinator with workers and no member are refused (`OnlyWorkers`, `NoMember`).
4. **A dependency feature is released by a merge only**, also with `--no-merge`; the next item of the *same* feature is released by an approval with `--no-merge`, as the brief says. An approved branch is not on the base branch a dependent feature builds on.
5. **Fix passes are counted as the state records them** (`ItemProgress::fix_passes`); the human's answer grants one more pass, so the next "changes requested" asks the human again (no reset of the budget). One REQUEST_CHANGES decides at once, as `docs/state.md` says.
6. **A merge failure, a lost review and a member leaving are not events of the round yet**; they come with the item that handles them (7.1, 8.1). `RoundEvent` is a closed enum of this workspace, so adding one breaks the build where a decision is needed.
7. **Outcome rules, where they differ from `bin/agent`:**
   - the first `STATUS:` line wins (as before), but a final message is read as a provider error only for a run that failed and has no status line, and only through its last `tail_lines` lines; the old code read a stale `429` of a clean run as a limit;
   - "rate limit reached" is a rate limit (the old `limit reached` pattern made it a quota); a full context window is no limit;
   - an overloaded model (`at capacity`, `503`, `529`) is `RateLimited`: `RunEnd` has no variant for it; the old *network* pause has no variant either, so a network failure is `NoStatus`;
   - a time of day or a date without a zone (`resets 3pm`, `try again at Nov 12th, 2026 3:04 PM`) is read in `LimitContext::local_offset`, which the runtime passes (a pure function cannot ask the machine); an IANA zone name in parentheses is not converted;
   - the pause ends a minute after the reset time, as before, and is never longer than eight days (`MAX_PAUSE`), the reset time recorded in `RunEnd` is the one the CLI gave.
8. **`after_hang`** (not named in the item) decides resume or mark hung from the watchdog's `hang_strikes` and whether a session exists, as `bin/agent` does; counting hangs in a row is the daemon's.
9. **The fixtures are reconstructed, not captured.** The item asks for "fixtures of real CLI outputs"; running the four CLIs costs the human money and quota (AGENTS.md rule 4). The texts come from the old harness's tests (`tests/unit/limits.bats`, `run_notes.bats`, `authenticated.bats`, the fake CLIs) and the CLIs' documented messages; every file says so in `provenance`. A capture can replace any text; the expected values are then the only thing to recheck.

## Out of scope

- The comment in `config/values.rs` that makes the second check of the item find a line (above).
- `cli::SessionId` and `state::SessionId` are two types with one meaning; `after_hang` takes the adapter's (`cli::SessionId`) because that is what `RunOutput` holds.
- The prompt-level rule "an investigator coordinating weaker models leads them" is not a decision of this item.

## Suggestions for `AGENTS.md`

- "Checks" of Rust items that grep for a pattern should say that a comment counts, or ask for `--` exclusions; this one found a comment.
- Integration-test helpers outside `#[cfg(test)]` trip the lint baseline (`unwrap_used`); the existing tests wrap their code in `#[cfg(test)] mod name { }`. One line in "Tests" would save the discovery.
