# Investigation: `--investigation` completion and compatibility

Investigator: `inv-astra`
Task: `20261102-201619-inv-astra`
Base: `facf3b8bb4ee30cbde86af771734751544c59217`

## Findings

The supplied base already implements the five requested behaviors. This patch
closes a remaining upgrade/recovery gap instead of duplicating those changes.

| Request | Evidence in the supplied base |
|---|---|
| Allow one investigator | `validate_investigation_team` accepts at least one name. `investigation_phase_pick` permits the sole investigator to review in a separate child run. `investigation_single_pipeline.bats` exercises the complete pipeline. |
| Report successful completion as done | `investigation_handoff` uses the recorded result for the parent status and notification. A completed review with `needs-fixes` or `incomplete` is distinct from a failed execution or missing verdict. |
| Do not wait for a decision after completion | New Phase 4 handoffs finish as `done` or `failed` and clear the lease. Interrupted pending handoffs recover. Already-delivered legacy handoffs were missed; see below. |
| Hide synthetic agents on the main dashboard | `isInvestigationTeam` filters synthetic tasks from the dashboard, live panel, and watcher events. `Snapshot.All` retains them for lookup. Real investigators remain visible, including real workers with similar names. Existing dashboard and watcher tests cover this. |
| Permit clarification questions | `inv-work.md`, `inv-merge.md`, and `review.md` permit `STATUS: QUESTIONS`. Independent runner handling preserves question files and pauses the child; resume retains isolation. Existing tests cover investigator, merger, and reviewer clarification. |

Relevant earlier commits already in this base include `6c1f345` (completion and
dashboard filtering) and `03da11a` (complete sole-investigator review). Applying
this incremental patch to a substantially older installation is insufficient;
it targets the supplied base. The installed runtime was not changed or audited.

## Reproduced defect

The original Phase 4 handoff implementation (`4279150`) recorded:

```text
STATUS=waiting
INVESTIGATION_PIPELINE=1
INVESTIGATION_PHASE=4
INVESTIGATION_LEASE=
INVESTIGATION_HANDOFF_PENDING=
INVESTIGATION_HANDOFF_RESULT=done
INVESTIGATION_COMMENTED=1
INVESTIGATION_NOTIFIED=1
```

Failed executions recorded `INVESTIGATION_HANDOFF_RESULT=failed` but also ended
as `waiting`. The current dispatch loop ticks parents only when a lease or
pending handoff exists. Supervision also misses this completed legacy state.
Consequently, upgrading code alone leaves those runs waiting indefinitely.

The new regression tests reproduce both outcomes before the fix: supervision
leaves a successful parent waiting, and dispatch leaves a failed parent waiting.

## Patch

`investigation_completed_wait` recognizes a waiting pipeline parent in Phase 4
with an explicitly recorded `done` or `failed` result. Both dispatch and
supervision use it to revisit that parent through the existing handoff path.
The path normalizes old decision-wait stage text and publishes the recorded
terminal status. Existing delivery markers prevent repeated notifications and
PR comments; no agent run or worktree is created.

Earlier phases, clarification/capacity waits, stopped parents, and absent or
unknown outcomes are not inferred to be successful completions. Existing
metadata and manifest formats, CLI arguments, MCP interfaces, and exit codes
are unchanged. No new configuration, dependencies, or log events are needed.

CLI help, README command examples, and the design's old idle-wait description
now describe completion and show the single-investigator invocation.

## Validation

- RED commit: `26c122d`. The two new legacy completion assertions failed on the
  supplied base, while both defensive tests passed. The full RED gate passed
  existing tests. Initial ShellCheck quoting findings in the new test were
  corrected, and `make -s lint-shell` then passed.
- GREEN targeted check: `bats tests/unit/investigation_completed_legacy.bats`:
  all four tests passed without changing the committed tests.
- Full `make gate`: all six steps passed (generated files, shell lint, Go lint,
  documentation lint, Go tests, and Bats tests).
- One initial GREEN gate run hit an unrelated failure in
  `tests/unit/delegate_jobs.bats:241`, checking a maximum-size escaped helper
  request. The exact test passed unchanged in isolation. The complete gate was
  rerun unchanged and passed; no timing limits, tests, stubs, or checks were weakened.
- Patch validation: applied `inv-astra.patch` to an isolated Git index loaded
  from the immutable base and verified that every patched file exactly matches
  this branch's corresponding file.

Tests use temporary homes, stub model CLIs, local repositories, and the existing
local test servers. No real model CLI, live installation restart, or hardware
test was used. No existing tests were changed. No skips or lint suppressions
were added. All required tools were available.

## Deliverable and integration

[`inv-astra.patch`](inv-astra.patch) is the final combined code, documentation,
and regression-test diff against the immutable base. It excludes this report
and the patch file itself. In a clean checkout of that base, apply it with:

```sh
git apply --check /path/to/inv-astra.patch
git apply /path/to/inv-astra.patch
make gate
```

The branch retains separate RED and GREEN commits for integration that
preserves test-first history. No pull request was opened. Deployment and live
manual verification remain with the human/merger after integration.

## Questions and blockers

No clarification is needed for the requested behavior. No notebook entry was
attempted because this independent investigation has no agentbus MCP access.
