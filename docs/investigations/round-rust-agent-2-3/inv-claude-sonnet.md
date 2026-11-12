# Round rust-agent/2.3 — findings of inv-claude-sonnet

Item 2.3 of the plan "Rewrite `bin/agent` in Rust": the prompt catalog and the model CLI contract, in `agent-core`.
Branch `agents/inv-claude-sonnet/20261112-005213`. No pull request, as instructed; the merger integrates the
independent results.

## Result

All three checks of the item hold, and the whole repository gate passes (`make gate`, recorded at `179b646`; `make
rust` again after the last commit, exit `0`).

- `git grep -nE '"[^"]{100,}"' -- 'crates/*.rs'` finds `3` hits, all older than this round and none a prompt: a
  lint `reason` in `ids.rs` and two test fixtures in `task_list.rs`. This round adds none.
- Removing a prompt file makes `a_missing_prompt_file_fails_with_its_name` pass only because the failure line starts
  with `prompts/worker-agent/rust-developer.md`; an unlisted or broken placeholder fails with the file's name (and
  the line); a listed variable the text leaves out fails as `PromptError::Unused` (unit test on a synthetic spec).
- `tests/cli_contract.rs` implements `CliAdapter` and `OutputParser` from strings alone: no `std::process` anywhere.

## Design

- `prompt`: `PromptId` → `PromptSpec { path, audience, variables }`. `Template::parse` / `render` are the strict
  renderer: `\{{` is a literal `{{`, a value is never re-expanded, names are `[a-z][a-z0-9_]{0,63}`, errors carry the
  line. `PromptSpec::render` also demands that the text and the catalog name the same variables.
- Languages: `PromptSpec::candidates(&Language)` lists `lang/<language lowercase>/<path>` then English for a text for
  the human (`Audience::Human`); a text for a model has English only. No human-facing prompt exists yet (they come
  with the `harness/` texts of later items), so this is tested on a synthetic spec.
- `cli`: `CliAdapter` (`kind`, `build_run`, `parser`), `OutputParser` (`push_line`, `finish`) as the only trait
  objects (the CLI is chosen at run time). `RunRequest` → `RunSpec` (program, args, env, files, `worktree_excludes`).
  MCP servers are a `BTreeMap<McpServerKind, CommandLine>`, so a kind cannot be given twice. `RunOutput` has the final
  message, the `SessionId`, `StreamOutcome` (`Succeeded` / `Failed` / `Incomplete`, for a CLI that reports an error
  with exit status 0) and the `ErrorTail`.

## Decisions to look at

- **Rule files.** At the base `prompts/agent/rust-developer.md` and `prompts/investigator/rust-developer.md` were
  tracked (identical), the review rules as `prompts/investigator/rust-review.md`. I renamed it to
  `prompts/investigator/code-review.md` (the name of the layout and of `AGENTS.md`) with `git mv`, and changed the one
  reference in the two developer files. `prompts/worker-agent/rust-developer.md` is new: sections 3–15 and the
  appendix are verbatim, the rest is rewritten for a delegated task (no PR size ownership, a result instead of a PR
  description). I did not write investigator-specific rules; the human-provided file is used as it is.
- **`keyword_enum!`** is used for `McpServerKind`; item 2.4 converts it ("any added since"). No dependency was added.
- **Files outside the item's list:** `crates/agent-core/src/lib.rs` (two `pub mod`), `docs/architecture.md`,
  `crates/agent-core/tests/{prompt_catalog,cli_contract}.rs`.
- The `rustdoc` text of `config::Language` ("names a directory of prompt overrides") matches `prompts/lang/<language>/`.
- `SessionId` is in `cli.rs`, not `ids.rs`, to stay inside the file list; it reuses `InvalidValue`/`ValueRule`.

## Checks run

```bash
GATE_WAIT=1 make gate     # PASS at 179b646 (all steps, recorded by the gate)
GATE_WAIT=1 make rust     # after a6573f9: fmt, clippy -D warnings, tests, cargo doc: exit 0
```

`+2,108` / `−6` lines in `10` files in total; `1,043` changed lines outside tests by the PR-1 command (limit `5,000`).
Tests added: `24` unit in `prompt`, `6` unit in `cli`, `4` in `tests/prompt_catalog.rs`, `6` in `tests/cli_contract.rs`.

Mutation check (by hand, not committed): `8` conditions broken one at a time; `7` were caught. The survivor was a
line count that no valid input needs; I removed that line (`a6573f9`).

## Commits

| | Commit |
|---|---|
| prompts | `5f9010c` Publish the role prompts of the Rust plan: review rules, worker variant |
| RED | `cfff216` Prompt catalog and renderer: tests first (red) |
| GREEN | `76a7058` Prompt catalog and renderer (green) |
| RED | `9a934ad` Model CLI contract: tests first (red) |
| GREEN | `442ae5f` Model CLI contract (green) |
| docs | `179b646` Document the prompt catalog and the CLI contract in the architecture |
| cleanup | `a6573f9` Drop a line count that a valid placeholder never needs |

(Commit links: `https://github.com/mateusz-szczyrzyca/team-agents/commit/<SHA>` once the branch is pushed.)

## Not done / for the merger

- No prompt of the catalog has a variable yet, so "removing a variable a prompt requires" is tested on a synthetic
  spec (`a_prompt_refuses_a_text_that_leaves_a_listed_variable_out`), not on a real file; real entries arrive with
  the `harness/` texts.
- Suggestion for `AGENTS.md` (not changed): the Rust section could name `prompts/lang/` as the place of overrides.
- Two RED commits changed lint findings in their own new tests in the GREEN commit (clippy), no assertion changed.
