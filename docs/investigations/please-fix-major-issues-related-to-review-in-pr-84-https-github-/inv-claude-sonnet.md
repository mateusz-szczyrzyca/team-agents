# Fixes for the major findings of the review of PR #84

Task `20261111-221854-inv-claude-sonnet`, branch `agents/inv-claude-sonnet/20261111-221854`.

The task text ends mid-word (`... - in the sa`). I read it as "in the same PR": the fixes sit on top of the head of
[#84](https://github.com/mateusz-szczyrzyca/team-agents/pull/84)
([`5c0e6a4`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5c0e6a4fe93c649f41c02c72220cd774201ff385)),
which I merged into the branch first, so the merger can take them as one stack. Source of the findings: the review by
`inv-astra` on that head (`0` critical, `5` major, `0` minor).

## Findings and fixes

| # | Finding | Fix | Commits |
|---|---|---|---|
| `M1` | `remote = "Self"` makes `Type::deserialize` public and unchecked | Private `remote` mirror types; only the `Deserialize` trait reads a value | $\color{red}{\textbf{RED}}$ [`5130a38`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5130a381e4ed578961a2310e52c38f31ed9969c1) / $\color{green}{\textbf{GREEN}}$ [`1e8a416`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1e8a416bbf648563605ac4c2bf9ac54d89be60a2) |
| `M2` | A pending review cannot receive its outcome | `ItemProgress::submit_review` / `ItemMut::submit_review`, typed `SubmitReviewError` | $\color{red}{\textbf{RED}}$ [`39a1f9c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/39a1f9c128b383653ec6cfda2ceece0e28beaf05) / $\color{green}{\textbf{GREEN}}$ [`ebd60e5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ebd60e511e25c9ca80542503244aea1b79702256) |
| `M3` | `advance` to `Done { Listed }` saves but cannot be loaded | `AdvanceError::ListedCompletion`; no mutation | same pair as `M2` |
| `M4` | `Client::Cli` ignores unknown fields | `Client` is read through a private wire enum with `Cli {}` | $\color{red}{\textbf{RED}}$ [`ba8a6d7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ba8a6d71c5e7a7f65f494754f701532b5aefc28c) / $\color{green}{\textbf{GREEN}}$ [`c8f6c79`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c8f6c79aead2298c0ec80ce9e94e89ecfff8aa33) |
| `M5` | `_ => &[]` over the workspace enum `RunEnd` | The empty-field arm lists `Reported`, `NoStatus`, `RateLimited`, `Hung`, `Stopped` | inside the `M1` GREEN; `Test-First: not possible`, no behaviour changes |

Also [`997a367`](https://github.com/mateusz-szczyrzyca/team-agents/commit/997a36783254b074212d9e6916f952aee939afc6):
a clippy fix (`needless_pass_by_value`) in the helper of the new `M4` test.

### M1 — how

serde gives the inherent `deserialize` that `remote` generates the visibility of the type it is derived on. So the
derive moved to a private mirror (`TeamWire`, `MemberWire`, `RunWire`, `QuestionWire`, `ItemProgressWire`,
`ItemPhaseWire`, `ReviewOutcomeWire`, `CompletionWire`, `GateOutcomeWire`, `QuestionStateWire`, `RunStateWire`,
`RunEndWire`), next to its type in the same module. The `checked!` and `strict_enum!` macros take the mirror's name and
call its function; `Serialize` is an ordinary derive on the public type again (`serialize_self!` is gone). A mirror that
differs from its type in a field or variant does not compile, so the two cannot drift silently. The tests call
`T::deserialize` directly, like an external crate, for all twelve types.

### M2 — how

`submit_review(task, verdict, at)` finds the round's review by the reviewer's `task` and turns `pending` into
`submitted`. `SubmitReviewError`: `NotInReview`, `UnknownReview`, `AlreadySubmitted`; every error leaves the item
unchanged. The tests walk the fixture's item `1.2` pending → submitted `REQUEST_CHANGES` → `Fixing` (recorded as a
second fix pass), and pending → `APPROVE` → `Approved`, saving and loading the state after each.

### M3 — how

`advance` refuses `ItemPhase::Done { completion: Completion::Listed }` after the table check, with
`AdvanceError::ListedCompletion { item }`. `Listed` stays what `ItemProgress::new` gives an item the task list marks
`[x]`. The test runs `1.2` through `Approved` and shows the refusal leaves the phase alone, then a merged completion
and a save/load round trip.

## Checks

- `GATE_WAIT=1 make gate`: **PASS** at [`997a367`](https://github.com/mateusz-szczyrzyca/team-agents/commit/997a36783254b074212d9e6916f952aee939afc6) (Rust gate: fmt, clippy `-D warnings`, tests, doctests, rustdoc).
- `cargo test --workspace --locked`: `366` unit tests, `8` `item_lifecycle`, `4` `public_client`, `8` `public_deserialize`,
  the existing integration tests and `23` compile-fail doctests passed.
- New tests: `20` in three new files under `crates/agent-core/tests/`. `0` existing tests changed, `0` skipped, `0` lint suppressions.

## Compatibility

- The state and protocol formats are unchanged: `state-v1.json` and the protocol fixtures still load and write back to
  the same text (their tests pass). The Rust contract is not deployed yet.
- API additions only: `SubmitReviewError`, `ItemProgress::submit_review`, `ItemMut::submit_review`,
  `AdvanceError::ListedCompletion`. `Type::deserialize` as an inherent function is gone from the checked types (callers
  use `Deserialize`, which is what the validating path was already).
- `AdvanceError` gained a variant; no code in the workspace matches it exhaustively.

## Not done / for the reviewer

- The review also asks to restore the assertion that refused a CLI client with a dashboard `login` in the existing test
  `a_client_is_the_cli_or_the_dashboard_with_the_login_it_saw` (`request.rs`). `AGENTS.md` rule 1 forbids changing existing tests, so the same
  assertion lives in the new `public_client::a_request_from_the_cli_with_a_dashboard_login_is_refused` and
  `a_direct_deserialize_of_a_client_refuses_an_unknown_field`.
- `submit_review` does not check `at` against `requested_at`; existing reads do not either, and a stricter read is a
  schema change (`docs/state.md`, "Reading and its rules").
- PR size: see "Pass 2" below.
- Suggestion for `AGENTS.md` / `rust-developer.md` (not changed): `#[serde(remote = "Self")]` on a public type is a
  TYP-2 violation; say so in the rules.

## Pass 2 — the review of PR #85 by `inv-opus` and `inv-astra`

| # | Finding | Fix |
|---|---|---|
| `N1` / `M1` | `advance` from `in_review` to `approved` accepts any `pr` and `reviews` | `ItemMut::advance` refuses an approval that is not the recorded round: `AdvanceError::RoundNotKept`. $\color{red}{\textbf{RED}}$ [`afc42cd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/afc42cd15ad14ae59e3cc04285e693afad8d2472) / $\color{green}{\textbf{GREEN}}$ [`018f9ad`](https://github.com/mateusz-szczyrzyca/team-agents/commit/018f9adca5fe53dc61b14fdb8e0efc9c0da26461), `tests/approval_round.rs` (`5` tests: dropped review, invented reviewer, other pull request, claimed outcome of a pending review, the recorded round accepted) |
| `N3` / `M3` | The twelve private `*Wire` types have no docs on fields and variants | Every field and variant points at the public type it mirrors (`DOC-2`), as `ClientWire` does |
| `N4` / `M4` | The PR description is only the script's text | Rewritten with the `PR-9` sections; the script's blocks are kept |
| `N2` / `M2` | The PR is over the `PR-1` limit against `main` | The PR is re-targeted to #84's branch, see below |
| `N5` | Loose assertions | See below |

### N1 — how

The check is in `ItemMut::advance`, the handle through which a caller outside the crate changes an item: an approval of
an item in review must equal the item's own `pr` and `reviews`. The outcomes reach the round through `submit_review`
only, so a forged approval of a pending review (same reviewer, outcome set by the caller) is refused as well, and a
round that does not approve is refused by the existing check of `Approved`. The check is not in
`ItemProgress::advance`: the existing unit tests of that method approve from a round whose reviews are pending, and
`AGENTS.md` rule 1 forbids changing them. `ItemProgress` cannot be written back by a caller (a clone is detached), so
the public mutation path is closed; if the human wants the same strictness in `ItemProgress::advance`, those unit
tests (`an_item_whose_reviews_all_approved_is_approved`, the helpers `progress_in` / `phase_of`) must change first.

The RED commit's test that invents a reviewer used a task id that is not a valid time, so it failed in parsing instead of in
the check; the GREEN commit corrects the id (my own test, one commit old).

### N2 — size

`bash` with the `PR-1` command, `docs/investigations/` excluded:

- against `main`: `5,503` lines, because the PR contains all of #84 (`4,974` at its head by its own count);
- against #84's head `5c0e6a4`: `645` lines (the fix pass, docs of the wire types included).

#84's branch is an ancestor of this one, so the PR is re-targeted to `agents/inv-kimi/20261111-205632`: it then holds
the fix pass only, `645` lines, and #84 merges first (GitHub re-targets #85 to `main` when it does). No test was removed
or shortened.

### N5

The two assertions in `item_lifecycle.rs` that use `any(…)` / `matches!` belong to a file added in the first pass; the
new `approval_round.rs` compares whole values with `assert_eq!`. I did not rewrite the first pass's assertions here.

### Not done

- `make gate` here runs the Rust gate only: `PASS` at [`f712cc1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f712cc1f1ea148d11a2d09affee85d1e3906b979). The `unit-bats` failure of `inv-opus` and `inv-astra`
  (investigation tests that time out) is on `main`, outside this change; the human should look at `main`.
