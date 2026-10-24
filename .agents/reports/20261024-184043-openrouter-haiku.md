# Report — 20261024-184043-openrouter-haiku (round 3 of 9)

Coordinator `openrouter-haiku`. Branch `agents/openrouter-haiku/20261024-155041`, PR #11 (open, not a draft).

## Who did what

- `codex-1`: B1 persistence (`5b2b85a`, `0c7895b`), B6 cancellation (`b15e5ce`, `4b86a7f`, `4cbe6e5`, `ae8eddb`), B7 clock regressions (`772192c`, `96ac9fe`). Report `.agents/reports/20261024-184043-codex-1.md`.
- `gemini-1`: B2 reports, transitions, offline requeue, expiry (`9b59c67`, `d89e196`, `aa28f1c`, fixes in `392370a`), an ancestor of this branch.
- `openrouter-haiku`: integration, review fixes M1–M3 (`fce63ec` RED, `5e648c0` GREEN), merge of codex-1 (`05b9ec9`), PR description.
- Investigator `inv-astra`, request #26: review of `4ec12b5`, verdict "needs changes".

## Verify gaps

- `agents/codex-1`: `772192c` and `96ac9fe` merged in `05b9ec9`. Both implemented the same clock clamp (`job_clock.go`); kept one helper, `atOrAfterLast`, removed `job_clock.go`, kept codex-1's regression test and design note.
- `agents/gemini-1`: its tip `392370a` was already an ancestor; the count was stale.

## Investigator findings (#26)

- `M1` clock regression makes history unreadable: fixed `5e648c0`, test `TestJobCancelEarlierClockKeepsHistoryReadable`.
- `M2` scalar replay / repeated waiting: fixed `5e648c0`, tests `TestJobReportEqualTimeStateChangeNeedsCumulativeHistory`, `TestJobReportRepeatedWaitingIsNotANewTransition`.
- `M3` cumulative time and terminal payload: fixed `5e648c0`, tests `TestJobReportCumulativeAtMustMatchLastTransition`, `TestJobReportTerminalResultIsTheTerminalPayload`.
- `M4` test history (`7f50f8d`, `6ff1635` edited committed tests after implementation): **not fixed**; a documented historical exception in the PR. History is pushed and not rewritten.

## Checks

- `make gate` on `05b9ec9`: all 6 steps passed, rc=0. `go test -race` on the registry tests: clean.
- Round size: `1,841` lines (`+1,810` / `−31`) in `16` files at that point (limit 3,000, target 2,000).
- Existing tests unchanged.

## Member branches merged

- `agents/codex-1`: `822ce86` "Verify exact causes for rejected job report replays" (`agentbus/job_report_causes_test.go`, +68) merged.
- `agents/gemini-1`: `ba42d30` "Add round 3 member report for gemini-1" merged.

## Not done

- Investigator re-review of `05b9ec9`.
- Round 4 (HTTP listener, Tailscale identity, authorization): not started; rounds 4–9 to be planned and approved.
- Board B3–B5 (easy, todo): not done.
- Skipped tests: 16 remain, all for rounds 4–9.
