# PR #81 — the major review findings (`M1`, `M2`)

Investigator: `inv-claude-sonnet`. Source: the review by `inv-astra` of
[#81](https://github.com/mateusz-szczyrzyca/team-agents/pull/81), head `5f432ea`. Work is on the branch
`agents/inv-claude-sonnet/20261111-183446`, which starts from that head (fast-forward from `main`), so the commits
below sit on top of the PR's own.

## What was done

### `M1` — the error-source contract

- `crates/agent-core/tests/task_list_error_sources.rs` (new, `5` tests) walks `Error::source()` through the public API
  and downcasts:
  - `TaskListError::FeatureNumber` → `NumberProblem::Shape`, and `NumberProblem::Range` → `ParseIntError`
    (`IntErrorKind::Zero` for `0`, `PosOverflow` for `65536`).
  - `TaskListError::MalformedDependsOn` → the same two, for a bad number after `Depends on:`.
  - An empty `Depends on:` has `source() == None` (there is no word to blame).
- Mutation check (the "demonstrate failure with the links removed" the review asks for):
  - Both `#[source]` on `FeatureNumber` and `MalformedDependsOn` removed: `4/5` new tests fail (the existing `169`
    still pass, as the review found).
  - `#[source]` on `NumberProblem::Range` removed: `2/5` fail.
  - Both restored; `git diff` of `task_list.rs` against the PR head is empty.
- **Against the pre-fix commit** [`d56b516`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d56b5160f9ff32f8b933ff1f73cc916f236b4996)`^`
  the tests cannot be run: `NumberProblem` does not exist there (`parse_number` returned an `Option`), so they do not
  compile. That is stated in the commit message rather than faked.
- **The historical test-first gap is accounted for, not rewritten.** The commit is pushed history of #81 and is not
  amended. The new commit carries `Test-First: not possible — <reason>`: the behaviour already exists and is correct,
  so a RED run against the current code is impossible; the mutation check stands in for it.

### `M2` — the README entry

- `README.md`, in the existing "The Rust rewrite" paragraph: what the two contracts are (`agents.toml` / `secrets.toml`
  as `Config` / `Secrets`, and `TaskList::parse`), links to `docs/configuration.md`, `docs/task-list.md` and
  `config/agents.example.toml`, and the statement that the installation does not read either yet (runtime items `4.1`
  and later).

## For the PR description (I may not edit it)

Replace the blanket test-first claim with:

> Four cycles (`config values`, `agents.toml`, `secrets.toml`, task list parser) have RED and GREEN commits.
> `d56b516` (cause of a bad feature number kept as the error source) changed behaviour and adjusted the tests of
> those two errors in the same commit, without a RED commit and without a `Test-First: not possible` line. It is
> accepted as an exception (self-review finding, found while the parser was written); the regression tests for the
> source chain came afterwards in `task_list_error_sources.rs` with a mutation check, because they cannot be red on
> code that is already right.

## Not done

- `N1` (minor: `dependencies × preceding_features` scan in `TaskListParser::depends_on`; `12.46 s` for `30,000`
  features / `100,000` references in a debug build) — outside "major"; the fix is the one the review names (use the
  `feature_lines` map and reject `current.number` separately), and it needs its own RED test first.

## Checks

- `agent gate` on the final commit: see the last section of the final message (recorded by the script).
- Gotcha for whoever writes the next Rust integration test: `clippy.toml`'s `allow-expect-in-tests` covers `#[test]`
  functions only, not helper functions of a test file, so helpers must return `Option` / `Result` and the `#[test]`
  does the `expect`.
- Suggestion for `AGENTS.md` (I may not change it): none beyond the gotcha above, which belongs in the notebook.
