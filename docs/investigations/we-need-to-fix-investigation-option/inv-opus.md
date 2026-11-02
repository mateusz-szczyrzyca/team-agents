# `--investigation` fixes — findings of inv-opus

Base: `facf3b8` (`main`). Patch: [`inv-opus.patch`](inv-opus.patch) (`git am` on `facf3b8`), the commits
`RED: Test names of independent investigators and hand-off label` and
`Name independent investigators and label the investigation hand-off`.

## Summary

Most of the topic is already fixed on `main` (commits `e6bd1a4` … `03da11a`, PR #40). The installed
`~/.local/bin/agent` (09:34) is **older than `main`**: it predates those fixes, so a run started by the installed
script still shows the old behaviour (for example, the earlier cohort `20261102-094135-investigation-team` stored
`INVESTIGATION_HANDOFF_STAGE=waits for you: merge failed`). Points 1, 2, 3 and 5 only reach the human once `main`
is installed. I found and fixed what was still wrong on `main`: how independent investigators are *named* on the
dashboard, in the panel and in the CLI (point 4), and the `--autopilot` label on the investigation's PR comment
(points 2/3: the message describes the wrong flow).

## Point by point (state of `main` at `facf3b8`)

| # | Requirement | On `main` | Evidence |
|---|---|---|---|
| 1 | One investigator allowed | ✅ already | `validate_investigation_team` needs `count >= 1`; `investigation_phase_pick` lets a sole investigator merge **and** review (separate child run). Tests: `investigation_single_investigator.bats`, `investigation_single_pipeline.bats` |
| 2 | Finished flow is `done`, not `failed` | ✅ already; label fixed here | `investigation_handoff … done` for every verdict (`ready-to-merge`, `needs-fixes`, `incomplete`); `failed` only for a failed merge or prechecks after the bounce and the replacement, a review without a verdict, or all phase‑1 runs failing. The PR comment still said `--autopilot complete: …` → now `--investigation complete: …` |
| 3 | No waiting for a decision at the end | ✅ already | `investigation_handoff` strips `waits for you:` (also from interrupted legacy hand-offs) and sets `STATUS=done/failed`; it calls `auto_stage … no-comment`, so no "decision" comment. Test: `investigation_terminal_completion.bats` |
| 4 | Main dashboard: real agents only | ✅ team row filtered already; **naming fixed here** | `isInvestigationTeam` filters `KIND=investigation_team` from `GET /`, `snap.Tasks` and watcher events; `/teams` keeps it. Still wrong: investigators shown as `investigator # (inv-opus)` / `investigator #20261102-…-inv-opus (inv-opus)` on the web, `inv#` in the panel, `inv#?` in `agent status`, a dangling `for` stage, `agent show`: `== investigation # for  (task …)` |
| 5 | Investigators may ask questions | ✅ already | `inv-work.md`, `inv-merge.md`, `review.md` allow `STATUS: QUESTIONS`; the runner sets the child to `waiting` and notifies. Tests: `investigation_clarification.bats`, `investigation_pipeline_clarification.bats` |

## Bugs fixed in the patch

1. **Independent investigators were named like requested investigations.** Children of an investigation team
   have `KIND=investigation` but no `INV_ID` (phase 1) or `INV_ID=<their own task ID>` (phases 2/3), and no
   `FROM_AGENT`. `live` labelled every `KIND=investigation` task `inv#<INV_ID>` / `investigator #<INV_ID> (agent)`
   and prefixed the stage with `for <FROM_AGENT>`.
   - `live/data.go`: new `Task.InvestigationTeam` (from `INVESTIGATION_TEAM`), `Task.Independent()`,
     `Task.forAgent()`. `Label()` returns the agent name for independent investigators. The web row
     (`makeRow`), the panel row (`ui.go`) and the watcher's "new task" event add `for <agent>` only when an agent
     requested the investigation.
   - `bin/agent`: `agent status` shows the agent name (and clears `INVESTIGATION_TEAM` before sourcing older
     `meta.env` files, so it is never inherited); `agent show` prints `== investigation team <id> (phase N)`.
   - Legacy `request_investigation` rows are unchanged (`investigator #10 (claude-inv)`, `inv#10`, `for solo`).
2. **The investigation hand-off PR comment said `--autopilot`.** `human_on_pr` hard-coded the word.
   It now takes an optional mode (default `--autopilot`, so `--autopilot` callers are unchanged), and
   `investigation_handoff` passes `--investigation`. The owner is still mentioned (`--body @owner …`, as the
   existing `investigation_team_handoff.bats` requires).

## Tests

- New `live/web_dash_independent_investigator_test.go`: `TestIndependentInvestigatorsShowTheirNames`
  (web `GET /`, `Label()`, panel row; the legacy investigation still shows `investigator #10 (claude-inv)` and `for solo`),
  `TestWatcherNamesNewIndependentInvestigators`.
- New `tests/unit/investigation_independent_labels.bats` (full one-investigator pipeline with the existing stub
  fixture): `agent status` rows say `inv`, `agent show` names the team and phase, and the PR comment is
  `--body @example --investigation complete: review ready (#73)` with no `--autopilot`.
- RED runs are in the RED commit message. `make lint test` on the fix: 0 lint issues, every Go and bats test passes (442 bats).
- No existing test was changed.

## Compatibility

- No on-disk format changes. `INVESTIGATION_TEAM` was already written by `bin/agent`; `live` now reads it, and an
  older task without it is shown as before.
- `agent status` output: the AGENT column of independent investigators changes from `inv#…` to the investigator's
  name (same columns). `agent show` output for those tasks changes its investigation heading line. README updated.

## Not changed (open questions for the human)

1. **A review without a `VERDICT:` line ends the flow as `failed`** (`failed: review failed`). `main` pins this
   in `investigation_single_pipeline.bats` ("sole investigator review without a verdict still fails the completed
   flow"), so I did not change it (AGENTS.md rule 1). If a published review without a verdict line should count
   as `done`, that test needs to change, and only you can approve that.
2. **Install `main`.** The running installation is older than the fixes for points 1–3 and 5; until it is
   reinstalled, `--investigation` behaves as before. I did not reinstall it (AGENTS.md rule 3).
3. The investigation's final notification has no `PR:` line. The PR is only named in the stage text
   (`complete: review ready (#73)`). A `PR: <url>` line would help; I left it out to keep the patch small.
