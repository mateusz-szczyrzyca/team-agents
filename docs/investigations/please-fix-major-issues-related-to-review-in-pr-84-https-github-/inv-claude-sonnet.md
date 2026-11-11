# Fixes for the major findings of the review of PR #84

Task `20261111-221854-inv-claude-sonnet`, branch `agents/inv-claude-sonnet/20261111-221854`, PR
[#86](https://github.com/mateusz-szczyrzyca/team-agents/pull/86) (its description has the full report: what, tests,
gate, compatibility, size). The task text ends mid-word (`... - in the sa`); I read it as "in the same PR", so the fixes
sit on top of the head of [#84](https://github.com/mateusz-szczyrzyca/team-agents/pull/84).

## Findings and commits

- `M1` public unchecked `T::deserialize`: private `*Wire` mirrors. RED `5130a38`, GREEN `1e8a416`.
- `M2` a pending review cannot receive its outcome: `submit_review`. RED `39a1f9c`, GREEN `ebd60e5`.
- `M3` `advance` to `Done { Listed }` saves an unreadable state: `AdvanceError::ListedCompletion`. Same pair as `M2`.
- `M4` `Client::Cli` ignores unknown fields: private `ClientWire`. RED `ba8a6d7`, GREEN `c8f6c79`.
- `M5` `_ =>` over `RunEnd`: the variants are listed. Inside the `M1` GREEN (`Test-First: not possible`, no behaviour change).
- `N1` approval with another round: `AdvanceError::RoundNotKept`. RED `afc42cd`, GREEN `018f9ad`.
- `N3` docs of the private `*Wire` types. `f712cc1`.
- Pass 3, `M3` the reviewed pull request replaced on `InReview -> Fixing` and `Approved -> Done { Merged }`: the same
  `RoundNotKept`. RED `4d07d44`, GREEN `fe3627d`.
- Pass 3, `m1`/`m2` loose assertions of `item_lifecycle.rs`: whole-value `assert_eq!`. `58d4e50`.

## Not done

- `unit-bats` fails (`8` investigation tests time out) on `main` (`3655000`), outside this change: the human should look at `main`.
- `ItemProgress::advance` stays lenient for a detached value; the checks are in `ItemMut::advance`, because existing unit tests
  of the former approve from pending reviews and rule 1 forbids changing them.
- The assertion the first review wanted restored in `request.rs` lives in the new `public_client` tests (rule 1).
- Suggestion for `rust-developer.md` (not changed): `#[serde(remote = "Self")]` on a public type is a TYP-2 violation.
