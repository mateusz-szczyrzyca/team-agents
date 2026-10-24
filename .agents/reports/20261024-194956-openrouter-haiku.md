# Team report — 20261024-194956-openrouter-haiku (round 3 of 9)

PR: #11 (open, not draft), branch `agents/openrouter-haiku/20261024-155041`, head `395fc00`.
Team: `20261024-194956-team`. Coordinator: `openrouter-haiku`. Members: `codex-1`, `gemini-1`. No quiz in this resume.

## Who did what

| Agent | Work | Commits |
|---|---|---|
| openrouter-haiku | integration, investigations #26 and #27 fixes, docs, report | fce63ec, 5e648c0, 9fac166, d78f564, 6af4478, 5c1dca2, 395fc00 |
| codex-1 | B1 persistence, B6 cancellation, B7 clock regressions | 5b2b85a, 0c7895b, b15e5ce, 4b86a7f, 4cbe6e5, ae8eddb, 772192c, 96ac9fe |
| gemini-1 | B2 reports, transitions, offline requeue, expiry; B8 replay tests | 9b59c67, d89e196, aa28f1c, 51d640f, 7059e84 (last two partly merged) |
| openrouter-minimax (worker) | B9 full agentbus race suite (read only) | — |

## Investigations

- `#26` (inv-astra): M1 lead transition clock, M2 report replay, M3 terminal matching, M4 test history. Fixed in `5e648c0`, `96ac9fe`, `d89e196`. M4 (`7f50f8d`, `6ff1635`) is a documented historical exception; history not rewritten.
- `#27` needs changes. Fixed:
  - M2 omitted-time replay rewinds a job: `9fac166` (RED), `d78f564` (GREEN). A state change without its transition time is refused, except the first start (`assigned` → `running`) and terminal states.
  - Discarded `r.get` errors in `job_registry_review_test.go`: `6af4478`. Rule documented in `docs/multi-instance.md` (`5c1dca2`).
  - R2 same-state payload overwrite: `afba5f4` (RED), `9c38f01` (GREEN). A same-state report may fill an empty branch, task or PR, but cannot replace a recorded one.
  - R3 and R4 (edits to committed tests `7f50f8d`, `6ff1635`, and `6af4478` in this PR's review test): **not self-cleared**; they need the human's explicit decision (see Open issues).

## Verify gaps

- `agents/gemini-1` had `51d640f` and `7059e84` not in the branch. Merged in `395fc00`, except:
  - `job_registry.go` (`7059e84`): not taken. Its default `at = now` reopens the rewind of `#27` and drops equal-instant and cumulative-`At` rules that existing tests pin.
  - `job_registry_supplement_test.go` (`51d640f`): not taken. It changes an existing test; AGENTS.md rule 1 forbids that.
  - `job_report_regression_test.go` (`51d640f`): merged without three tests that contradict existing rules (zero-time `running` → `waiting`, same-instant `waiting`, cumulative report with differing `At`). The terminal replay test is kept.
- The `GetUnknownJob` test from `51d640f` (in the forbidden supplement file) is not merged; it can be re-added as a new file.

## Checks

- `make -s gate`: rc=0, all 6 steps, before the merge commit and on `395fc00` for the docs. After R2 (`9c38f01`): rc=0, 6 steps. `go test -race` on the job tests passed.
- Round size: `2,055` lines (`+2,024` / `−31`) in `19` files at that point. Above the 2,000 aim, under the 3,000 hard limit.

## Not done

- codex-1 re-check of the M2 probe against `5c1dca2` (requested; not yet returned).
- Board B3–B5 (`easy`, `todo`) do not block round 3.
- Rounds 4–9 need the human's approval of the plan first.

## Open issues

- The time rule (refuse time-less non-terminal changes) is stricter than gemini-1's design. The human should confirm it.
- Question for the human (R3/R4): authorize repair of tests edited in committed history?
  - a) accept the current state: M4 stays a documented historical exception; `6af4478` stays as is.
  - b) authorize a narrow repair: restore the earlier bodies of the edited tests and move the added coverage into separate tests. History not rewritten.
  - Recommended: b), per codex-1's review of investigator #27.
- Round size is above the 2,000 aim (2,055); under the hard limit.
- Three of gemini-1's tests are not in the branch; if wanted, their expectations must be revised to match the existing rules.
