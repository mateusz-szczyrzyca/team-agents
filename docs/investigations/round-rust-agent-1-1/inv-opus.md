# Round rust-agent/1.1 — inv-opus

Item 1.1 of the plan "Rewrite `bin/agent` in Rust" is implemented on branch `agents/inv-opus/20261111-153601`
(base `744056b`), test first, with the whole gate green on a clean clone. One expected part is missing: the CI
workflow change. The token of this run may not push `.github/workflows/` (GitHub refuses it without the `workflow`
scope), so the diff is under [Needs the human](#needs-the-human) for the human to apply.

## Task

```text
1. Workspace and core vocabulary
Depends on: none
[ ] 1.1 Create the Cargo workspace, its gate and the core vocabulary
    Workspace with the four crates of the target layout; `agent --version` and `agent-web --version` work.
    `rust-toolchain.toml` (current stable), edition 2024, the lint baseline from the appendix of the developer rules
    in `[workspace.lints]`, `rustfmt.toml`, `clippy.toml`. A `make` target that runs fmt check, clippy `-D warnings`,
    tests and `cargo doc` with warnings denied, added to the repository's existing gate and CI.
    `docs/architecture.md` with the layout, the process model and the rules above; a Rust section in `AGENTS.md`
    pointing to it and stating the relaxed size limit of this plan.
    In `agent-core`: validated newtypes (parsing, `Display`, serde) for agent names, investigator names, team IDs,
    task IDs, run IDs, task-list item IDs (`2.3`) and branch names, invalid values unconstructable (TYP-1), ID
    generation with an injected clock and random source; `CliKind` (claude, codex, gemini, opencode), `Tier`
    (normal, worker, investigator), `Role` (solo, coordinator, member, worker, reviewer), run outcomes, review
    verdicts, and task and item statuses as state machines whose invalid transitions do not compile or return a
    typed error. Reference: the STATUS values of `bin/agent` `final_status`.
    Expected result:
    - Files: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`; `Cargo.toml` and
      `src/` of `crates/agent-core`, `crates/agent-runtime`, `crates/agent`, `crates/agent-web`;
      `crates/agent-core/src/{ids,kinds,status}.rs`; `docs/architecture.md`. Changed: `Makefile`, `AGENTS.md`, the CI
      workflow.
    - Adds: `AgentName`, `InvestigatorName`, `TeamId`, `TaskId`, `RunId`, `ItemId`, `BranchName` (`FromStr`,
      `Display`, serde); `CliKind`, `Tier`, `Role`, `RunOutcome`, `ReviewVerdict`; `TaskStatus` and `ItemStatus` with
      their transitions; the Rust gate target.
    - Tests: round trip and rejected values for each newtype; every allowed and every refused status transition.
    - Why: every later item builds on these types, and the gate makes every later PR checkable.
    Checks:
    - On a clean clone the Rust gate target and the repository's whole gate pass.
    - `cargo run -q -p agent -- --version` and `cargo run -q -p agent-web -- --version` print a version and exit 0.
    - `cargo tree -p agent-core -e normal` lists only approved dependencies.
    - Each newtype refuses, in a named test, every invalid value its documentation names.
```

## What was done

- **Workspace:** [`Cargo.toml`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/Cargo.toml) (four members; `[workspace.package]` with edition 2024 and `rust-version = "1.99"`;
  dependencies declared once with their minimal features; the lint baseline of the developer rules' appendix in
  `[workspace.lints]`), [`Cargo.lock`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/Cargo.lock), [`rust-toolchain.toml`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/rust-toolchain.toml) (`1.99.1`, the current stable:
  `rustup check` reported `1.99.0 -> 1.99.1`), [`rustfmt.toml`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/rustfmt.toml), [`clippy.toml`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/clippy.toml); `/target/` in `.gitignore`.
- **Crates:** [`crates/agent-core/src/lib.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/crates/agent-core/src/lib.rs), [`crates/agent-runtime/src/lib.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/crates/agent-runtime/src/lib.rs) (crate docs only — see
  [Deviations](#deviations)), [`crates/agent/src/main.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/crates/agent/src/main.rs), [`crates/agent-web/src/main.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/crates/agent-web/src/main.rs): clap with
  `--version` / `--help` on stdout and status `0`, no arguments print the help, a refused command line goes to stderr
  with status `2`, output through injected writers (a closed pipe is `OutputError::Write`, returned from `main`).
- **[`crates/agent-core/src/ids.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/crates/agent-core/src/ids.rs):** `AgentName`, `InvestigatorName`, `TeamId`, `TaskId`, `RunId`, `ItemId`,
  `BranchName` — private fields, the only constructors `FromStr`, `TryFrom<String>` and serde (`try_from`), all
  running one check; `Display` / serde give the text back. `TeamId` / `TaskId` / `RunId` have
  `generate(&impl Clock, &mut impl Rng)`; `Clock` is the injected clock.
