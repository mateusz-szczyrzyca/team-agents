# Round accounting — independent investigation

Investigator: `inv-astra`.

Base: [`45f1e7a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/45f1e7a29ebd2797d3112e39e129bfe938236604).

## Result

The requested accounting can be implemented in the Bash runner without changing bus state or adding dependencies. The work needs explicit creation-time attribution, a deduplicated descendant traversal, and run attribution layered onto the existing usage reducer. Re-parsing generated task text or recomputing a child's attribution from its live parent would produce incorrect history.

This is a Phase 1 investigation deliverable, not the implementation. The investigation instructions explicitly prohibit opening a PR; the two implementation PRs belong to the subsequent integration work, with Part 2 after Part 1 merges. No production code, existing tests, helpers, stubs, or repository instructions were changed.

Serena was unavailable in the session's tool catalog. Code discovery used shell searches. No other investigators, worktrees, real agent CLIs, hardware devices, or running services were accessed for the investigation.

## Findings that affect implementation

### 1. Six metadata constructors need coverage, plus their callers

There are `5` literal `ROUND=0` writers and `1` additional writer using `meta_set ... ROUND 0`. Searching only for the literal misses remote proxies.

| Constructor in the base | Creation-time source for `ROUND_ID` |
|---|---|
| [`create_task`, line 6792][create] | Explicit resolved ID: public task text for normal starts; inherited ID for internal callers. |
| [`resume_run`, line 7026][resume] | Validate the new addition alone; use its ID if present, otherwise the previous team's ID. Include joiners and replacements. |
| [`start_helper`, line 5940][helper] | Snapshot `from_task`, not the worker's generated request/context text. |
| [`start_investigation`, line 5782][investigation] | Snapshot `from_task`; the same constructor also makes reviews. |
| [`cmd__investigation_team`, line 5704][cohort] | Public investigation topic; an explicitly parented cohort must use its requester. |
| [`remote_member_task`, line 8247][proxy] | The resolved round of the originating start/resume, before any job submission. |

The shared constructor has additional callers that must pass attribution deliberately:

- [`investigation_team_admit`][admit] creates Phase 1 investigators, then sets `PARENT_TASK` and `INVESTIGATION_TEAM` to the cohort parent.
- [`investigation_phase_start`][phase] creates the merger and reviewer. Phase `3` sets `REVIEW=1`.
- [`investigation_resume_admit`][advisory] creates Phase `5` advisory reviewers and sets `REVIEW=1`.
- [`prepare_job_task`][node-create] creates node tasks and then adds job/team identity.
- [`cmd_run`][run] creates ordinary local tasks and remote proxies.

[`supervise_failovers`][failover] reaches normal replacement creation through `run --resume ... --same-round --team ...`. Helper failover requeues the request and reaches `start_helper`. [`coordinator_takeover`][takeover] changes the lead among existing members; it does not create a task. Do not invent a constructor there or relabel historical tasks during takeover.

The current public investigation cohort constructor has no requester argument and writes no `PARENT_TASK`; queued bus investigations use the single-investigation constructor instead. Cover parented cohort graphs with artificial fixtures, but do not silently invent a new delegation API to satisfy that graph test.

### 2. Validate input before allocation, but do not revalidate composed history

[`cmd_run`][run] has separate text ingestion paths for ordinary runs, investigations, and node jobs. [`resume_run`][resume] reads its addition separately, then composes old text plus additions and generated round headings. Validate the fresh input before task IDs, worktrees, bus jobs, or units are created.

Recommended parser contract:

- Recognize the exact line prefix `Round:` and trim surrounding blanks from its value.
- Validate the entire value with `^[A-Za-z0-9._-]+(/[A-Za-z0-9._-]+)*$` using ASCII semantics.
- Return both whether a declaration was present and its resolved value. No declaration resolves to `?`; an empty declaration is an error, not absence.
- Diagnose the offending source line and line number. A duplicate is invalid even if both IDs match.
- Do not interpret task text as shell code or use a glob/regular expression for descendant matching.

A resumed task can legitimately contain the old declaration in retained historical prose and a new declaration in its addition. Parsing the combined result would reject a valid resume. Resolve once at the boundary and pass the ID into constructors. Internal children must inherit metadata even when their generated text contains a different or duplicate `Round:` line.

Investigation advisory resume is a separate route through [`investigation_resume`][advisory-resume], reusing the existing cohort parent and writing a resume request. It needs an explicit policy for an addition that declares a new ID: store attribution for the new advisory cohort without rewriting already-created children. Test this route separately from ordinary resume.

### 3. An absent field is not reliably empty with the existing accessor

[`task_field`][meta] sources metadata in a subshell without first unsetting the requested variable. Thus an older task with no `ROUND_ID` can appear to have the caller's ambient `ROUND_ID`. Existing [`investigation_field`][inv-field] explicitly unsets a field for this reason.

Use an accessor that distinguishes a persisted field from ambient state. Preserve this distinction:

- Old task with no `ROUND_ID`: leave it unlabelled and exclude it from `--round ?`.
- New task with an explicit persisted `ROUND_ID=?`: include it in `--round ?`.
- New child of an unlabelled old task: write `ROUND_ID=?` without modifying its parent.

The metadata writer uses Bash `%q`. `?` is shell-escaped on disk; raw `sed`/`grep` values are not always decoded values. Test the actual `meta_set` → reader round trip, alongside literal artificial fixtures.

### 4. The current team query can both omit and duplicate spending

[`cmd_tokens`][tokens] seeds tasks by exact `TEAM`, then scans only their immediate `PARENT_TASK` children. It neither follows investigation links nor deduplicates seeds against children. It also admits a direct child carrying another team's explicit `TEAM`.

Use a snapshot of task metadata and a queue/set traversal:

1. Seed every task with `TEAM` equal to the requested team.
2. Add descendants whose `PARENT_TASK` or task-valued `INVESTIGATION_TEAM` references a selected task.
3. Reject a candidate with a nonempty, different `TEAM`; do not traverse through it.
4. Mark IDs visited before enqueueing; produce each task once, in deterministic order.
5. Treat missing references as absent edges and preserve zero-usage tasks in the report.

`INVESTIGATION_TEAM` is overloaded: a cohort parent's value is a comma-separated investigator roster, while a child's value is its parent task ID. Check the record kind and exact task reference; never turn a roster into graph edges.

Do not follow `RESUMED_FROM` for a team query: that would add another team's spending. Round queries are what combine resumed teams.

Do not casually replace [`team_related_tasks`][related] with the new traversal. It is also used by pause/unpause; changing it broadens operational behavior outside accounting. It currently performs a single scan with an expanding set, so its depth depends on filename order and is not a correct reusable closure.

### 5. Role and model grouping need explicit semantics

Recommended role precedence is `REVIEW=1` → `review`; `KIND=helper` → `worker`; investigation/cohort kinds → `investigation`; team lead task → `coordinator`; remaining team tasks → `member`. This correctly distinguishes a worker-tier team member from a delegated worker.

Use the bus's lead/member-task mapping, not an agent name containing `lead` or `coordinator`. Read each team's mapping once per report. A takeover changes which task is the current coordinator; the specification requests task roles, not historical role splits per run. Historical role accounting would need additional data and is outside these two parts.

Two display policies are not specified by the role list: ordinary solo tasks and a team whose lead record is unavailable. Recommended integration choice: solo tasks count as `coordinator`; otherwise unresolved team tasks count as `member`. Document and test the chosen behavior before shipping.

[`task_model`][model] already supplies a fallback chain for missing `MODEL`, but it consults current configuration. Prefer the persisted model for labelled tasks; document any fallback as an approximation. Do not introduce API price estimation into `tokens`: [`cmd_spend`][spend] separately distinguishes paid cost from subscription equivalents. Existing `COST` means cost reported by the CLI.

### 6. Keep the usage reducer's seven-column interface stable

[`runs_usage`][usage] already handles cumulative Codex usage, reset on `new`/`fresh`, ordinary usage records, and OpenCode step usage. Its output is `run, steps, in, cached, out, cost, mode`.

Its consumers include:

- [`cmd_tokens --runs` and task totals][tokens].
- [`fresh_session_due`][fresh], including mode at field `7`.
- [`round_card`][card], which sums field `3`.
- [`cmd_spend`][spend], which joins usage to [`run_days`][days].

Appending attribution columns to this internal output is unsafe without updating consumers: `read -r run steps in cached out cost mode` assigns all remaining columns to `mode`. The lowest-risk change is a separate run-attribution reader/map, leaving `runs_usage` unchanged. Do not create a second token reducer.

The single marker writer is in [`cmd__runner`, line 6218][runner]. Append ` round <id> pr <#n|->` after the existing `▶ run N (mode)` text. Both existing marker readers use prefix matches, so suffixes are compatible.

### 7. Snapshot the inherited PR; never read the live requester at report time

Children need a creation-time PR snapshot, including an explicit empty snapshot when their requester has no PR. A later parent change to `#75` must not move a child created for `#74`, nor move a child created before any PR under the new PR.

Reuse `ROUND_PR` for the snapshot if its ownership contract is documented and tested. If a separate field is needed to distinguish inherited from own PR, keep it additive and ensure every child constructor writes it, including empty values. Current reviewers often carry `PR_NUMBER` rather than `ROUND_PR`; normalize this deliberately during creation. A marker must use the task's applicable PR at run start, not a later lookup during reporting.

[`round_pr_supersede`][supersede] updates one task's `ROUND_PR`; it does not broadcast that value to all members. The requested rule is the task's `ROUND_PR` at run start. Do not silently expand the work into PR lifecycle propagation.

Join each usage row with its marker by task ID and run number. Avoid an inner join that drops run `0` or unmarked runs. Attribute:

- Valid round/PR marker → `<marker round>/#<n>`.
- Valid marker with `pr -` → the round's explicit `(no PR)` bucket.
- No attribution marker → `<task ROUND_ID>/?`.

Keep `(no PR)` distinct from missing evidence. Preserve task totals and do all grouping from the same unrounded numeric records; round cost only for display.

### 8. Round selection and remote synchronization have separate boundaries

Match exact IDs or the literal prefix `<query>/`. This includes `R12/A` and stored `R12/A/#74`, excludes `R120`, and avoids regex meaning in `.` or glob meaning in `?`. The task-input grammar excludes `#`; the requested synthetic `ROUND_ID=R12/A/#74` fixture tests report selection, not permission to accept that task-input declaration. Do not revalidate persisted report records against the narrower task-text grammar.

[`docs/multi-instance.md`][nodes] describes existing mirrors: activity is bounded to `2,000` lines and `256` KiB per artifact. [`agentbus/lead_sync.go`][sync] confirms that activity is a tail. Usage may appear in those mirrors, but there is no complete synchronized accounting stream and `tokens` reads local task activity only. The future-work document should make that distinction rather than claiming nodes send no activity.

The Part 2 start marker labels a run; it does not annotate every subsequent usage record. Later synchronization must send the start marker with its usage records, or send a self-contained derived per-run record. Sending only raw `usage:` lines loses round/PR attribution and, for Codex, the previous cumulative baseline. The future design also needs stable node/task/run identity, idempotent delivery, and protection against truncated tails. No task graph is needed once attribution accompanies each usage group.

## Suggested two-part delivery and tests

Part 1 should add the parser, creation-time propagation, accounting-only closure, role/model summaries, appended row columns, README documentation, and additive compatibility fixtures. Append `ROLE` and `ROUND` after the existing `TASK AGENT KIND IN CACHED OUT COST` fields. Retain the existing no-usage message and fill missing numeric positions consistently so appended values remain identifiable.

Part 2 should add PR snapshots, marker suffixes, sub-round summaries, the requested node future-work document/link, and tests for both new and old marker formats. Do not write skipped placeholders for Part 2 in Part 1.

Each implementation cycle must commit its new failing tests as $\color{red}{\textbf{RED}}$, then implementation as $\color{green}{\textbf{GREEN}}$, with `agent gate` before commits. Existing [`tests/unit/tokens.bats`][token-tests] remains untouched.

Recommended additional cases beyond the topic's acceptance list:

- Invalid declaration leaves no task, worktree, unit, or remote job; cover argument, file, stdin, and resume-addition paths.
- Duplicate identical IDs, empty value, leading/trailing slash, doubled slash, spaces inside a value, and ASCII-only parts.
- Descendants inherit metadata despite misleading declarations in generated child text.
- Empty inherited PR remains empty after the requester acquires a PR.
- Missing old metadata cannot inherit ambient shell state or enter the new unknown-round bucket.
- Same task reachable through both links and through the team seed appears once.
- Cycle reachable from a team terminates; a disconnected cycle remains excluded.
- Foreign explicit team blocks traversal even when its parent link points into the requested team.
- No bus lead record, coordinator takeover, solo task, and zero-usage cohort parent.
- Codex cumulative usage across different PR markers, including a fresh-session reset.
- Usage before any start marker remains in the unknown-sub-round bucket.
- Old start markers and suffixed start markers preserve `--runs`, fresh-session, card, and spend behavior.
- Role/model/sub-round totals reconcile with task totals, including cost precision and absent usage.

New Bats tests also need appended mapping entries in [`tests/agent-go-map.tsv`][test-map] and explicit shell-lint registration in [`Makefile`][makefile]. [`tools/check-agent-test-map.sh`][map-check] enforces one mapping per test. Those migration-ledger round values are separate from runtime `ROUND_ID`; do not change the ledger validator to accept runtime IDs. Add rows without editing existing tests or mappings.

## Checks

- `agent gate --wait`: PASS; all `8/8` steps passed on the unchanged implementation at the base commit. Checks included generated errors, shell/Go/doc lint, Go tests, unit Bats, harness tests, and the Go-agent Bats lane. No lint findings.
- `GATE_WAIT=1 tools/test-lock.sh round-accounting-probe bash <temporary-probe> <worktree>`: PASS, exit `0`; ran after the gate under the machine test lock. The probe sourced the base runner in a temporary home with empty agent configuration and synthetic metadata/activity; it invoked no external agents or bus operations.
- `git diff --check`: PASS.
- No production change was made, so a RED/GREEN implementation cycle is not applicable to this report.

### Reproduced accounting failures

The probe constructed this graph; all usage records were ordinary noncumulative input usage:

| Task | `TEAM` | `PARENT_TASK` | `INVESTIGATION_TEAM` | Input tokens |
|---|---|---|---|---:|
| `root` | `T-team` | — | — | `100` |
| `helper` | empty | `root` | — | `10` |
| `cohort` | empty | `root` | investigator roster | `0` |
| `phase` | empty | `cohort` | `cohort` | `20` |
| `review` | empty | `cohort` | — | `30` |
| `deep` | empty | `helper` | — | `40` |
| `foreign` | `OTHER-team` | `root` | — | `50` |

`cmd_tokens T-team` returned `root`, `cohort`, `foreign`, and `helper`: `160` input tokens. The required closure returns the first `6` tasks in the table: `200` input tokens. Thus this fixture demonstrates undercounting and cross-team contamination simultaneously.

Adding a task with both `TEAM=T-team` and `PARENT_TASK=root` produced `2` rows for that task. Setting the caller's `ROUND_ID=ambient-value` while the task's metadata omitted the field made `task_field root ROUND_ID` return `ambient-value`.

### Verified marker compatibility

The probe created `4` Codex runs with cumulative `(input, cached, output)` readings:

- `1 new`: `(1000, 800, 10)`.
- `2 resume`: `(1500, 1200, 15)`.
- `3 fresh`: `(300, 200, 3)`.
- `4 resume`: `(500, 350, 5)`.

It saved `runs_usage`, `run_days`, and `cmd_tokens --runs` output, appended ` round R12/A pr #74` to every start marker, and compared each output with `cmp`. All `3` comparisons were identical; total input remained `2,000` tokens. This verifies suffix compatibility in the existing readers, not the unimplemented sub-round attribution feature.

## Skipped tests

- None added or changed.
- Real agent runs and integration tests were not run; they are unnecessary for this investigation and are prohibited without explicit authorization.

## Not done

- Neither implementation part is shipped by this report.
- No PR was opened, following the Phase 1 delivery instruction.
- Remote usage synchronization, panel round display, round lifecycle management, and retroactive labelling remain outside the requested implementation scope.
- No repository-instruction changes are proposed; the accessor and constructor caveats above are the notebook-equivalent findings for the merger.

[create]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L6792-L6846
[resume]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L7026-L7346
[helper]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L5940-L5983
[investigation]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L5782-L5829
[cohort]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L5703-L5743
[proxy]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L8247-L8281
[admit]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L5624-L5686
[phase]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L5159-L5227
[advisory]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L4863-L4915
[node-create]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L6883-L6936
[run]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L7384-L7595
[failover]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L2901-L2968
[takeover]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L1300-L1321
[advisory-resume]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L4918-L4963
[meta]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L743-L798
[inv-field]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L5117
[tokens]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L8940-L8993
[related]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L1194-L1209
[model]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L8696-L8701
[spend]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L8703-L8752
[usage]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L8421-L8457
[fresh]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L8535-L8577
[card]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L8624-L8687
[days]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L8688-L8694
[runner]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L6171-L6219
[supersede]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/bin/agent#L2461-L2473
[nodes]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/docs/multi-instance.md#L369-L419
[sync]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/agentbus/lead_sync.go#L58-L67
[token-tests]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/tests/unit/tokens.bats
[test-map]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/tests/agent-go-map.tsv
[makefile]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/Makefile
[map-check]: https://github.com/mateusz-szczyrzyca/team-agents/blob/45f1e7a29ebd2797d3112e39e129bfe938236604/tools/check-agent-test-map.sh
