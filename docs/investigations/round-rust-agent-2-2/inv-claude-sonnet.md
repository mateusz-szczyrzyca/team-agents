# Round rust-agent/2.2 — findings of inv-claude-sonnet

Item 2.2 of the plan "Rewrite `bin/agent` in Rust": the state, the control protocol and the questions files, in
`agent-core`. Branch `agents/inv-claude-sonnet/20261111-191554`, last code commit
[`7e61d6f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7e61d6f83b62b86fee7bac06f6e9c1b6101a59b1).
No pull request, as instructed; the merger integrates the independent results.

## Result

All three checks of the item hold, and the whole repository gate passes on the last code commit.

- A state saved and loaded back is equal (`state::tests::a_state_saved_and_loaded_back_is_equal`, and
  `tests/state.rs`, where a fixture of schema version 1 loads and writes back to the same text). A state of another
  schema version is refused with `StateError::UnsupportedSchema { found }`, checked before anything else of the file
  (`2`, `0`, `u64::MAX`, and a newer file whose body this build could not parse).
- A request, and a frame, with an unknown protocol version is refused with `ProtocolError::UnsupportedVersion { found }`
  (`tests/protocol.rs`, `protocol::tests`).
- A questions file rendered, answered with `ok` and with own text, and marked ANSWERED parses back to those answers
  (`tests/questions.rs`, with the two shipped templates).

## Checks run

```bash
GATE_WAIT=1 make gate     # PASS at 7e61d6f: all steps; recorded by `agent gate`
cargo test -p agent-core --offline          # 351 unit, 19 new integration, 17 doctests: all pass
cargo clippy -p agent-core --all-targets --offline -- -D warnings   # 0 findings
RUSTDOCFLAGS="-D warnings" cargo doc -p agent-core --no-deps --offline   # 0 warnings
git grep -nE 'std::(fs|process|net)|tokio' -- crates/agent-core/src    # one hit: a comment that already existed
```

| Tests added | Count |
|---|---|
| unit, `questions` | `52` |
| unit, `state` (`values`, `run`, `question`, `item`, `member`, `team`, `State`) | `127` |
| unit, `protocol` (lines, requests, replies, events) | `38` |
| integration, `tests/state.rs`, `tests/protocol.rs`, `tests/questions.rs` | `7`, `7`, `5` |
| doctests (new: `Timestamp`, `SessionId`, `CommitId`, with `compile_fail` for sealed constructors) | `4` |

Every behaviour was seen failing first: each cycle has a $\color{red}{\textbf{RED}}$ commit (bodies that compile and
return a wrong value, the failing counts in the message) and a $\color{green}{\textbf{GREEN}}$ commit. After the last
green I broke `34` conditions one at a time (a `<` to `<=`, a check removed, two fields swapped, a table entry added;
script not committed) and ran the suite: `29` were caught at once and `5` survived. Three were real gaps — the marker
line may not carry other text; the state and a protocol line are read at exactly their limit and refused one byte over —
and got tests, each seen to catch its mutation. One survivor was an edit that changed only a comment (equivalent). The
last, the `year > 9999` half of `Timestamp::from_utc`, cannot be killed: `time` without its `large-dates` feature cannot
hold such a year; the check stays because another crate of the workspace could turn that feature on (features are
additive). `cargo-mutants` is not installed here.

## Self-review before the report

I swept the added lines with the review rules' command and read them against `rust-review.md`. What it found, all fixed
in commits of their own:

- [`353f866`](https://github.com/mateusz-szczyrzyca/team-agents/commit/353f8668dd3e0eadf9c6d26ac4dce5127a106034)
  (RED [`f21b24a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f21b24ac538954e2f37e6d0a6467f66132fff865)):
  `expand` hid a case that cannot happen with `unwrap_or_default()` and let a template that gave no verdict through
  (ERR-2); the two renderings are one now, and the words that accept a recommendation are checked (`ExpandError`).
- [`18739e3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/18739e399a453d06b35d7e12c89ae98c28cdf19e):
  arguments that could be swapped (`Entry::new(number, title, body)`, `Crew::team(coordinator, members, workers)`:
  TYP-3), a `bool` parameter (`mark_answered(bool)`: TYP-4), a label kind and participant names as strings (TYP-4,
  TYP-3), errors that printed their source twice (ERR-7). The calls in my own tests were rewritten mechanically.
- [`7e61d6f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7e61d6f83b62b86fee7bac06f6e9c1b6101a59b1):
  reading a state looked every task up in the teams before it, and `expand` scanned the questions for each answer: both
  were quadratic on inputs of up to 64 MiB and 1 MiB (PERF-3).

## Size (PR-1)

