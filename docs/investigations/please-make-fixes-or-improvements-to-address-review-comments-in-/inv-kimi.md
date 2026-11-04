# PR #58 review fixes — inv-kimi

## Result

Implemented the requested fixes for [#58](https://github.com/mateusz-szczyrzyca/team-agents/pull/58) on
`agents/inv-kimi/20261104-143442`, using the reviewed head
[`d52b688`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d52b6886a8d781d64e0bb16c5ff6fd3d8e2c509e)
without the later unrelated merge of `main`.

- [`822f7b5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/822f7b56afe17291ad91073d3ec476b19ef4548b) — imported the PR baseline and passed the gate.
- [`c888ad7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c888ad7d870a294280389f2a36d7b9a5087ed9b4) — $\color{red}{\textbf{RED}}$: added `5` tests; the `3` tests for unimplemented behaviour failed for the intended reasons.
- [`a38c92a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a38c92a49fddbeed429283aa2542593180fec515) — $\color{green}{\textbf{GREEN}}$: implemented the fixes and passed the full gate.

The review input was authored by `ms-ai-agents`; no text from another account was used as instructions.

## Review findings addressed

### Major 1 — member checkpoints were write-only and left behind by archives

- [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a38c92a49fddbeed429283aa2542593180fec515/bin/agent) now packs every grouped task's own `task-<id>.md` checkpoint and removes it only after the archive passes its integrity test.
- [`agentbus/collab.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a38c92a49fddbeed429283aa2542593180fec515/agentbus/collab.go) now tells a member that its own record is archived and is not carried into a resume.
- [`agentbus/collab.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a38c92a49fddbeed429283aa2542593180fec515/agentbus/collab.go) drops the unreachable `member-<team>-<agent>` fallback; a team run always has a task ID.
- [`README.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a38c92a49fddbeed429283aa2542593180fec515/README.md) and [`prompts/team.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a38c92a49fddbeed429283aa2542593180fec515/prompts/team.md) document the lifecycle explicitly.
- A new bats test proves that a member checkpoint is present in the `7z` archive and gone from the live checkpoint directory.

### Major 2 — coordinator `WORK_ROUND` selection had no resume test

- A new same-round resume test gives the old member `WORK_ROUND=3` and the coordinator `WORK_ROUND=2`, then asserts `## Round 2 of 5 continues` and rejects a Round `3` heading.
- The existing implementation passed this characterization test in the RED commit; no production change was needed for this finding.

### Minor 1 — `gh api` bypassed the member PR guard

- [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a38c92a49fddbeed429283aa2542593180fec515/bin/agent) reuses the live bus-backed member/coordinator decision for REST API writes.
- It refuses `pulls/N/merge`, explicit non-`GET` methods on `pulls` and `pulls/N`, and field- or input-implied `POST` requests on those endpoints.
- It handles options before or after the endpoint, including `--method`, `-X`, raw fields and `--input`.
- The test covers merge, close, create, implicit `POST`, and `--method=PUT`; it also proves that a member's `GET` and a non-member's mutation still reach `gh`.
- Refusals use the existing `github_member_pr_refused` event with `action=api`.

### Minor 2 — checkpoint broadcast and pre-election behaviour lacked assertions

- A new Go test asserts that a non-coordinator checkpoint adds no bus message and wakes no task.
- A new Go test asserts that a checkpoint before coordinator election still goes to `team-T.md`, creates no member task record, and broadcasts `KindCheckpoint`.
- The pre-existing checkpoint test was not changed.

### Minor 3 — direct pushes to another member's branch

No hook was added. The reviewer explicitly judged the existing prompt prohibition sufficient for this round and suggested enforcement as follow-up work.

## Checks

```bash
cd agentbus && go test -run 'TestMemberCheckpoint(DoesNotPolluteTeamCheckpoint|IsTaskRecordWithoutBroadcast|BeforeCoordinatorUsesTeamRecord)$' ./...
bats tests/unit/member_in_team.bats
env -u AGENT_GITHUB_NAME_COLORS -u AGENT_GITHUB_COLORS make gate
```

- Targeted Go tests: passed.
- [`tests/unit/member_in_team.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/a38c92a49fddbeed429283aa2542593180fec515/tests/unit/member_in_team.bats): `12/12` passed.
- Full gate: `7/7` steps passed.
- Bats: `564/564` passed.
- Harness: `20/20` passed.
- Lint: `0` findings.
- Review-fix commits: `225` lines (`+196` / `−29`) in `7` files.
- Existing tests changed: `0`; new tests added: `5`.
- New dependencies and lint suppressions: `0`.
- Two intermediate gate attempts hit unrelated timing-sensitive failures in `resuming a team with a remote member brings the member back on its node` and `a leased parent without manifest keeps the lease while its tagged child is still running`. Each failed test passed when rerun alone, and the subsequent full gate passed `7/7`; no test or production workaround was made. These may merit a separate flake investigation if they recur.

## Compatibility

- Checkpoint file formats and naming remain unchanged; archives now retain the member task records that were previously omitted.
- Member task checkpoints are removed only after a successful archive integrity check.
- No resume behaviour was changed: the new documentation and tool answer state the existing rule that only the coordinator's team checkpoint is carried into a team resume.
- The API restriction applies only to non-coordinator team members. Reads, coordinators, solo tasks, investigators, and non-members retain their previous access.

## Skipped tests

- None during verification. The new archive test retains the repository's conditional `7z not installed` skip for hosts without the required archiver; `7z` was installed here, so the test ran and passed.

## Not done

- No real agent CLI or integration suite was run.
- No installation or running service was changed.
- No direct-`git push` hook was added; this remains optional follow-up work, as stated above.