- **[`crates/agent-core/src/kinds.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/crates/agent-core/src/kinds.rs):** a keyword-enum macro (one text per variant, `ALL`, `as_str`, `Display`,
  `FromStr`, serde, `UnknownKeyword`), `CliKind`, `Tier`, `Role`.
- **[`crates/agent-core/src/status.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/crates/agent-core/src/status.rs):** `RunOutcome`, `ReviewVerdict`, `TaskStatus`, `ItemStatus`;
  `StatusMachine::transition_to` refuses every change outside the status's table with `InvalidTransition`.
- **Gate:** [`Makefile`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/Makefile) target `rust` (fmt `--check`, clippy `-D warnings`, tests under the test lock, `cargo doc`
  with `RUSTDOCFLAGS="-D warnings"`, all `--locked`), part of `make test`; [`tools/gate.sh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/tools/gate.sh) adds a `rust` step
  when the Makefile defines the target (as it does for `unit-agent-bats`, so the existing gate tests with their fake
  Makefile keep their 7 steps), with an excerpt of cargo's diagnostics. New test file
  [`tests/harness/gate_rust.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/tests/harness/gate_rust.bats).
- **Docs:** [`docs/architecture.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/docs/architecture.md) (layout, process model, the rules of the plan, the vocabulary with its
  transition tables, the gate, test-first in Rust); [`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/AGENTS.md) — a "Rust" section pointing to it with the
  relaxed size limit, the workspace in the components table, rustup in Setup, `rust` among the targets holding the
  test lock; [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2/README.md) — `make rust` and a note on the workspace in "Development and tests".

### Decisions a reviewer should look at first

1. **ID formats** (the item leaves them open):
   - names: `1`–`32` bytes of `a-z`, `0-9`, `-`, first a letter, last not `-` (safe in branch, file and unit names);
   - `team-` / `task-` / `run-YYYYMMDD-HHMMSS-xxxx`: UTC, `xxxx` from `0-9a-z`; the prefix makes an ID of one kind
     refused where another is expected; the timestamp is checked against the calendar; a clock year outside
     `0000`–`9999` is a `GenerateIdError`;
   - `ItemId`: `FEATURE.ITEM`, numbers `1`–`65535` without leading zeros, ordered numerically (`1.9` < `1.10`);
   - `BranchName`: the rules of `git check-ref-format --branch`, checked natively, at most `255` bytes.
2. **`RunOutcome`** keeps `DONE`, `QUESTIONS`, `FAILED`, `SLEEPING`, `EXCLUDED` of `final_status` and refuses
   `ROUND_DONE` (an item replaces a round) and `INVESTIGATING` (standalone investigation requests are out of scope).
   Quota, rate limit and hang are not words an agent writes; item 3.1's `classify_outcome` adds them.