`4,768` changed lines outside tests with the plan's command (limit `5,000`; the planning threshold of PR-2, about
`4,000`, is passed). Tests, the `state/fixtures.rs` builders and the fixtures are not counted. By part: state `2,365`,
protocol `974`, questions `1,004` (with the two templates), documentation `385`, Cargo and the small edits `40`.
`31` files, `+10,042` / `−17` in total.

PR-2 asks for a split planned before coding. I did not split: the protocol names the state's types (`Team`, `Member`,
`GateResult`, `RunEnd`, `Timestamp`), the questions file's verdict is a protocol field, and the human reads one
`docs/state.md` at the checkpoint after feature 2. A split along the same seams is possible if the reviewers want one:
questions (≈1,000), state (≈2,400), protocol (≈1,000), each with its tests; the commits are in that order.

## Commits

| | Commit |
|---|---|
| $\color{red}{\textbf{RED}}$ | [`5250a70`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5250a7023afa96b3e1134d87cc7f0845508e8f58) Questions files: tests first |
| $\color{green}{\textbf{GREEN}}$ | [`5c81c15`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5c81c1596fd4e0d033f9bbb492bff666911d4cf2) Questions files: parsing, rendering and answers |
| $\color{red}{\textbf{RED}}$ | [`dac2bd5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/dac2bd5c89de7f489f637e81ca72601365769753) State values: tests first |
| $\color{green}{\textbf{GREEN}}$ | [`f928464`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f9284642f60faedc267216c71f9da18431b1827c) State values: timestamps, session and commit IDs |
| $\color{red}{\textbf{RED}}$ | [`767fb79`](https://github.com/mateusz-szczyrzyca/team-agents/commit/767fb79343c4eb0aa49412e19621f2e9ec2c7d53) State model: tests first |
| $\color{green}{\textbf{GREEN}}$ | [`8d02514`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8d025143e58933135cce02a51467b19e7816e738) State model: checks, schema version and JSON |
| $\color{red}{\textbf{RED}}$ | [`d84a8e1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d84a8e1038669a380463c89bbe1747d7c92e9c87) Control protocol: tests first |
| $\color{green}{\textbf{GREEN}}$ | [`d6b4e65`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d6b4e65ef1294436b05b3d8c12d1b81067bce6db) Control protocol: reading and writing lines |
| | [`e8b0f00`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e8b0f000e9e9722e9c3b11e7eb11af83347f8431) Document the state, the control protocol and the questions files (`Test-First: not possible`, stated) |
| $\color{red}{\textbf{RED}}$ | [`f21b24a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f21b24ac538954e2f37e6d0a6467f66132fff865) Questions files: words that accept a recommendation, tests first |
| $\color{green}{\textbf{GREEN}}$ | [`353f866`](https://github.com/mateusz-szczyrzyca/team-agents/commit/353f8668dd3e0eadf9c6d26ac4dce5127a106034) Questions files: words that accept a recommendation |
| | [`1a9b4c3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1a9b4c3b25efb6ae8cce94fd4824f614de2f252d) Test the exact limits and the marker line (mutation survivors) |
| | [`18739e3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/18739e399a453d06b35d7e12c89ae98c28cdf19e) Name what could be swapped, give kinds and names their types (`Test-First: not possible`, stated) |
| | [`7e61d6f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7e61d6f83b62b86fee7bac06f6e9c1b6101a59b1) Read a large state and expand a large questions file in linear time (`Test-First: not possible`, stated) |

Tests of earlier commits that I changed in a later commit of this branch are my own new tests, never an existing one;
each such change is named in its commit message (for example: the fixture builder `at` had a bug past 45 minutes,
`ListedMismatch` names the list's `ItemMark`, two assertions on unit variants were dropped because serde ignores a
stray field next to the tag of a unit variant).

## Files

Expected by the item:

- [`crates/agent-core/src/state.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/7e61d6f83b62b86fee7bac06f6e9c1b6101a59b1/crates/agent-core/src/state.rs)
  with its modules in [`state/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/7e61d6f83b62b86fee7bac06f6e9c1b6101a59b1/crates/agent-core/src/state)
  (`values`, `run`, `question`, `item`, `gate`, `member`, `team`; the test builders `fixtures` under `#[cfg(test)]`)
- [`crates/agent-core/src/protocol.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/7e61d6f83b62b86fee7bac06f6e9c1b6101a59b1/crates/agent-core/src/protocol.rs)
  with [`protocol/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/7e61d6f83b62b86fee7bac06f6e9c1b6101a59b1/crates/agent-core/src/protocol)
  (`request`, `reply`, `event`)
