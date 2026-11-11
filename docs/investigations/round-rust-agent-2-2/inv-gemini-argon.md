# Investigation Report — Round `rust-agent/2.2` (`inv-gemini-argon`)

- Investigator: `inv-gemini-argon` (`gemini/gemini-4-argon-high`)
- Task: `20261111-191554-inv-gemini-argon`
- Branch: `agents/inv-gemini-argon/20261111-191554`
- Base commit: [`26b65de`](https://github.com/mateusz-szczyrzyca/team-agents/commit/26b65de7e4076596768ca96b48a7f94799c93874)
- Code head commit: [`da76f9e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/da76f9e89c49b21c89a58c3cdf18afa4b57e75af)

## Summary

Implemented item `2.2` of the plan "Rewrite `bin/agent` in Rust": the persisted daemon state model, the Unix domain socket JSON-line wire protocol (`Request`, `Response`, `Event`), and the `questions-N.md` Markdown parser and renderer in `agent-core`, together with [`docs/state.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/docs/state.md) and [`rounds/round-2.2.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/rounds/round-2.2.md).

## Task

> **2.2** State model, daemon socket protocol (`Request`, `Response`, `Event`) and questions file parser and renderer
>
> *Files:* `crates/agent-core/src/{state,protocol,questions}.rs`, `docs/state.md`.
> *Adds:* `State`, `Team`, `Member`, `ItemProgress`, `Run`, `Question`, `Review`, `GateResult` with a schema version; `Request`, `Response`, `Event` with a protocol version, covering JSON-line requests and responses for `run`, `status`, `show`, `answer`, `stop`, `pause`, `unpause`, `resume`, `say`, `gate` and `subscribe` (state events for the dashboard); `QuestionsFile` parsing and rendering: questions, options, recommendation, `Verdict` per question (`ok` accepts the recommendation, or own text), `ANSWERED` marker on the last line, and instruction text injected from a template.
> *Checks:*
> 1. A state saved and loaded back is equal; a state with another schema version is refused with a typed error.
> 2. A request with an unknown protocol version is refused with a typed error.
> 3. A questions file rendered, answered with `ok` and with own text, and marked `ANSWERED` parses back to those answers.

## Files Created and Modified

- [`crates/agent-core/src/questions.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/questions.rs) — `QuestionsFile`, `QuestionEntry`, `QuestionOption`, `OptionLabel`, `Verdict`, `OwnAnswer`, `QuestionAnswer`, `OpenQuestion`, `QuestionsRenderTemplate`, `InvalidOwnAnswer`, and `QuestionsError`, plus `verdict_map` for `NonZeroU32`-keyed verdict maps across direct and flattened `serde` representations.
- [`crates/agent-core/src/state.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/state.rs) — `SCHEMA_VERSION` (`1`), `State`, `Team`, `TeamMode`, `AutopilotState`, `Member`, `MemberRole`, `ItemProgress`, `Run`, `Question`, `QuestionState`, `QuestionPresentation`, `PendingAnswer`, `RecordedAnswer`, `Review`, `ReviewFinding`, `ReviewSeverity`, `GateResult`, `GateStepResult`, `CommitSha`, `Timestamp`, `InvalidTimestamp`, `InvalidCommitSha`, and `StateError`.
- [`crates/agent-core/src/protocol.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/protocol.rs) — `PROTOCOL_VERSION` (`1`), `TargetId`, `Request` (`11` command variants: `Run`, `Status`, `Show`, `Answer`, `Stop`, `Pause`, `Unpause`, `Resume`, `Say`, `Gate`, `Subscribe`), `Response` (`12` variants including `Error(ProtocolFailure)`), `Event` (`14` state-event variants for `subscribe` clients and `agent-web`), and `ProtocolError`.
- [`crates/agent-core/src/lib.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/lib.rs) — exports `protocol`, `questions`, and `state` modules (required so callers and tests can import the new modules).
- [`crates/agent-core/src/config/values.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/config/values.rs) — derives `PartialOrd` and `Ord` on [`AbsolutePath`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/config/values.rs#L317) for deterministic ordering in state records.
- [`crates/agent-core/Cargo.toml`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/Cargo.toml) — links workspace dependency `time` for `Timestamp` (ISO-8601 UTC `YYYY-MM-DDTHH:MM:SSZ`).
- [`docs/state.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/docs/state.md) — reference documentation for `SCHEMA_VERSION`, `state.json` invariants, `PROTOCOL_VERSION`, JSON-line messages, and `questions-N.md` syntax.
- [`rounds/round-2.2.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/rounds/round-2.2.md) — round description file for round `rust-agent/2.2`.
- [`docs/architecture.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/docs/architecture.md) — adds `state.rs`, `protocol.rs`, and `questions.rs` to the `agent-core` module table.
- [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/README.md) — documents the round `2.2` modules in the Rust workspace section.

## TDD Commit History

- [`30c1029`](https://github.com/mateusz-szczyrzyca/team-agents/commit/30c10291d52ac18aeb2bf028bffb8e258a31f586) — $\color{red}{\textbf{RED}}$: `Questions file parser and renderer: tests first (red)` (`18` failing unit tests in [`crates/agent-core/src/questions.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/questions.rs)).
- [`dfb39ae`](https://github.com/mateusz-szczyrzyca/team-agents/commit/dfb39aed207435703f36fa1f35e1098ce6a1fd64) — $\color{green}{\textbf{GREEN}}$: `Questions file parser and renderer (green)` (`18/18` unit tests passing in [`crates/agent-core/src/questions.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/questions.rs)).
- [`d69ebc6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d69ebc6887c2e8f15391865c61fc4ed81e9b4c10) — $\color{red}{\textbf{RED}}$: `Persisted state model and schema validation: tests first (red)` (`20` failing unit tests in [`crates/agent-core/src/state.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/state.rs)).
- [`2fd4967`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2fd4967efdce87d1677371411ef2b51a6fe2e525) — $\color{green}{\textbf{GREEN}}$: `Persisted state model and schema validation (green)` (`20/20` unit tests passing in [`crates/agent-core/src/state.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/state.rs)).
- [`e31c722`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e31c72265276c649e76fec677c370ec151652a7f) — $\color{red}{\textbf{RED}}$: `Daemon socket protocol (Request, Response, Event): tests first (red)` (`12` failing unit tests in [`crates/agent-core/src/protocol.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/protocol.rs)).
- [`044f2eb`](https://github.com/mateusz-szczyrzyca/team-agents/commit/044f2ebfc260c03412068770444b3c5aa8dd075b) — $\color{green}{\textbf{GREEN}}$: `Daemon socket protocol (Request, Response, Event) (green)` (`12/12` unit tests passing in [`crates/agent-core/src/protocol.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/protocol.rs)).
- [`d7dab8a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d7dab8a2522c2b0efd92f7fe8e9e225c83bba959) — Documentation: `Document state, socket protocol, questions files, and round 2.2`.
- [`da76f9e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/da76f9e89c49b21c89a58c3cdf18afa4b57e75af) — Refactor: `Return StateError from state JSON serializers (refactor)`.

## Design Notes

- **Cross-entity state validation at the deserialization boundary (`DES-2`, `TYP-5`):** [`State`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/state.rs#L1026) deserializes via `#[serde(try_from = "RawState", into = "RawState")]` and validates `schema_version == SCHEMA_VERSION` (`1`) before verifying that every `TeamId` and `TaskId` is unique, every member's `Run`, `ItemProgress`, `Question`, `Review`, and `GateResult` references existing run/item numbers, and every `current_run` or `current_item` pointer targets a declared entry.
- **Typestate / closed-enum representations (`TYP-2`):**
  - [`TeamMode`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/state.rs#L938) (`Interactive | Autopilot(AutopilotState)`) attaches autopilot round counters only when autopilot is enabled.
  - [`MemberRole`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/state.rs#L803) (`Standalone | TeamMember { agent } | Investigator { investigator }`) ties role-specific agent/investigator identifiers to their role variant.
  - [`QuestionState`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/state.rs#L492) (`Open(QuestionPresentation) | Answered(RecordedAnswer) | TimedOut { timed_out_at, presentation }`) prevents answered questions from lacking an answer timestamp/verdict map or open questions from carrying answers.
- **Versioned single-line JSON wire frames (`DES-2`, `ERR-1`):** [`Request`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/protocol.rs#L424), [`Response`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/protocol.rs#L597), and [`Event`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/protocol.rs#L696) inspect `protocol_version` against `PROTOCOL_VERSION` (`1`) before parsing payload variants, so a future client or daemon speaking version `2` receives `ProtocolError::UnsupportedProtocolVersion { found, expected: 1 }` even if version `2` introduces new `op`, `kind`, or `event` names.
- **Newline-free `OwnAnswer` (`TYP-1`):** [`OwnAnswer`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/questions.rs#L73) rejects empty/whitespace-only text, `ok` in any ASCII case (which is [`Verdict::AcceptRecommended`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/questions.rs#L139)), and newlines (`\n` / `\r`), guaranteeing that rendering a [`QuestionsFile`](https://github.com/mateusz-szczyrzyca/team-agents/blob/da76f9e89c49b21c89a58c3cdf18afa4b57e75af/crates/agent-core/src/questions.rs#L350) never injects structural Markdown lines.

## Checks

- `PR-1` non-test changed line count: `3,172` lines (limit: `5,000` lines; split threshold: `4,000` lines).
- Total diff stat (`git diff --shortstat 26b65de...da76f9e`): `10` files changed (`+4,863` / `−21`).
- Item `2.2` check 1 (`cargo test -p agent-core state::tests:: -- --locked`): `20/20` tests pass (`state_round_trips_through_json_and_lookup_helpers_work`, `state_with_unknown_schema_version_is_refused_with_typed_error`, and invariant checks).
- Item `2.2` check 2 (`cargo test -p agent-core protocol::tests:: -- --locked`): `12/12` tests pass (`request_with_unknown_protocol_version_is_refused_with_typed_error`, `all_request_variants_round_trip_as_json_lines`, `all_response_variants_round_trip_as_json_lines`, `lifecycle_event_variants_round_trip_as_json_lines`, `artifact_event_variants_round_trip_as_json_lines`).
- Item `2.2` check 3 (`cargo test -p agent-core questions::tests:: -- --locked`): `18/18` tests pass (`rendered_questions_file_answered_with_ok_and_own_text_and_marked_answered_parses_back_to_those_answers` and structural validation tests).
- Workspace Rust test suite (`cargo test --workspace --all-features --locked`): `178/178` unit and integration tests pass (`160` in `agent-core`, `6` in `agent-core` error source integration tests, `7` in `agent`, `7` in `agent-web`) plus `29/29` doc-tests (`12` run, `17` `compile_fail`).
- Full repository gate (`GATE_WAIT=1 make gate`): `PASS` at [`da76f9e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/da76f9e89c49b21c89a58c3cdf18afa4b57e75af) with `0` lint findings across `shellcheck`, `gofmt`, `golangci-lint`, `staticcheck`, `doccheck`, `errgen`, `unit-go`, `unit-bats`, and `rust`.
- Lint suppressions added: `0`.

## Skipped tests

- `0` skipped tests.

## Not done

- Nothing from item `2.2` was left undone. Socket I/O, atomic `state.json` persistence, and daemon handler loops belong to feature `4` (`agent-runtime`) as planned.
