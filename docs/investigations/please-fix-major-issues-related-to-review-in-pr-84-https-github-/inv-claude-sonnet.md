# Fix of the major findings of the review of PR #84 — `inv-claude-sonnet`

The branch `agents/inv-claude-sonnet/20261111-235405` is [`5c0e6a4`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5c0e6a4fe93c649f41c02c72220cd774201ff385)
(the head of [#84](https://github.com/mateusz-szczyrzyca/team-agents/pull/84)) plus `6` commits, so it fast-forwards the
PR's branch `agents/inv-kimi/20261111-205632`. No pull request was opened.

## Findings

| # | Finding | Result | Commits |
|---|---|---|---|
| `M1` | public unchecked `Type::deserialize` | **Fixed** | $\color{red}{\textbf{RED}}$ [`bdea9ea`](https://github.com/mateusz-szczyrzyca/team-agents/commit/bdea9eac49b0f5312b4b49abafd21181db3ffe46) / $\color{green}{\textbf{GREEN}}$ [`8b39391`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8b3939143fce7be5acacfbccaef9aef438d3f2f2) |
| `M2` | a pending review cannot receive its outcome | **Fixed**: `submit_review` on `ItemProgress` and `ItemMut` | $\color{red}{\textbf{RED}}$ [`bdea9ea`](https://github.com/mateusz-szczyrzyca/team-agents/commit/bdea9eac49b0f5312b4b49abafd21181db3ffe46) / $\color{green}{\textbf{GREEN}}$ [`b081fd6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b081fd686312769c94227b19c24cc37be72c6585) |
| `M3` | `advance` accepts `Done { Listed }` | **Fixed**: `AdvanceError::ListedCompletion` | $\color{red}{\textbf{RED}}$ [`bdea9ea`](https://github.com/mateusz-szczyrzyca/team-agents/commit/bdea9eac49b0f5312b4b49abafd21181db3ffe46) / $\color{green}{\textbf{GREEN}}$ [`b081fd6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b081fd686312769c94227b19c24cc37be72c6585) |
| `M4` | `Client::Cli` ignores unknown fields | **Fixed** through `strict_enum!` | $\color{red}{\textbf{RED}}$ [`bdea9ea`](https://github.com/mateusz-szczyrzyca/team-agents/commit/bdea9eac49b0f5312b4b49abafd21181db3ffe46) / $\color{green}{\textbf{GREEN}}$ [`8fda77f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8fda77f17136b80f7cd9ded9c09fb9fb7a6bcb85) |
| `M5` | `_ =>` over `RunEnd` | **Fixed**; `Test-First: not possible` (exhaustive match, no behaviour) | [`8b9a88e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8b9a88e8f24738af0fc7acdc72142f8930523601) |

Documentation of the new behaviour: [`a5bf4f2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a5bf4f2fe858c397cbd4a6186e65887dd200bb42) (`docs/state.md`).

## How

- **`M1`.** serde's `remote = "Self"` generates a public inherent `deserialize` that wins over the trait function.
  `checked!` and `strict_enum!` in `state/strict.rs` now take the definition itself and emit it twice: the public type
  derives only `Serialize`; a private copy inside the `Deserialize` implementation carries the raw derive through
  `remote`. Wire format and checks are unchanged (`state-v1.json` and the frames fixture still pass). Applies to
  `Team`, `Member`, `Run`, `Question`, `ItemProgress`, `GateOutcome`, `ReviewOutcome`, `Completion`, `ItemPhase`,
  `RunState`, `RunEnd`, `QuestionState`.
- **`M2`.** `submit_review(reviewer, verdict, at)` records the outcome of the reviewer's pending review of an item in
  review. Errors (`SubmitReviewError`): `NotInReview`, `UnknownReviewer`, `AlreadySubmitted`; nothing changes on error.
  The test walks the fixture's approval-plus-pending round: pending → rejection → `Fixing` with the fix pass recorded →
  state round trip.
- **`M3`.** `listed` stays reserved for `ItemProgress::new`; the test goes `Approved` → `Listed` (refused, phase
  unchanged) → `Merged` and round-trips the state.
- **`M4`.** `Client` is read by `strict_enum!`, so a field beside `{"kind":"cli"}` is refused, as is a `login` on `cli`.

## Checks

- `make gate` at `a5bf4f2`: **PASS** (Rust gate included: fmt, clippy `-D warnings`, tests, rustdoc).
- `cargo test -p agent-core`: `366` unit, all integration tests, `9` doctests and `23` compile-fail doctests pass; `14`
  new tests (`public_api` `6`, `review_lifecycle` `5`, `cli_client` `3`), each seen failing on an assertion in the RED
  commit.

## For the reviewer

- **Lint suppression:** `#[expect(unreachable_pub, reason = "…")]` on the private copy inside `checked!` and
  `strict_enum!` in [`state/strict.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a5bf4f2fe858c397cbd4a6186e65887dd200bb42/crates/agent-core/src/state/strict.rs)
  (two sites, one per macro): the copy repeats the definition, `pub` included.
- **Size (PR-1):** `5,090` counted lines against `main` (the PR's `4,974` + `116`), `90` over the relaxed `5,000`
  ceiling (1.8 %). The fixes cannot be a smaller PR: the task puts them into #84. `M2` is new code (`~70` lines with its
  error type and docs); the macro change is nearly neutral. A deviation to record in the PR description.
- **Removed CLI assertion (`d6b4e65`):** not restored in the existing test (rule 1); `cli_client.rs` covers it (a
  `cli` client with a dashboard `login`).
- **Not done:** the reviewer's mutation check and external-crate probes were not repeated beyond the new tests.
- **Suggested for `AGENTS.md`:** note that serde `remote = "Self"` leaks a public `deserialize`; use `checked!` /
  `strict_enum!`.