3. **The transition tables** (in `status.rs`'s docs and `docs/architecture.md`): every pause, stop, answer and resume
   of a task goes back through `queued`; `done` is final; an item's fix pass is `in_review` → `in_progress`.
4. **The size rule in `AGENTS.md`:** for PRs of this plan the PR-1 count with its `5000`-line limit *replaces* the
   `3000` lines (tests included) of "Rounds". The plan says only that PR-1's limit is `5000`; I read it as replacing
   the round limit for these PRs.

## Commits

- [`b41a7a9`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b41a7a9e8c8cba1033cb8cdfc9f877cd221ca563) Rust workspace and its binaries' command lines: tests first (red)
- [`cbbac7e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/cbbac7e229db49ca38f8da754c3997c71408601a) Rust workspace and its binaries' command lines (green)
- [`6f17a5d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6f17a5d13cd364d293894ed55f244f0ab61ece6c) The Rust checks in the gate: tests first (red)
- [`5451f5b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5451f5ba3af5e1c8f9d0910afd9d5d4dff84605f) The Rust checks in the gate (green)
- [`9a8e682`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9a8e68268efb093d67e937d73656c8341cf855f1) Core identifiers: tests first (red)
- [`567e99d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/567e99d6d1d7543a01a2d90d45dd9a2424dee695) Core identifiers (green)
- [`c233037`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c2330379efd51f1128e218353b9ab39ce7ab4f20) Core kinds and statuses: tests first (red)
- [`d0caed3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d0caed3c75ac5dac0a8c6324589d1ac195a9b4d1) Core kinds and statuses (green)
- [`0df6bb3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2) Document the Rust workspace: architecture, AGENTS.md, README

## Checks

| Check of the item | Result |
|---|---|
| On a clean clone the Rust gate target and the repository's whole gate pass | `make rust`: exit `0`; `make gate`: `all 9 steps passed` (second run; the first had one timing failure outside this change — see [Gate](#gate)) |
| `cargo run -q -p agent -- --version` / `-p agent-web` print a version and exit 0 | `agent 0.1.0` / `agent-web 0.1.0`, exit `0` both |
| `cargo tree -p agent-core -e normal` lists only approved dependencies | direct: `rand`, `serde`, `thiserror`, `time`; the rest are theirs (`rand_core`, `serde_core`, `serde_derive`, `proc-macro2`, `quote`, `syn`, `unicode-ident`, `thiserror-impl`, `deranged`, `num-conv`, `powerfmt`, `time-core`) |
| Each newtype refuses, in a named test, every invalid value its documentation names | see the next table |

| Newtype | Named test (in `ids.rs`) |
|---|---|
| `AgentName`, `InvestigatorName` | `names_refuse_every_invalid_value_with_its_rule` |
| `TeamId`, `TaskId`, `RunId` | `ids_refuse_every_invalid_value_with_its_variant`, `no_id_is_generated_before_year_0` |
| `ItemId` | `item_ids_refuse_every_value_that_is_not_two_numbers`, `item_ids_refuse_leading_zeros_and_zeros`, `item_ids_refuse_numbers_above_65535` |
| `BranchName` | `branch_names_refuse_every_invalid_value_with_its_rule` |

Each refusal test checks the exact rule or variant (and for timestamps the calendar component), through `FromStr`
and through JSON. Round trips: `*_round_trip` tests per type, plus `1000` seeded random values for names, item IDs and
generated IDs. Transitions: `task_statuses_change_only_along_their_table` and
`item_statuses_change_only_along_their_table` check the whole from × to product (every allowed and every refused
change). Compile-fail doctests: the private constructors of the seven newtypes, and an `InvestigatorName` used as an
`AgentName`; stable rustdoc does not check the error code in `compile_fail,E0603`, so I compiled each snippet by hand
(`E0603` for the tuple types, `E0451` for `ItemId`, `E0308` for the mix-up).

## Gate

Commands run on the final code ([`0df6bb3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0df6bb3f5124eb82e58a49e34eb901b919a4f0f2)), in a clean clone of the branch:

```bash
GATE_WAIT=1 make rust    # exit 0
GATE_WAIT=1 make gate
```

```text
gate: 9 steps, full logs in build/gate/
ok    check-generated     0s
ok    lint-shell         20s
ok    lint-go             2s
ok    lint-docs           0s
ok    unit-go             1s
ok    unit-bats         212s
ok    harness            20s
ok    unit-agent-bats     4s
ok    rust                1s
gate: all 9 steps passed
```

The first `make gate` run of the same commit failed one bats test, outside this change, and passed on re-run:

```text
FAIL  unit-bats         225s  build/gate/unit-bats.log
  not ok 20 a job whose checkout fails is failed with the typed cause, and nothing is recorded as started # in 15668 ms
  # (from function `wait_lead_up' in file tests/unit/node_two_homes.bats, line 133,
```

`tests/unit/node_two_homes.bats` is one of the `BATS_SERIAL` files the Makefile names as bound to timings a loaded
machine misses; alone (`make unit-bats BATS_FILES=tests/unit/node_two_homes.bats`) it passed `12/12`, and the second
whole gate above passed. This change touches neither `bin/agent` nor that test.

`cargo test --workspace --locked` on the final code: `37` unit tests in `agent-core`, `6` unit and `1` binary test
per binary, `4` doctests, `8` compile-fail doctests. `bats tests/harness/gate_rust.bats tests/harness/gate.bats`:
`8/8`.

Mutation check (TDD-2), each reverted after: `len() > MAX_NAME_LEN` → `>=`, the year range `0..=` → `1..=`, the
leading-zero length `> 1` → `> 2`, the suffix check widened to uppercase, the suffix mapping `n < 10` → `n < 9`, the
`@` rule — each turned `1` to `3` unit tests red.

## Size

`1569` changed lines outside tests by the PR-1 command (limit `5000` for this plan). With tests and `Cargo.lock`:
`2,872` insertions and `8` deletions in `25` files.

## Deviations

- `#[expect(clippy::case_sensitive_file_extension_comparisons, reason = ...)]` on `check_branch` in `ids.rs`:
  `.lock` is git's reserved component suffix, not a file extension, and git compares it case-sensitively.
- `debug_assert!` in the private `suffix_char` (ERR-3 allows it for internal invariants; documented under
  `# Panics`): its one caller draws `n` from `0..36`.
- Public error types and enums are not `#[non_exhaustive]` (ERR-8, API-3): the crates are `publish = false`, and
  `#[non_exhaustive]` would force `_` arms in the other workspace crates, which TYP-6 forbids.
- `UnknownKeyword::what` is a `&'static str` label, used only in the message (TYP-4 is about logic).
- `crates/agent-runtime` has only its crate docs: the item lists its `Cargo.toml` and `src/`, and its content comes
  with items 4.1 and 4.2. Nothing calls it, so it is not a placeholder of behaviour (GEN-3).
- `Clock` has no system implementation yet: only tests implement it. The runtime or the daemon passes the system
  clock when it first generates an ID (API-7: nothing here would use it).
- `rustfmt.toml` sets `max_width = 120`, the width of the rest of the repository.

No `unsafe` (every crate has `#![forbid(unsafe_code)]`), no `#[allow]`, no dependency outside the approved list.

## Needs the human

1. **The CI workflow.** `git push` refused it: `refusing to allow a Personal Access Token to create or update
   workflow .github/workflows/test.yml without workflow scope`. I removed the change and its bats test from my
   commits; the docs say CI does not run `make rust` yet. The change for `.github/workflows/test.yml`:

   ```diff
   diff --git a/.github/workflows/test.yml b/.github/workflows/test.yml
   index 3a372d6..9cfc24f 100644
   --- a/.github/workflows/test.yml
   +++ b/.github/workflows/test.yml
   @@ -20,3 +20,7 @@ jobs:
            run: make unit-go
          - name: Script tests (bats, stub CLIs)
            run: make unit-bats
   +      - name: Rust toolchain (rust-toolchain.toml)
   +        run: rustup toolchain install
   +      - name: Rust workspace (fmt, clippy, tests, docs)
   +        run: make rust
   ```

   `rustup` is on the `ubuntu-latest` runners; `rustup toolchain install` without arguments installs the toolchain
   of `rust-toolchain.toml` (checked here with rustup `1.29.3`). The bats test that went with the change, for
   `tests/harness/gate_rust.bats`:

   ```bash
   @test "the CI workflow runs the rust target" {
     grep -qE '^[[:space:]]+run: make rust$' "$REPO_ROOT/.github/workflows/test.yml"
   }
   ```

2. **The rule files are not in the repository.** The topic names `prompts/agent/rust-developer.md` and
   `prompts/investigator/code-review.md`; at base `744056b` neither exists. I read the untracked copies in the human's
   checkout: `prompts/agent/rust-developer.md`, and the review rules as `prompts/investigator/rust-review.md` — there
   is no `code-review.md`. Item 2.3 adds them to the repository; until then the reviewers of this round need the
   human's copies. `AGENTS.md` and `docs/architecture.md` use the plan's names (`code-review.md`).
3. **`AGENTS.md` and the precheck.** The item asks for the Rust section in `AGENTS.md`; the precheck sends a round
   with an `AGENTS.md` change back unless the human allows it (`agent allow <team> AGENTS.md`).

## Out of scope

- "Rounds" in `AGENTS.md` allows "a stub returning a typed not implemented error" for unfinished work; the Rust rules
  forbid placeholders (GEN-3). The Rust section of `AGENTS.md` says the Rust rules apply under `crates/`.
- `rustup` updated itself (`1.29.1` → `1.29.3`) when it installed `1.99.1` into `~/.rustup`; nothing under
  `~/agents` was touched.

## Suggestions for `AGENTS.md`

(No notebook in this run.)

- Once CI runs `make rust`, change "(CI: not yet — rule 6)" in the Rust section.
- Say that the error code of a `compile_fail` doctest is checked by hand: stable rustdoc ignores it.