- [`crates/agent-core/src/questions.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/7e61d6f83b62b86fee7bac06f6e9c1b6101a59b1/crates/agent-core/src/questions.rs)
- [`docs/state.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/7e61d6f83b62b86fee7bac06f6e9c1b6101a59b1/docs/state.md)

Outside the list, each with its reason:

- `prompts/harness/questions/instructions.md`, `accepted.md` — "the instruction text comes from a template" (item), and
  no prompt text in `.rs` files. The words that tell the agent a recommendation was accepted are a template too. Nothing
  renders them yet (the renderer is item 2.3): the code takes the text as an argument, and `tests/questions.rs` fills
  the one variable of each by hand to prove they fit the format.
- `crates/agent-core/tests/{state,protocol,questions}.rs` and `tests/fixtures/{state-v1.json,protocol-v1-requests.jsonl,protocol-v1-frames.jsonl}`
  — the checks of the item through the public API, and the formats of version 1 as golden files that must keep loading.
- `Cargo.toml`, `crates/agent-core/Cargo.toml`, `Cargo.lock` — see *Dependencies*.
- `crates/agent-core/src/config.rs`, `config/values.rs` — the state and the protocol write a repository name, a
  Tailscale login and a path, which the configuration values (deserialize-only until now) could not. The `text_value!`
  macro and `AbsolutePath` gain `Serialize` (the stored text, which parses back to an equal value), and `check_text` and
  `refuse` become crate-visible for `SessionId`. No existing test or behaviour changed.
- `crates/agent-core/src/lib.rs`, `docs/architecture.md`, `README.md` — the modules in the vocabulary tables, and the
  README paragraph on the Rust rewrite names the new contracts, as the item 2.1 PR did for its own.

## Dependencies

No new crate was added. Two changes of declarations, both for approved crates:

- `time` gets the features `formatting` and `parsing` (RFC 3339 times in the state and the protocol). `Cargo.lock` gains
  `time-macros 0.2.32`, which `time` pulls in with those features (it needs `num-conv`, already locked).
- `serde_json` moves from `[dev-dependencies]` to `[dependencies]` of `agent-core`: states and lines are JSON. It does no
  I/O; `docs/architecture.md` says so.

No lint suppression (`#[allow]`, `#[expect]`) was added. No existing test was changed.

## Decisions for the human (checkpoint after feature 2)

[`docs/state.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/7e61d6f83b62b86fee7bac06f6e9c1b6101a59b1/docs/state.md)
ends with the thirteen choices that are expensive to change once state files exist. The ones I would look at first:

1. **A solo task is a team of one** (role `solo`), so one record and one code path serve both.
2. **Reviewers are tasks of the team they review** (`reviewers[]`, each a `Member` with runs), not a separate registry.
3. **The questions and answers are not in the state**, only that they were asked and answered, when and through which
   channel. The questions file is the one source.
4. **`RunEnd` lists what ends a run** (reported status, no status, quota with a reset time, rate limit, hung, stopped).
   Item 3.1 classifies into it; a new ending is a schema change.
5. **The phase `fixing`** keeps the pull request across a fix pass; fix passes are the records of the rounds that asked
   for changes (their number is compared with `[review] fix_passes`).
6. **`run` carries `repo` and `checkout`** (the client's repository root); the base branch and the gate come from the
   configuration by repository name. The plan did not say how the daemon learns the checkout.
7. **`answer` carries verdicts, not a file**; a saved file with the marker is read by the daemon with the same code.
8. **A task list is stored as its text** and parsed again when read: a stricter task-list format later would make old
   states unreadable (a schema change with a migration).

## Not done / open

- Nothing renders the two templates yet (item 2.3's renderer); `QuestionsFile::set_instructions` and `expand` take the
  text, and the runtime (7.2) will fill the templates and pass it in.
- The state has no overall team status: it is derived (`TeamSummary` shows the parts). Item 3.1 may want a function.
- A worker may not work alone (`CrewError::WrongTier`), and a team needs a member besides its coordinator; both follow the
  plan's "Behaviour in short" and `Tier`'s documentation. If item 3.1 or 7.1 reads them differently, the check is one
  line in `Shape::check`.
- `agent-core`'s `config` `values` are deserialize-and-serialize now; `Secret` still has no `Serialize`, on purpose.

## For `AGENTS.md` (not changed, as the rules say)

- The "Rust" section could say where the developer and review rules are: they are not tracked yet (item 2.3 adds them),
  and the review file is named `rust-review.md` in the human's checkout, not `code-review.md`. I read them there — the
  main checkout, not an agent's worktree — as the previous round did.
- `make gate` prints the whole Rust test output on the host used here (the step lines are fine; the `rust` step's
  output is long). That is a tool matter, not of this change.
