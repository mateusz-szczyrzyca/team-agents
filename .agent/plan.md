# Task task-20261124-221204-v08a — worker notes

## Step 15.2 (request by codex-1) — DONE

Read-only inventory of the owned settlement recovery tests and the file/storage helpers
in the requester's worktree `/home/dev/agents-rs-trial/worktrees/team-agents/task-20261123-214920-qskl`
(branch `agents/codex-1/task-20261123-214920-qskl-1.2b`, HEAD `d4d18ce58 Release 0.92.0 with fenced worker settlement`).

Files read (only the named ones): `agentbus/helper_owned_test.go` (196 lines),
`agentbus/helper_owned.go` (226 lines), `agentbus/main.go` (1709 lines, relevant parts).

Nothing in the requester's worktree was changed; no test was run.

## Inventory

### Recovery test (the one review fix 2 needs)

`TestOwnedReceiptRepairsAnInterruptedStateSave` — `agentbus/helper_owned_test.go:95`

Fixture setup, step by step:

1. `b, cmd := ownedFixture(t)` (line 96) — the shared fixture.
2. `b.helperOwned(cmd)` with `cmd.Action == "start"` → expects `ownedApplied` (lines 97-99).
3. `before, serr := b.load()` (line 100) — snapshots the state *before* the settlement.
4. `cmd` becomes a settle: `Action "settle"`, `Status HelperDone`, `Summary "kept"`,
   `Notice "worker served the step"` (line 104); `b.helperOwned(cmd)` → `ownedApplied` (105-107).
   The notice is appended to `messages.jsonl` by `postTo`.
5. `b.save(before)` (line 109) — simulates the crash: `state.json` is rolled back to the
   pre-settlement state while `messages.jsonl` keeps the receipt.
6. `restarted := &Bus{dir: b.dir}` (line 112) — a second bus handle on the same directory.
   `restarted.load()` (line 113) runs the recovery (`restoreOwnedSettlements`).
7. Assertions: the reloaded request is settled (`Status HelperDone`, `Summary "kept"`, lines 117-119);
   replaying the same command returns `ownedReplay` (120-122); exactly one message exists and
   `loaded.NextID > messages[0].ID`, i.e. the message cursor was repaired too (127-129).

### Shared fixture helper

`ownedFixture` — `agentbus/helper_owned_test.go:6`

- `b := newBus(t)` — temporary bus directory (`agentbus/agentbus_test.go:18`).
- `mustWith(t, b, ...)` (`agentbus/agentbus_test.go:50`) seeds
  `st.Helpers["1"] = &Helper{ID: 1, From: "lead", FromTask: "requester-task", Worker: "worker", Status: HelperStarting}`.
- Returns `ownedCommand{ID: 1, Action: "start", Worker: "worker", Task: "worker-task", Run: "run-1"}`.

Other fixtures used by the sibling tests (not by the recovery test):

- `installState(t, helperBeforeWorkerResult)` — `agentbus/legacy_test.go:19` with the legacy
  fixture `helperBeforeWorkerResult` (`agentbus/helper_worker_result_test.go:18`), used by
  `TestOwnedOperationLeavesLegacyOpenRequestsUnfenced` (`helper_owned_test.go:140`).
- `captureLogs(t)` (`agentbus/logging_test.go:15`) and `records(t, logs)` (`agentbus/logging_test.go:25`),
  used by `TestOwnedSuppressedResultsLogExecutionIdentity` (`helper_owned_test.go:171`).
- `failsWith(t, cause)` (`agentbus/agentbus_test.go:89`) for the cause assertions.

### All tests in `agentbus/helper_owned_test.go`

