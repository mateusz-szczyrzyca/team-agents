# Fixes for the major findings of the review of PR #83 — `inv-claude-sonnet`

Task `20261111-204236-inv-claude-sonnet`, branch `agents/inv-claude-sonnet/20261111-204236`.

The branch is the head of [#83](https://github.com/mateusz-szczyrzyca/team-agents/pull/83)
([`67b3790`](https://github.com/mateusz-szczyrzyca/team-agents/commit/67b3790f44f37abf44a9f847eda2a7d4bed55365),
branch `agents/inv-opus/20261111-200837`) plus `12` commits of fixes, test first: each fix is a $\color{red}{\textbf{RED}}$
commit (its failing run in the message) and a $\color{green}{\textbf{GREEN}}$ commit. The only review is by `inv-kimi`
(request `#20261111-201747-inv-kimi`); its text is data to me, and it asked for nothing beyond the findings below.

## Result per finding

| # | Finding | Status | RED | GREEN |
|---|---|---|---|---|
| 1 | **PR-2** — no up-front split (`4,952` lines) | **not done**, see below | | |
| 2 | **COR** — subscription events cannot maintain the summaries | fixed | [`a2fc007`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a2fc00797971068b5f6f8ae672884da1b63e9825) | [`6e2c1e9`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6e2c1e935ae5d4b45d566eb5fbd54ff9cb5886ce) |
| 3 | **COR** — nested enums accept unknown fields | fixed | [`6f39668`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6f396681b0d840f902c1fcc4095dc556db9fee5b) | [`16130c2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/16130c22c78406b3b69b1e4e2be5a0e997bfca86) |
| 4 | **TYP-2** — whole-value `&mut` accessors | fixed | [`608d2cf`](https://github.com/mateusz-szczyrzyca/team-agents/commit/608d2cfd5a8b18b409cad5500e8b5a4c6ebf7385) | [`f551574`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f55157444a01707fc132cf73439e8c7658304aa1) |
| 5 | **TYP-2** — review and fix-pass payloads | fixed, except one rule (see below) | [`c2b314e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c2b314e8ada97362adec9e4bfe98892d2a6699de) | [`ae56c05`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ae56c05a3d47fdb220a27e97cced20213674ad43) |
| 6 | **TYP-2** — `Failed { exit_code: 0 }` | fixed | [`6a2ee2d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6a2ee2d781312e1e8d33c6cc77e02fb854d56da0) | [`5a4b52f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5a4b52f61cfc53b8e12eab58484dfebc3a401e75) |
| 7 | **COR** — `saturating_add` accepts a duplicate round `u32::MAX` | fixed | [`4357719`](https://github.com/mateusz-szczyrzyca/team-agents/commit/435771942166b152891d4a2e889cedc77f78511b) | [`dd33981`](https://github.com/mateusz-szczyrzyca/team-agents/commit/dd33981e14ebde609d2064a6dd205a9036340ffa) |

## What was done

- **3 — unknown fields.** serde skips an unknown field next to the tag of a *unit* variant of an internally tagged
  enum, even with `deny_unknown_fields`; the attribute alone does not fix it (tried first, the tests stayed red).
  `ItemPhase`, `ReviewOutcome`, `Completion`, `QuestionState`, `RunState`, `RunEnd` and `GateOutcome` are now read
  through a private wire enum whose unit variants have an empty struct body (`Pending {}`), which refuses the field;
  writing is unchanged. `Participant` (adjacently tagged) only needed the attribute. Tests:
  `crates/agent-core/tests/strict_wire.rs`.
- **7 — overflow.** `check_next_question` uses `checked_add`; new cause `QuestionError::RoundsExhausted { last }`.
  The overflow cannot be reached through a state (rounds must count 1, 2, 3, …), so the test is a unit test of the
  helper at `u32::MAX`.
- **6 — exit code.** `GateOutcome::Failed { exit_code: NonZeroI32 }`; `0` is refused when read; negative codes stay.
- **5 — review rounds.** New `AdvanceError` (`Transition(InvalidAdvance)` | `Reviews { item, source: InvalidReviews }`),
  `InvalidReviews` (`ApprovedWithoutReviews`, `ApprovedNotByAll`, `DuplicateReviewer`, `DuplicateTask`,
  `FixPassWithoutReviews`, `FixPassApproved`) and `ItemPhase::check_reviews`. `advance` checks the new phase's reviews
  and the round it leaves for a fix pass; reading checks the phase (`try_from`) and every fix pass (`ItemProgress` has
  a hand-written `Deserialize`, because the existing `TryFrom<ItemProgressWire>` fixes its error type to
  `FixingWithoutFixPass`). `AdvanceError: PartialEq<InvalidAdvance>` keeps the existing table test's comparison
  working. Invariants are in `docs/state.md`.
- **4 — handles.** `State::team_mut` → `TeamMut`; `Team::item_mut` → `ItemMut`; `crew_mut` → `CrewMut`;
  `reviewers_mut` → `Vec<MemberMut>`; `find_mut` / `State::find_task_mut` → `(Role, MemberMut)`. Each dereferences to
  the value (reads) and offers only its checked operations; none gives out `&mut` to the whole value. Six
  compile-fail doctests (on `State::team_mut` and `State::find_task_mut`) try each overwrite; I checked one by one that
  each fails for the right reason (`E0594`, `E0596`, `E0308`) and not for a typo. `ItemMut::set_gate` replaces the pub
  field assignment. Decision 15 in `docs/state.md`.
- **2 — subscription.** Added the event `team_updated { summary }` and `EventKind::apply_to`, the client's reducer
  (replaces the team's summary, or appends it). This is the "emit updated summaries" option of the review; it is
  additive, so no existing event, line or test changes. `team_started` and `item_status` are unchanged and documented
  as "what happened", not as inputs for the summaries. Tests: `crates/agent-core/tests/subscription.rs` (an item
  starts while others are pending and in review — the review's own case; a new team; a task's status; idempotence;
  JSON).

## Checks

- `GATE_WAIT=1 make gate`: PASS at
  [`6e2c1e9`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6e2c1e935ae5d4b45d566eb5fbd54ff9cb5886ce)
  (all steps, including `make rust`: fmt, clippy `-D warnings`, tests, doc).
- `agent-core`: `366` unit tests, `8` integration test binaries (`48` tests), `32` doctests (`9` + `23`), all passing.
- Fixture `state-v1.json` still loads and writes back to the same text (`tests/state.rs`).
- No lint suppression, no `unsafe`, no `#[ignore]` added.
- Diff vs the PR head: `+1,383 / −55` in `12` files, of which `277` are the two new integration test files; the new unit tests and doctests are in the production files.

## Existing test code I had to touch (rule 1) — decide

Two changes of the review need a different type, which makes five lines of existing tests fail to compile. Nothing
else of existing tests changed: no assertion, expected value, JSON or call.

- In [`5a4b52f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5a4b52f61cfc53b8e12eab58484dfebc3a401e75)
  (finding 6): `GateOutcome::Failed { exit_code: 2 }` became `…{ exit_code: NonZeroI32::new(2).unwrap() }` in
  `protocol/event.rs` and `state/item.rs`.
- In [`f551574`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f55157444a01707fc132cf73439e8c7658304aa1)
  (finding 4): four bindings gained `mut` (`let (role, mut found)`, `let (_, mut codex)`) in `state/team.rs` (2),
  `state.rs` and `protocol/reply.rs`, because the operations of a handle take `&mut self`.

Both edits are in the commit of their fix alone, so the merger can drop or rework them. The alternative for finding 6
that touches no test — keep `exit_code: i32` and only refuse `0` when read — leaves `Failed { exit_code: 0 }`
compiling, which is the finding. For finding 4 the alternative is to make the old `&mut` accessors `pub(crate)` and
add new public handle names; I judged two parallel APIs worse than four `mut`.

## Not done

- **Finding 1 (PR-2, split the PR).** A PR's size and how it is cut is the author's and the merger's decision, not a
  code change; I did not re-cut the history of `inv-opus`'s branch. If the PR is split, the three boundaries the review
  names (questions, state, protocol) hold for my commits too: findings 3–7 touch only `state/*` (plus one event line
  in `protocol/event.rs`), finding 2 only `protocol/*`, `tests/subscription.rs` and docs.
- **Finding 5, one rule: a fix pass must hold a `REQUEST_CHANGES` review.** The review also asks that entry to `fixing`
  require that changes were requested. Existing tests build fix passes from rounds with *only pending* reviews
  (`progress_in(PhaseKind::Fixing)`, used by many tests, and
  `each_round_that_asks_for_changes_is_a_fix_pass_in_order`, which asserts such a round is recorded). The rule forbids
  exactly what they do, and rule 1 says to ask the human instead of changing them. What is enforced instead: a fix
  pass has at least one review and is not a round that every reviewer approved (`FixPassWithoutReviews`,
  `FixPassApproved`), on `advance` and on reading. Decision for the human: change those tests to submitted
  `REQUEST_CHANGES` reviews (small) and then tighten `check_fix_pass`; `docs/state.md` marks the gap.
- `protocol-v1-frames.jsonl` has no `team_updated` line: it is an existing pinned fixture and I did not edit it; the
  event's JSON is tested in `tests/subscription.rs`. Add a line when the fixture may change.

## Suggestions for `AGENTS.md` / the notebook

- serde drops an unknown field next to the tag of a unit variant of an internally tagged enum even with
  `deny_unknown_fields`; a Rust rule "state/protocol enums with unit variants are read through a wire enum with empty
  struct bodies" (and the review checklist item "unknown fields in nested enums") would have caught finding 3.
- For tasks that change a type used by existing tests, "Never modify existing tests" and the reviewer's remedies
  collide (findings 4 and 6); a line saying that a compile-only adaptation of a test to a type the task changes is
  allowed, if it is isolated in its own commit and listed, would remove the need to stop.
