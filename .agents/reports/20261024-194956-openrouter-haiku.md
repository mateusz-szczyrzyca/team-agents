# Team report — 20261024-194956-openrouter-haiku (round 3 of 9)

PR: #11 (open, not draft), branch `agents/openrouter-haiku/20261024-155041`, head `395fc00`.
Team: `20261024-194956-team`. Coordinator: `openrouter-haiku`. Members: `codex-1`, `gemini-1`.
Quiz: none in this resume (the team passed it earlier).

## Who did what

| Agent | Work | Commits |
|---|---|---|
| openrouter-haiku | integration, investigator #26 and #27 fixes, docs, report | fce63ec, 5e648c0, 9fac166, d78f564, 6af4478, 5c1dca2, 395fc00 |
| codex-1 | B1 persistence, B6 cancellation, B7 clock regressions | 5b2b85a, 0c7895b, b15e5ce, 4b86a7f, 4cbe6e5, ae8eddb, 772192c, 96ac9fe |
| gemini-1 | B2 reports, transitions, offline requeue, expiry; B8 replay tests | 9b59c67, d89e196, aa28f1c, 51d640f, 7059e84 (51d640f and 7059e84 partly merged, see below) |
| openrouter-minimax (worker) | B9 full agentbus race suite (read only) | — |

## Investigations

- `#26` (inv-astra): M1 lead transition clock, M2 report replay, M3 terminal matching, M4 test history. Fixed in `5e648c0`, `96ac9fe`, `d89e196`. M4 (test history in `7f50f8d`, `6ff1635`) is a documented historical exception; history was not rewritten.
- `#27`: needs changes. Fixed:
  - M2 omitted-time replay can rewind a job: `9fac166` (RED), `d78f564` (GREEN). A state change without its transition time is refused, except the first start (`assigned` → `running`) and a terminal state, because a replayed time-less report could not be told apart.
  - discarded `r.get` errors in `job_registry_review_test.go`: `6af4478`.
  - the rule is documented in `docs/multi-instance.md` (`5c1dca2`).
  - R2 same-state payload overwrite (`running@2` branch A, `running@4` branch B, retry of `running@2` overwrote B): `afba5f4` (RED), `9c38f01` (GREEN). A same-state report may fill an empty branch, task or PR, but cannot replace a recorded one.
  - R3 and R4 (edits to already-committed tests: `7f50f8d`, `6ff1635`, and `6af4478`, which changes the `r.get` error handling in this PR's own review test): **not self-cleared**. They need the human's explicit decision; see Open issues.

## Verify gaps (before the investigator review)

- `agents/gemini-1/20261024-155041` had 2 commits not in the branch: `51d640f` and `7059e84`.
- Merged in `395fc00`, with these exceptions:
  - `job_registry.go` (`7059e84`): not taken. Its default `at = now` for time-less reports reopens the rewind of `#27`, and it drops the equal-instant and cumulative-`At` rules that existing tests pin. The terminal-result behaviour it covers is already on the branch.
  - `job_registry_supplement_test.go` (`51d640f`): not taken. It changes an existing test (the "different At" terminal retry case is moved out of its function); AGENTS.md rule 1 forbids that.
  - `job_report_regression_test.go` (`51d640f`): merged without three tests that contradict existing rules: a zero-time `running` → `waiting`, a same-instant `waiting`, and a cumulative report with a differing `At`. The terminal replay test (`TestLeadRegistryTerminalReportReplayWithEmptyResultAfterPriorResult`) is kept; it passes.
- Not merged: the `GetUnknownJob` test from `51d640f` (it sat in the forbidden supplement file). It can be re-added as a new test file if wanted.

## Checks

- `make -s gate`: rc=0, all 6 steps passed (on the merged tree before the merge commit, and on `395fc00` for the docs).
- `go test -race` on the job tests: passed (before the merge commit).
- After R2 (`9c38f01`): `make -s gate` rc=0, 6 steps passed; `go test -race` on the job tests passed.
- Round size: `2,055` lines (`+2,024` / `−31`) in `19` files. Above the 2,000 soft limit, under the 3,000 hard limit.

## Not done

- codex-1 re-check of the M2 probe against `5c1dca2` (requested; result not yet returned).
- Board B3–B5 remain `easy` and `todo` (baseline gate on origin/main, skip inventory, locating helpers). They do not block round 3.
- Rounds 4–9 (HTTP listener, Tailscale identity, node daemon, commands, delegation, web interface). They need the human's approval of the plan first.
- The human merges PR #11; the team resumes from the merged base.

## Open issues

- The time rule (refuse time-less non-terminal changes) is stricter than gemini-1's design. The human should confirm it.
- Question for the human (R3/R4): authorize the repair of tests that were edited in committed history? Options:
  - a) accept the current state: M4 (`7f50f8d`, `6ff1635`) stays a documented historical exception; `6af4478` stays as is.
  - b) authorize a narrow repair: restore the earlier executable bodies of the edited tests and move the added coverage into separate tests. The history is not rewritten.
  - Recommended: b), per codex-1's review of investigator #27 (no edit to an existing test without an explicit verdict).
- Round size is above the 2,000 soft limit (2,055); under the hard limit. Splitting the PR would need the human's decision.
- Three of gemini-1's tests are not in the branch; if the human wants them, their expectations must be revised to match the existing rules.