| Line | Test |
|---|---|
| 15 | `TestOwnedSettlementPostsOneNoticeAndRejectsLateResults` |
| 47 | `TestOwnedRequestRetryFencesTheEarlierExecution` |
| 72 | `TestOwnedCommandErrorsKeepTheirCauses` (table-driven, 7 cases) |
| 95 | `TestOwnedReceiptRepairsAnInterruptedStateSave` — the recovery test |
| 132 | `TestOwnedCommandReportsStorageFailure` (bus dir under `state.json/child` → lock fails) |
| 140 | `TestOwnedOperationLeavesLegacyOpenRequestsUnfenced` |
| 151 | `TestOwnedRetryReplayKeepsTheQueueAndDoesNotPost` |
| 171 | `TestOwnedSuppressedResultsLogExecutionIdentity` |

### Recovery code (`agentbus/helper_owned.go`)

- `restoreOwnedSettlements(st *State) *StorageError` — line 119.
  Opens `messages.jsonl` (130), returns early when the file is missing and the cursor is 0 (131-133),
  validates the cursor against the file size (146-148), seeks to `st.OwnedReceiptOffset` (149),
  reads the suffix line by line (154-172), skips a corrupt line with `warnOnce` (161),
  applies each receipt (162), advances the cursor (176) and saves the state under the lock (179).
- `restoreOwnedReceipt(st *State, m Message) *StorageError` — line 183.
  Repairs `st.NextID` (184-186), ignores messages that are not a script from `agent` (187-189),
  ignores events without `NoticeHelperResult` / `owned_request` (194-196), ignores a receipt whose
  run/task/worker no longer own the request (197-200), parses the status (201-204) and rejects a
  nonterminal status (205-207), then writes `h.Status, h.Summary` (208).
- `helperOwned` (line 58) is the writer side: the receipt is appended by `b.postTo` (103) and the
  request is updated after it (106) — the receipt is the commit point (comment at 98).
- `ownedCommand` 14, `ownedOutcome` 26, `ownedApplied/ownedReplay/ownedStale` 29-31,
  `ownedAction` 35, `ownedStart/ownedRetry/ownedSettle` 38-40, `valid()` 44, `cmdHelperOwned` 213.

### File / storage helpers (`agentbus/main.go`)

| Line | Helper | Note |
|---|---|---|
| 134 | `State.OwnedReceiptOffset int64` (`owned_receipt_offset`) | bytes already replayed from `messages.jsonl`; absent in older states |
| 157 | `(*Bus).path(n string) string` | absolute path inside the bus directory |
| 161 | `(*Bus).load() (*State, *StorageError)` | reads `state.json`, initialises nil maps, **calls the recovery at line 198** |
| 205 | `(*Bus).save(st *State) *StorageError` | atomic write: `state.json.tmp` + rename |
| 221 | `(*Bus).withLock(fn)` | `bus.lock` flock |
| 247 | `(*Bus).locked(fn)` | load → gc → fn → save under the lock |
| 261 | `transact[C opCause](b, storage, fn)` | operation wrapper: lock/load/save failures become the storage cause |
| 319 | `(*Bus).keepWhole(id, text)` | long-text companion file |
| 366 | `(*Bus).post(st, from, to, ev)` | unbound post |
| 371 | `(*Bus).postIn(st, team, from, to, ev)` | team-bound post |
| 376 | `binding` struct | `team` / `task` |
| 382 | `(*Bus).postTo(st, bind, from, to, ev)` | appends to `messages.jsonl` (401), `st.NextID++` (404) |
| 412 | `(*Bus).messages() ([]Message, *StorageError)` | reads the whole log, skips a corrupt line (434) |

Helpers used by the recovery code but defined elsewhere:
`appendFile` (`agentbus/invest.go:186`), `parseEnum` (`agentbus/types.go:295`),
`storageErr` (`agentbus/errors.go:241`), `valueErr` (`agentbus/errors.go:272`),
`helperKey` (`agentbus/collab.go:1129`).

## Next session

- If codex-1 asks for more: the adjacent file `agentbus/helper_receipt_cursor_test.go`
  (4447 bytes, present in the requester's worktree) was deliberately NOT read; open it if the
  cursor/recovery test inventory must be complete.
