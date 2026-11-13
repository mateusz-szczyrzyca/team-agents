# Item 8.2 — Round checks before review (`inv-claude-sonnet`)

Branch `agents/inv-claude-sonnet/20261113-174121`, from `0a774462` fast-forwarded to `caa41ea6` (see "Base" below). Not a pull
request: the merger integrates the branches.

## Task

Item 8.2 of the plan "Rewrite `bin/agent` in Rust" (feature 8, Autopilot; depends on 7), as `rounds/round-8.2.md` has it:

> **8.2 Round checks before review.** Before a PR's head goes to the reviewers — the first time and after every fix pass — the
> daemon checks the round's own diff (from the merge base with the base branch to the head) and its commits, natively through
> git, with no model. What is a test and what judges a round comes from the repository's `checks` table in `agents.toml`, as
> path patterns: `test_paths`, `code_paths`, `protected_paths`. Nothing in the code names a repository or its layout; a
> repository without the table goes straight to review.
> Only the human may allow — the round waits on a question that names each file; allowed: the review starts and the paths are
> recorded for the item; refused: back to the team: an existing test file changed other than by added lines, removed or
> renamed; a protected file changed.
> Back to the team with the reasons, at most `check_passes` times (`[review]`), then the human is asked: a test file that a
> `RED:` commit of the round touched has lines removed or replaced by a later commit, unless that commit touches test files
> only, its subject starts with `TEST-FIX:` and the PR description names it under `## Tests changed after RED` (TDD-14); the
> round changes production code and adds no line to a test file, and none of its commits says `Test-First: not possible —
> <reason>` (TDD-1); an added line that skips a test, suppresses a lint or asks whether the code runs under test, in a file the
> PR description does not name (TDD-11, LINT-3, TDD-12); the markers are a built-in table per file extension. A test module
> inside a source file is not a test file: it stays with the reviewers.

## What and why

