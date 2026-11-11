# Round rust-agent/2.1 — findings of inv-claude-sonnet

Item 2.1 of the plan "Rewrite `bin/agent` in Rust": the configuration and the task list file, in `agent-core`. Branch
`agents/inv-claude-sonnet/20261111-173122`, last code commit
[`024c334`](https://github.com/mateusz-szczyrzyca/team-agents/commit/024c334ee44ad10d3302c136f5b322cf664d1fbf).
No pull request, as instructed; the merger integrates the independent results.

## Result

All three checks of the item hold, and the whole repository gate passes on the last code commit.

- `config/agents.example.toml` loads in a test; a copy with `codex-1` in both tier lists fails with
  `ConfigError::NameInTwoLists { name: "codex-1", first: Normal, second: Worker }`.
- The plan parses in a test into `10` features and `16` items, each with its expected result (and its checks).
- A task list with a malformed item line fails with the number of that line (`TaskListError::MalformedItem`), on small
  fixtures and on the plan with one line broken.

## Checks run

```bash
GATE_WAIT=1 make gate     # PASS at 024c334: all 9 steps (check-generated, lint-shell, lint-go, lint-docs,
                          #   unit-go, unit-bats, harness, unit-agent-bats, rust); recorded by `agent gate`
make rust                 # fmt --check, clippy -D warnings, 183 tests, cargo doc -D warnings: exit 0
cargo tree --locked -p agent-core -e normal   # rand, serde, thiserror, time, toml (+ toml's parser crates)
```

| Tests added | Count |
|---|---|
| unit, `config::values` | `30` |
| unit, `config` | `19` |
| unit, `config::secrets` | `12` |
| unit, `task_list` | `36` |
| integration, `tests/config.rs` (the example file) | `5` |
| integration, `tests/task_list.rs` (the plan) | `8` |
| doctests (`4` run, `6` `compile_fail`) | `10` |

Every test was seen failing before its code ($\color{red}{\textbf{RED}}$ commits carry the failing output), then passing
($\color{green}{\textbf{GREEN}}$). After the last green I broke `36` conditions one at a time (a boundary `>` to `>=`, a
check removed, a mask changed, an order swapped; script not committed) and ran the suite: all `36` were caught.
`cargo-mutants` is not installed here.

## Size (PR-1)

`2,780` changed lines outside tests with the plan's own command (limit `5,000`). `15` files, `+5,417` / `−3` in total,
tests and the `438`-line fixture included.

## Commits

Four test-first cycles, then documentation:

| | Commit |
|---|---|
| $\color{red}{\textbf{RED}}$ | [`e516c5b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e516c5b88b61f90e2d23563d68e96319197a96a6) Configuration values: tests first |
| $\color{green}{\textbf{GREEN}}$ | [`27f8d60`](https://github.com/mateusz-szczyrzyca/team-agents/commit/27f8d6053df9286503e1581a27cf8ef255b8c158) Configuration values |
| $\color{red}{\textbf{RED}}$ | [`d7e3b6f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d7e3b6f669c83ab62c05458562ae6c4ed6883491) Configuration file agents.toml: tests first |
| $\color{green}{\textbf{GREEN}}$ | [`7d50233`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7d50233329f1a3457b35b9014ec39dba5a9c82fd) Configuration file agents.toml |
| $\color{red}{\textbf{RED}}$ | [`700aa56`](https://github.com/mateusz-szczyrzyca/team-agents/commit/700aa5603cca258fa534ac194e84f00f9bf21320) Secrets file secrets.toml: tests first |
| $\color{green}{\textbf{GREEN}}$ | [`2452325`](https://github.com/mateusz-szczyrzyca/team-agents/commit/24523254754e300558598059dffbf4727cba987e) Secrets file secrets.toml |
| $\color{red}{\textbf{RED}}$ | [`101f7ec`](https://github.com/mateusz-szczyrzyca/team-agents/commit/101f7ec481bbe98e5b13f9cf7be2a71955b5d4a2) Task list parser: tests first |
| $\color{green}{\textbf{GREEN}}$ | [`013f57b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/013f57b8391c5436c4733680ff0e1ec9a6bd57aa) Task list parser |
| | [`b44f8f3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b44f8f39376bc9b5d46cfa4233d94e450809719e) Doc examples and compile-fail checks (`Test-First: not possible`, stated) |
| | [`d6f833e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d6f833ec081342c398a7f48b0c9ce9d1b9aa0632) Document the configuration and the task list |
| | [`d56b516`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d56b5160f9ff32f8b933ff1f73cc916f236b4996) Keep the cause of a bad feature number (found in self-review) |
| | [`024c334`](https://github.com/mateusz-szczyrzyca/team-agents/commit/024c334ee44ad10d3302c136f5b322cf664d1fbf) State the order of the list checks as the code applies it |

## Files

Expected by the item:

- [`crates/agent-core/src/config.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/024c334ee44ad10d3302c136f5b322cf664d1fbf/crates/agent-core/src/config.rs),
  with its modules [`config/values.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/024c334ee44ad10d3302c136f5b322cf664d1fbf/crates/agent-core/src/config/values.rs)
  (the typed values) and [`config/secrets.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/024c334ee44ad10d3302c136f5b322cf664d1fbf/crates/agent-core/src/config/secrets.rs)
- [`crates/agent-core/src/task_list.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/024c334ee44ad10d3302c136f5b322cf664d1fbf/crates/agent-core/src/task_list.rs)
- [`config/agents.example.toml`](https://github.com/mateusz-szczyrzyca/team-agents/blob/024c334ee44ad10d3302c136f5b322cf664d1fbf/config/agents.example.toml)
- [`docs/configuration.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/024c334ee44ad10d3302c136f5b322cf664d1fbf/docs/configuration.md),
  [`docs/task-list.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/024c334ee44ad10d3302c136f5b322cf664d1fbf/docs/task-list.md)

Outside the list, each with its reason:

- `Cargo.toml`, `Cargo.lock`, `crates/agent-core/Cargo.toml` — the `toml` dependency (below); `Cargo.lock` only gains
  `toml` and its crates.
- `crates/agent-core/src/lib.rs` — `pub mod config; pub mod task_list;`.
- `crates/agent-core/tests/config.rs`, `tests/task_list.rs` — the integration tests for the item's checks.
- `crates/agent-core/tests/fixtures/rust-agent-plan.md` — a byte copy of the plan, which the item says is a test
  fixture. The plan file is untracked in the main checkout; this copy is as it was on 2026-11-11.
- `docs/architecture.md` — a table of the two new modules.
- `docs/investigations/round-rust-agent-2-1/inv-claude-sonnet.md` — this report.

## What was built

- **`Config`** from `Config::from_toml_str(&str)`: `language`, `[agents.NAME]`, `[investigators.NAME]`, `[tiers]`
  (`normal`, `workers`), `[binaries]`, `[watchdog]`, `[limits]`, `[review]` (`fix_passes`, default `5`), `[git]`,
  `[github]`, `[notifications]`, `[web]`, `[repos.NAME]`. `deny_unknown_fields` everywhere, defaults for every optional
  table, `1 MiB` bound. Typed `ConfigError`: `TooLarge`, `Parse` (with the TOML error as its source, line and
  column), `UnknownName`, `DuplicateName`, `NameInTwoLists`, `AgentInNoList`.
- **Typed values** (`config::values`), each parsed once and refusing with an `InvalidValue { what, value, rule }`:
  `Language`, `ModelName`, `RepoName`, `EnvName`, `GithubLogin`, `TailscaleLogin` (both kept lowercase),
  `NameTemplate` (`{agent}`), `AbsolutePath`, `Program` (bare name or absolute), `CliArgs` (an array), `CommandLine`,
  `Minutes` (never `0`), `LoopbackAddr` (loopback, port not `0`).
- **`Secrets`** from `Secrets::parse(text, file_mode)`: the mode is a parameter, so it cannot be forgotten, and is
  checked first (`SecretsError::Mode`); `Secret` prints `Secret(<redacted>)`, has no `Display` or `Serialize`; parse
  errors keep only line and column, because `toml`'s messages quote the refused line.
- **`TaskList`** from `TaskList::parse(&str)` (also `FromStr`): the optional first-line `Coordinator:`, the brief,
  features, `Depends on:`, items with `[ ]` / `[x]`, indented description, and the parts `Expected result:` and
  `Checks:` kept apart. `17` error variants, each with its line.

## Decisions to check (the human reads `docs/configuration.md` after feature 2)

Also listed in `docs/configuration.md`, "Decisions to review":

1. An agent in neither tier list is an error (`AgentInNoList`); the old harness counted it as normal.
2. Paths are absolute: no `~`, no `$HOME`.
3. Investigators are a table, iterated by name; no configured order.
4. One `language`; the old `REPO_LANGUAGE` is gone.
5. Secrets in three scopes: `github_token`, `[agent_env]` (every model CLI), `[notification_env]`.
6. `[repos.NAME]` needs `gate` and `base_branch`; a repository not listed has no entry.
7. Not in the configuration yet: Claude's turn limit, Antigravity's print timeout, opencode's bash timeout. Item 5.1
   decides; a new optional key is compatible.
8. A later feature depends only on features written before it, so dependencies cannot form a cycle.
9. **`Checks:` is a part of its own**, next to `Expected result:`. The item names title, description and expected
   result; the plan's format has a `Checks:` block in every item and the reviewers run it, so dropping it would lose
   it, and folding it into the expected result would blur what the PR is checked against. `Item::checks` is
   `Option<String>`.
10. `Coordinator:` is typed `AgentName`: an investigator's name has the same form, and the configuration says which it
    is (item 3.1 resolves it).

## Deviations

- **New dependency `toml` `1.2.1`**, features `std`, `serde`, `parse` only (DEP-1/DEP-2). It reads the two files;
  nothing in std does. On the plan's approved list, MIT or Apache-2.0, maintained by the `toml-rs` organisation, the
  de-facto TOML crate. Needs the human's approval per AGENTS.md "Never" 8, and is listed under NEEDS HUMAN.
- **`unwrap()` in six `const` initialisers** (`NonZeroU32::new(30).unwrap()`, ...): TYP-10 names this pattern; a zero
  would fail the build. clippy accepts it. No `#[expect]` was needed.
- **`ConfigError::Parse` exposes `toml::de::Error`** as its source (ERR-8, minor): deliberate, it carries the line and
  column.
- **Unchecked crate-internal constructors** `Program::literal`, `AbsolutePath::literal`, `NameTemplate::literal`,
  `Language::literal`, `Minutes::literal` write the defaults as literals. They are `pub(crate)`;
  `defaults_pass_their_own_checks` re-parses each default.
- **`ParseIntError` kept** as the source of `NumberProblem::Range`; `LoopbackAddr` drops `AddrParseError`, which says
  no more than `ValueRule::NotSocketAddr` and the refused text (commented in the code).
- **No `#[allow]`, no `#[expect]`, no `unsafe`, no lint or CI configuration touched.** `AGENTS.md` and `CLAUDE.md` are
  unchanged.
- **Two tests were changed after their RED commit**, both my own and both in the GREEN commit of their cycle, with the
  reason in its message: two wrong expectations of the `Config` cycle (an unknown section is reported at its header,
  one table case had its line second), and in the task-list cycle a `"+1"` case added and seen failing, plus a test
  that passed on an empty list made to count its items. `d56b516` then changes the shape of `FeatureNumber` and
  `MalformedDependsOn` and their two tests with it.

## Notes on the environment

- **The base was behind.** The branch was created from `096caa3`, before item 1.1 merged, and the item depends on it.
  I fast-forwarded to `origin/main` (`bee5ef0`); no commit of mine existed then. The diff above is against it.
- **The rule files are not tracked.** `prompts/agent/rust-developer.md` and `prompts/investigator/code-review.md` are
  not in the repository; the main checkout has the first, and the second as `rust-review.md`. I read both from there
  (read-only), as the investigator of item 1.1 did. Item 2.3 is to add them.

## Not done, and for the merger

- `Config` and `Secrets` are not read from disk: that is item 4.1 (the runtime reads the file, takes `st_mode` and
  calls `Secrets::parse`).
- `TaskList` and `Config` are not serializable. The state (item 2.2) stores what it needs; a `Deserialize` for
  `TaskList` would have to go through the same checks.
- Independent investigators will have chosen differently on the points above; the merger should compare these first:
  the shape of `[tiers]`, whether `Checks:` is a field, whether a repository needs `gate` and `base_branch`, the error
  for an agent in no list, and the position-only secrets errors.

## For `AGENTS.md` / the notebook (no notebook tool in this run)

- The Rust section could say that `tests/*.rs` helper functions are linted (`unwrap` there fails `-D warnings`) while
  `#[test]` functions are not, and that `assert!(x.is_empty())` and `assert!(r.is_ok())` are denied by the baseline.
- `cargo fmt` reflowed lines of tests written before it ran; format before the RED commit.
