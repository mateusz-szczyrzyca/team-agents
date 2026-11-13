# Item 8.2 integration — `inv-astra`

The authoritative manifest contains one published investigator result. The branch fast-forwards through that result,
preserving every RED/GREEN commit and the already-merged 8.1 dependency. No other investigator, agent or peer worktree was
consulted. Serena tools were unavailable; local searches were the instructed fallback.

## Taken and superseded

- `inv-claude-sonnet`: took the complete result [`451288b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/451288b93956aa43980ed63aea2d5d931f921911) and its
  [`docs/investigations/round-rust-agent-8-2/inv-claude-sonnet.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/79832ab6068408419165ef905816172afc83022e/docs/investigations/round-rust-agent-8-2/inv-claude-sonnet.md).
  Configuration, pure decisions, git facts, records, prompts, daemon integration and tests are retained. No findings file was
  absent, and there were no second-investigator duplicates.
- `inv-claude-sonnet`: superseded the assumption that Rust lint expectations need no disclosure and the omission of merge
  changes from frozen-test checks. No other result was dropped.
- `inv-astra`: [`0cd8273`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0cd8273d442a3569fe0c6a7ab0a44db79e8928d8) corrects the expectation test in a dedicated `TEST-FIX:`;
  [`c6e2714`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c6e27142a2b873d64bef4d0f1b7272e68f6f8603) detects both item and crate lint expectations.
- `inv-astra`: [`05162bd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/05162bd032d25ed0a043ee831b479592c837cda2) adds five pure merge cases and a real-git regression;
  [`79832ab`](https://github.com/mateusz-szczyrzyca/team-agents/commit/79832ab6068408419165ef905816172afc83022e) collects parent diffs and checks only parents containing the relevant RED commit. It covers either
  parent, indirect ancestry, RED merges, unrelated-parent imports and the tests-only TEST-FIX exception.

## Checks

- Recorded task gate: `GATE_WAIT=1 make gate` **PASS** at [`79832ab`](https://github.com/mateusz-szczyrzyca/team-agents/commit/79832ab6068408419165ef905816172afc83022e).
- The full repository gate used `GATE_WAIT=1 tools/test-lock.sh full-repository tools/gate.sh`.
- Rust: `1,998` tests passed, `0` failed, `0` ignored; formatting, clippy and documentation passed with warnings denied,
  including the repository's `--locked` and all-features checks.
- Generated-code check, shell lint, Go lint, documentation check, Go tests and `66` harness tests passed.
- The Go candidate Bats step initially exhausted `/tmp` quota while copying build artifacts. Rerunning `make unit-agent-bats`
  with the runner's supported `TMPDIR` set to an isolated disk-backed cache directory passed, including parity validation.
- The sole unresolved failure is the unchanged test in
  [`tests/unit/context_hygiene.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/caa41ea6baa4e9f58a1573bdb633805f3493e2cc/tests/unit/context_hygiene.bats) at line `28`: it requires `--name-only` in
  [`prompts/review.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/caa41ea6baa4e9f58a1573bdb633805f3493e2cc/prompts/review.md). Both files have identical blobs on this branch and `origin/main`.
  The prompt uses the GitHub files list instead. No test or prompt was changed to hide the failure.
- RED for lint expectations failed on the expected missing marker. RED for merge checks failed four pure assertions and the
  real-git assertion of `0` parent diffs versus `2`. The corresponding GREEN checks passed.

## Tests changed after RED

- [`0dcd4f2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0dcd4f29ef1a288247f107dfe524b7d651a938e9): imported `TEST-FIX:` corrects an allegedly additive fixture that actually removed a line.
- [`0cd8273`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0cd8273d442a3569fe0c6a7ab0a44db79e8928d8): `TEST-FIX:` corrects the false exemption of lint expectations.
- [`a721ec2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a721ec207708c3b9a33b441251cdc72250f68753): imported TDD-14 deviation changes a closure to the equivalent method reference in a test, in a mixed
  production/test commit. The assertions and behavior are unchanged, but it is not a `TEST-FIX:`. History is retained and the
  deviation must be listed in the PR.
- [`30c5af4`](https://github.com/mateusz-szczyrzyca/team-agents/commit/30c5af42371fbcf7290975b9fa164d36fb5c59af): four imported daemon tests were added after their behavior; its commit documents the TDD deviation.
- No test from `main` changed. The existing fake CLI only gains a new scripted action. No skips or suppressions were added.

## Not done

The full gate cannot be claimed green. The repository instructions require asking about a failure outside this change.
The task's `questions-1.md` asks whether to add one explicit local file-listing command to the review prompt, without changing
its test, or wait for a separate baseline repair. The single PR has not been opened pending that decision; its description is
prepared. Pushes remain restricted to `agents/inv-astra/20261113-183408`.

The original report's documented limits remain: checks-specific daemon tests use a solo lead, paths with control characters
and ranges over `500` commits fail publication, and PR disclosures require GitHub access. No live models were run.
The task gate rejects dirty trees, so focused tests ran before integration commits and recorded gates after commits.
No dependency, persisted legacy schema, repository rule or live installation was changed. A separate `Checking` type preserves
existing exhaustive round-state tests; merge-parent facts are in memory only.