Without these checks the default `--autopilot` merges on the word of two models; the Bash harness held such rounds without a
model (its precheck). The design, in short (the section "Round checks before the review" of
[`docs/architecture.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/0c1c25ec9a22795524b88c913aac1ee04a39694a/docs/architecture.md) has the whole):

- **`[repos.NAME.checks]`** has `test_paths`, `code_paths`, `protected_paths` (path patterns: `*`, `?`, `**`, `dir/`, a bare
  name at any depth); `[review] check_passes` (default `2`, the Bash `AGENT_PRECHECK_MAX`). A repository without the table goes
  straight to review.
- **The rules** are one pure function, `agent_core::decide::checks::assess`: facts in (files, commits with their parents, added
  lines, the description), `Issue`s out. Each issue says who may allow it (`Judge::Human` for a changed test and a protected file,
  `Judge::Team` for the others) and what it is about (`Subject`: a path, or the lack of tests).
- **The checking phase** is `decide::round::Checking`: no issue → reviews start; an issue the human refused → back to the team;
  an issue the team can fix → back while `check_passes` has passes left, then the human is asked; only what the human alone may
  allow → the human is asked at once.
- **When**: in the autopilot's publishing, when the lead waits and its gate passed at the tip. A first round is checked after the
  draft pull request is opened and before it is marked ready; a fix pass before the branch is pushed, so a head the checks hold
  back is not pushed.
- **The message to the team** is the reasons (templates under `prompts/harness/checks/`) given to the lead with `Why::Checks`.
- **The question to the human**: the status table has no `waiting → questions`, so — as for 8.1's used-up passes — the lead is the
  messenger, but the daemon writes the questions file itself (one question per subject, with the reasons and a recommendation to
  refuse) and wakes the lead to end its turn with `STATUS: QUESTIONS`. **Only the word `allow` allows.** A lead that does not put
  the question is woken again twice, then the human is told directly.
- **The record** (`tasks/<lead>/checks.json`: passes and allowed subjects per item, the question the round waits on, what the lead
  is still to be told) is a file of the lead's task directory like the gate record, not a part of the state, so nothing of the
  state's format changed and a restart loses nothing.

## Where the code is

| What | Where |
|---|---|
| path types and globs | `crates/agent-core/src/repo_path.rs` |
| the `checks` table and `check_passes` | `crates/agent-core/src/config.rs`, `config/agents.example.toml`, `docs/configuration.md` |
| the rules, the facts, the markers | `crates/agent-core/src/decide/checks.rs`, `decide/checks/markers.rs` |
| the checking phase | `crates/agent-core/src/decide/round.rs` (`Checking`, `ChecksStep`, `ChecksPolicy`) |
| merge base, files, commits and added lines of a range | `crates/agent-runtime/src/git.rs`, `git/range.rs` |
| the record | `crates/agent-core/src/state/checks.rs`, `crates/agent-runtime/src/store.rs` |
| the texts | `prompts/harness/checks/` (22 files), catalog `crates/agent-core/src/prompt/checks.rs`, loader `crates/agent-runtime/src/checks_prompts.rs` |
| reasons, questions, what an answer means | `crates/agent/src/daemon/checks.rs` |
| carrying it out | `crates/agent/src/daemon/owner/rounds/checks.rs`, `publish.rs`, `prompts.rs` |

Files outside the expected list of the item, each needed for it to work: `repo_path.rs` (the vocabulary the table and the rules
share); `decide.rs`, `lib.rs`, `prompt.rs`, `state.rs`, `runtime/lib.rs`, `daemon/mod.rs`, `owner.rs`, `owner/start.rs`,
`team/prompts.rs` (wiring and one new `Why`); `state/checks.rs` and `store.rs` (the record); `prompt/checks.rs` and
`checks_prompts.rs` (a fifth catalog: the tests of the autopilot catalog pin its two directories, so the texts of
`prompts/harness/checks/` cannot join it); `git/range.rs` (the part of `git.rs` that reads a range, in a submodule so that
`git.rs`, which has 900 lines of its own tests, grows by 50); `owner/rounds/{checks,publish,prompts}.rs`;
`tests/fixtures/team-cli.sh` (one new action, `script:NAME`); `docs/architecture.md`, `docs/state.md`, `README.md`.

## Size

`3,181` changed lines outside tests, with docs and prompts (the PR-1 counter over the whole branch, this report not counted), `2,995` of them Rust:
under the limit of 5,000 and under the 4,000 at which PR-2 asks for a split planned up front. The branch is a series of
`RED:`/`GREEN:` pairs, so it can be taken as a stack: the core (paths, config, rules, the phase, the record), the runtime (git,
store, texts), the daemon.

## Tests

`1,992` Rust tests pass in the workspace (`0` failed, `0` ignored), `make gate` passes at the head (`0` findings). New, by
behaviour:

- `repo_path`: the parsing of paths and patterns and every glob case (`crates/agent-core/src/repo_path/tests.rs`).
- the table: `crates/agent-core/tests/config_checks.rs`.
- each rule as a decision, the case that raises it and the nearest that does not (a named `TEST-FIX:`, a `Test-First: not
  possible` line, a named file, a commit that is not later because it is not a descendant, a merge that makes it later):
  `decide/checks/tests.rs`, `markers/tests.rs`; the phase: `tests/round_checks.rs`.
- the facts of a range read from real repositories in temporary directories, isolated from the machine's git configuration:
  `crates/agent-runtime/tests/git_range.rs` (renames with their lines, binary files, merges, messages with control characters,
  paths taken literally, the commit limit).
- in a daemon, with the fake GitHub, a bare `origin` and a stand-in CLI that runs a script the test wrote
  (`crates/agent/src/daemon/owner/rounds/tests/checks.rs`): an edited existing test asks the human by name, no review until they
  `allow`; a refusal goes back with the refusal named; a protected file waits for the human; code without a test goes back with the
  reason and no review, and after `check_passes` the human is asked; a RED test edited by its GREEN commit goes back with the
  commit and the file; the same change as a named `TEST-FIX:` goes to review; the checks run again on a fix pass's head and a
  held head is not pushed; a lead that does not ask is woken twice and the human told once; nothing is asked twice while the
  human has not answered; an answer survives a restart; two questions answered each for its own file; a file that only the
  item's text names is not disclosed; a repository without the table goes straight to review.

### Tests changed after RED

One, `0dcd4f29` (`TEST-FIX:`), `crates/agent-core/src/decide/checks/tests.rs`: `a_round_that_adds_its_tests_is_no_issue` had a
case that replaced a line of an existing test file, which the first rule rightly reports; the case now adds two lines and removes
none. No existing test (before this branch) was changed, removed, renamed or skipped; the existing tests that list every
`RoundPhase` and `RoundEventKind` are why the phase is a type of its own.

### Tests added after the code

Four tests of `tests/checks.rs` (the lead that does not ask, no second question while waiting, the restart, two questions) were
written after the behaviour: the commit says so with `Test-First: not possible`; the first was run with `MAX_RELAYS` changed
and fails. This is a deviation from TDD-1 and I record it as such.

## Checks the item names, as run

- *A round that edits an existing test file gets no review; the human's question names the file; after the human allows it, the
  two reviews start* — `a_round_that_edits_an_existing_test_asks_the_human_by_name_and_starts_no_review_until_they_allow_it`.
- *A `GREEN:` commit that edits a test its `RED:` commit added goes back with the commit and the file; the same change as a named
  `TEST-FIX:` goes to review* — `a_green_commit_that_edits_the_test_of_its_red_commit_goes_back_to_the_team_with_the_commit_and_the_file`,
  `the_same_change_as_a_test_fix_that_the_description_names_goes_to_review`.
- *A round that changes a protected file waits for the human; one that changes code and adds its tests goes straight to review* —
  `a_round_that_changes_a_protected_file_waits_for_the_human`,
  `a_round_that_changes_code_without_a_test_goes_back_to_the_team_with_the_reason_and_starts_no_review` (the fix adds the test
  and the review starts).

```bash
make gate                                              # PASS at the head
cargo test -p agent --bin agent checks                 # 37 tests, about 2 s
```

## Commits

| Commit | Subject |
|---|---|
| [`e72fa1b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e72fa1bc501e7b29ce890e674894f629fff1c11e) | RED: test the paths of a repository and the patterns that match them |
| [`ece5c8a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ece5c8a7edc716eba79dd118df9f57e64d9dbce2) | GREEN: parse repository paths and path patterns, and match them |
| [`b0d854c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b0d854ce67ce4abb74b301c55b650d3d05fd4e12) | RED: test the checks table of a repository and check_passes |
| [`f3baa51`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f3baa512758f419448cbe4690237deca046b542d) | GREEN: read the checks table of a repository and check_passes |
| [`153f06f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/153f06fab66520e7a830c1bf0b9ca01c328b799a) | RED: test the rules of the checks before a review |
| [`0dcd4f2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0dcd4f29ef1a288247f107dfe524b7d651a938e9) | TEST-FIX: a round that adds its tests must not replace a line of one |
| [`26845c0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/26845c0bf83a288547395471bb1948007de7b8b4) | GREEN: assess a round against the rules of the checks |
| [`b2cc527`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b2cc527d6756b92fb4172aa633c458a8fb00c184) | RED: test the checking phase of a round |
| [`fea4342`](https://github.com/mateusz-szczyrzyca/team-agents/commit/fea4342ad520825b784432b1e856216f6d5c0698) | GREEN: conclude the checking phase of a round |
| [`fbb0374`](https://github.com/mateusz-szczyrzyca/team-agents/commit/fbb037494bac188963961b5c4b285f23d1dc6461) | RED: test what git says about a range of commits |
| [`74cb88e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/74cb88e4e044436c4d30a32573d566fc66b3a4bf) | GREEN: read the merge base, files, commits and added lines of a range |
| [`59e90f1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/59e90f161750d93fbc8b0e7f92bc860a55cc8a6f) | RED: test the record the checks keep for the lead of a team |
| [`cce77d2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/cce77d241b30e7ba7d55b3ca3b3a16aefd4e409b) | GREEN: keep what the checks hold of each item |
| [`4d4410c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4d4410c3f1dd1093a8f46a57bbeb8ff99e13b4d6) | RED: test the file that keeps the record of the checks |
| [`ed25818`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ed25818e58a2d8712db3c0a62d2e6f5195a0acd8) | GREEN: keep the record of the checks in the lead's task directory |
| [`12f9fad`](https://github.com/mateusz-szczyrzyca/team-agents/commit/12f9fad55245966e7394650b7607b6476635d244) | RED: test the catalog of the texts of the checks before a review |
| [`eb3197b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/eb3197b397f942b720444ed7fdc9bcb01a8ebc03) | GREEN: write the texts of the checks before a review |
| [`5ad5b56`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5ad5b5650fe11b4d6b1007964466854a37e1118c) | RED: test the loader of the texts of the checks |
| [`4f45466`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4f454666a02dc5cb27880af559fac996f37d718c) | GREEN: load the texts of the checks with the language override and every problem |
| [`1c039d3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1c039d3c8ae119f5b84adbcf116c4e31773a42d8) | RED: test the reasons, the questions and the answers of the checks |
| [`a016e60`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a016e60e5b0e0d1c181c9335a00f90035f5093ed) | GREEN: make the reasons, the questions and the answers of the checks |
| [`a4b98b4`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a4b98b45831e87f76d2b02afdfa1ce48cc23d1b9) | RED: test the checks before a review in a daemon |
| [`e5954be`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e5954be3d94dfea1d51ac782ea7006ab501de09a) | GREEN: check the round before its review in the owner |
| [`30c5af4`](https://github.com/mateusz-szczyrzyca/team-agents/commit/30c5af42371fbcf7290975b9fa164d36fb5c59af) | Test the lead that does not ask, a restart and answers for two files |
| [`a721ec2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a721ec207708c3b9a33b441251cdc72250f68753) | Read the answers by position, and keep no error in an unwrap_or_default |
| [`4590942`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4590942a803df3ef3c2c16ce804e795eec5068d3) | RED: test that the text of an item does not disclose a file to the checks |
| [`38ddeed`](https://github.com/mateusz-szczyrzyca/team-agents/commit/38ddeedf43acdcb565c9f6040c31162c39afb69c) | GREEN: read only what the team wrote in the description |
| [`0c1c25e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0c1c25ec9a22795524b88c913aac1ee04a39694a) | Document that the description is read without the harness's own lines |

## Deviations

- **Base.** `0a774462` has no item 8.1, which 8.2 builds on (the pull-request loop is merged in PR #97, one commit later). I
  fast-forwarded the branch to `caa41ea6` (`git merge --ff-only origin/main`; the branch had no commit of its own), so the diff
  against `main` is only this item.
- **The checking phase is not a state of `RoundState`.** The existing tests of `decide/round.rs` list every `RoundPhase` and
  `RoundEventKind` and refuse an unknown one, and they may not be changed (rule 1). `Checking` is a type of its own in
  `round.rs`; the item's text says "in `decide/round.rs`", which it is. If the merger prefers a variant of `RoundPhase`, three
  existing tests have to change and the human has to allow it.
- **Intermediate commits of the daemon part warn of unused code** until `owner/rounds/checks.rs` wires it (PR-5 offers
  `#[expect(dead_code)]`; I did not add them, because they would be removed again in the same branch). The head is clean.
- **Suppressions:** none (`#[allow]`, `#[expect]`, `nolint`): `0`.
- **New dependencies:** none.
- The record is a file of the lead's task directory, not a field of an item (rule 9: no persisted type changed, so no fixture of
  an earlier format).

## Not done, and limits

- The description is read from GitHub when the check runs, without the harness's own lines: the harness writes it, so a
  `## Tests changed after RED` section or the name of a file with a suppression has to be added by the team or the human; the
  first check of such a round sends it back and the next reads the edit (a test does it). A lead that has no GitHub access
  cannot, and the human is asked after `check_passes`.
- One question at a time per lead; two items that share a pull request move together (their passes are the larger, what was
  allowed for either applies to both).
- A path with a control character, or a range of more than 500 commits, cannot be checked: the branch is not published and the
  human is told once, as for any failed publication.
- No test through a team with a coordinator and members: the checks run on the lead's branch and the lead's wake is the same for
  a coordinator, but the tests use a solo agent.

## For `AGENTS.md` (not changed)

Nothing to change. One note for the notebook: a rule that says where a team's agents can edit the description of their pull
request (the harness opens it and the checks read it) would help the leads: today the reasons say "name it in the description of
the pull request" and rely on the lead having a way to do it.
