# Rust rewrite task-list investigation — `inv-astra`

## Recommendation

Keep the proposed Rust architecture and the deliberate break from the Bash/Go harness, but revise the task list before assigning implementation branches. The main blockers are contracts and dependency order, not the choice of language.

- Supply the missing developer/reviewer rules before task `1.1`.
- Specify how the unchanged `agentbus` calls back into the new daemon and how its types map to the new model.
- Correct the dashboard dependency and define how `--no-merge` advances branches and dependencies.
- Bind gates, reviews, and merges to the same immutable revision, including recovery after uncertain external writes.
- Split runtime, adapter, daemon, and autopilot work into independently verifiable PRs.
- Make the cutover/removal exceptions explicit; the Bash file alone exceeds the proposed deletion budget.

This is an independent planning investigation, not an implementation of all `16` submitted items. It does not import the old Go rewrite's parity requirements. The proposed task breakdown below preserves the requested scope and identifies decisions the implementation brief needs to settle.

## Scope and evidence

- Base: [`27b39ba`](https://github.com/mateusz-szczyrzyca/team-agents/commit/27b39ba293e36633a7c07ec562d0d3c9b8626725).
- Branch: `agents/inv-astra/20261111-141417`.
- Only this worktree was inspected; no other investigator's work or findings were read.
- Serena was not exposed in the session's tool catalog. Targeted `rg` searches and source reads were used instead.
- Repository instructions were read from [`AGENTS.md`][agents]; no nested instruction file was found in the repository file inventory.
- The supplied task list was assessed as the proposed specification. No Rust workspace or checked-in Rust task-list fixture was found at this base.
- The paths `prompts/agent/rust-developer.md` and `prompts/investigator/code-review.md` do not exist at this base. No alternative file with their names or the referenced rule IDs was found in the searched repository instructions, prompts, or documentation. These missing paths cannot be linked to existing source.
- External API details below were checked against official documentation on `2026-11-11`. No GitHub comments or reviews were fetched, and no account or model CLI was invoked for research.

Existing source links in this report are pinned to the immutable base. Proposed paths and absent files are identified as such rather than linked to nonexistent source.

## Findings that change the implementation plan

### 1. The rule pack is a prerequisite, not a task `2.3` deliverable

Task `1.1` needs the Rust rules' lint appendix and the interpretation of `PR-1`/`PR-2`; the brief makes the review rules binding on every PR. Task `2.3` currently adds these documents later. The existing [`prompts/review.md`][review] is a different protocol and cannot establish the absent `ERR-2`, `DEP-1`, or `PR-6` requirements.

**Amendment:** put the human-supplied rule documents and canonical task-list source in a preparation step. Do not fabricate their contents. Task `1.1` can then link to them, record the authorized `5,000`-line ceiling and approximately `4,000`-line split trigger, and integrate the actual lint baseline. A versioned stable Rust toolchain and lockfile should make the workspace reproducible; select the current stable version when that task is implemented.

The task explicitly anticipates changes to repository instructions, CI, and eventual deletion of old tests. Record those narrow exceptions in the implementation brief so they are distinguishable from unrelated changes forbidden by [`AGENTS.md`][agents]. This investigation makes none of those edits.

### 2. Unchanged `agentbus` requires an explicit harness boundary

The bus is not just a passive subprocess returning JSON. It owns mutable state, exposes MCP operations, and launches callbacks:

| Existing behavior | Evidence | Required decision |
|---|---|---|
| A sleeping member receives a message; the bus sets it to `waking` and invokes `<wake_cmd> _wake <task>`. | `spawnWake` / `wakeFor`, [`agentbus/team.go`][team] | Supply a callback that forwards to the daemon, or define a complete alternative polling/reconciliation path. Leaving the command empty only logs a warning. |
| A delegation is persisted, then `<agent-cmd> _dispatch-helpers` is spawned. | `delegateWith`, [`agentbus/collab.go`][collab] | Define the dispatch trigger and recover queued work if notification fails. |
| Local `board_done` calls `<agent-cmd> _board_done_check <task>` when a task directory is supplied. Unknown callback results currently warn and allow completion. | `gateCheck`, [`agentbus/collab.go`][collab] | Define the callback's exit/output contract; do not mistake an unavailable check for verified work. |
| Detached callbacks may be launched via `setsid`. | `spawnDetached`, [`agentbus/collab.go`][collab] | Scope the process allowlist to the new harness, with an explicit unchanged-bus exception, or change the bus requirement. |
| MCP initialization tells ordinary agents that `request_investigation` is available. | `instructions`, [`agentbus/main.go`][bus-main] | Define which existing MCP tools are supported for new runs. An out-of-scope tool must not leave invisible queued work. |

**Recommended resolution:** permit a small internal compatibility surface in the Rust binary for the callbacks required by the unchanged bus. These commands only submit typed requests to the daemon; they neither call the Bash harness nor read its metadata. This is an explicit exception to “no shared commands,” not a reason to retain the old public CLI or state format. Use periodic reconciliation as a fallback for missed callbacks.

The alternative is to revise “`agentbus` stays as it is” and implement a native event/control interface there. That is additional scope, so it should not be silently assumed by task `4.2`.

### 3. Define bus ownership, mappings, and capacity before the state model

The daemon can be the sole writer of **harness state** and sole launcher of model runs. It cannot be the sole writer of all state while model-owned `agentbus serve` processes mutate their bus state. Document two ownership domains and the direction of reconciliation.

- The bus accepts tiers `normal` and `worker`, not `investigator`; its role values are historical `worker` and `investigator`, not the five proposed harness roles. See [`agentbus/types.go`][bus-types]. Map these types explicitly at the adapter boundary. Sending a harness investigator tier directly would be rejected.
- Bus presence and read cursors are keyed by agent name. See `State` in [`agentbus/main.go`][bus-main]. Define one active run per configured agent identity per bus/home, or introduce a documented identity mapping. Team admission and reviewer/worker reservations need the same capacity authority.
- Task-list IDs such as `2.3` differ from bus board IDs such as `B1`. Persist the mapping; repeated recovery must not create a second board item for the same task-list item.
- The bus board permits claims and leadership changes independently of daemon scheduling. Decide whether the task list is authoritative and board results are proposals. A model claim must not unlock a dependency or launch a duplicate run.
- `team-create` replaces a team entry with the supplied object. It is not a harmless retry after members have progressed. Reconcile existing teams before retrying creation; see [`agentbus/team.go`][team].
- `helper-next` claims a request by changing its state, and `helper-set` updates it without publishing the final requester message. The wrapper needs recovery and explicit result delivery, not merely exit-code conversion. See [`agentbus/collab.go`][collab].
- The available release CLI is `board-release-agent`, which releases that agent's items; there is no equivalent single-item `board-release` CLI in the dispatcher. Specify the intended operation in the runtime contract. See [`agentbus/main.go`][bus-main].

All bus calls must set the isolated directory/home explicitly. The new daemon should not deserialize and rewrite the bus's state as if it owned that format.

### 4. Resolve dependency and responsibility mismatches

| Mismatch in the submitted list | Smallest correction |
|---|---|
| Feature `9` depends only on `6`, but promises answers, stop, pause, unpause, and resume implemented in `7.2`. | Make feature `9` depend on `7`, or separate read-only and control dashboards into different features. Merely declaring socket request types in `2.2` does not implement those actions. |
| `1.1` promises task/item state machines before `2.2` defines their persistent model and `3.1` defines their events. | Put vocabulary/validated IDs in feature `1`, serialization in `2`, and transition policy in `3`; keep status validation distinct from transition validation. |
| The coordinator opens the PR in the brief, but the daemon opens it in `8.1`. | Let the coordinator integrate and supply the PR text; let the daemon own push/publication/reviews/merge and persisted outcomes. |
| A solo run has no separately selected coordinator, while assignment is phrased only for normal members. | Specify that the solo agent performs both implementation and integration. |
| Investigators may be team members/coordinators, but assignment is phrased as “normal member.” | Define whether this means the `normal` tier or every non-worker implementer. Recommended: any admitted non-worker, with a serialized coordinator integration lane. |
| Both a file's coordinator and the CLI option “win.” | Define a total order: CLI override, then file value, then automatic selection. Validate that the selected name belongs to the requested team and is eligible. |

Add cases for an investigator-only team, a single investigator solo run, duplicate names, an all-worker request, no eligible implementer, and a busy requested agent. Validate the two-outside-reviewers rule for solo autopilot as well as multi-member teams if both get mandatory reviews.

### 5. `--no-merge` needs branch and completion semantics

The brief allows the next item in a feature to start after two approvals under `--no-merge`; feature dependencies still require merged features. This can be coherent, but a scheduler needs distinct `approved`, `handed_to_human`, and `merged` facts.

Example: item `2.1` is approved but unmerged. Item `2.2` starts. Starting its branch from the repository base would omit the approved implementation it needs. The plan does not specify a stacked PR, a retained integration branch, or a wait for human merge.

**Recommended amendment preserving the stated behavior:** allow same-feature successors to build on the approved predecessor revision, record their PR base relationships, and reconcile those bases after a human merge. Dependent features stay blocked until all required feature items are merged. This adds real GitHub/branch work to feature `8`.

The simpler alternative is to wait for human merge before any successor starts, but that changes the requested behavior and needs an explicit plan decision. Also define whether a team with every PR handed over is closed or waiting for external merges, and how a closed-unmerged PR is recovered. `[x]` imports need a clear meaning: a trusted completed prerequisite, not fabricated review/merge evidence.

### 6. REST-only does not cover the requested ready transition

GitHub's documented REST update operation does not accept a draft-state field; GraphQL provides `markPullRequestReadyForReview`. See the official [REST update contract](https://docs.github.com/en/rest/pulls/pulls#update-a-pull-request) and [GraphQL mutation](https://docs.github.com/en/graphql/reference/pulls#markpullrequestreadyforreview).

**Recommended amendment:** allow the narrowly typed GraphQL operation through the same `reqwest` client if draft-to-ready remains required. Alternatively, create an already-ready PR after the gate and remove the separate ready transition. Neither requires adding `gh` as a subprocess.

Specify pagination for trusted comments/reviews, repository identity validation, authentication failures, rate limits, response size caps, and uncertain writes. A timed-out create/review/merge request may already have succeeded; persist the intent and reconcile before retrying it. “Retries for idempotent calls only” does not itself solve recovery after a lost response.

### 7. Freeze the revision being gated, reviewed, and merged

“Gate green at the head” is insufficient if that head or worktree changes while the gate runs or while reviews wait.

- Record item, run, repository, branch, commit, attempt, and worktree cleanliness with a gate result.
- A dirty tree or a merge in progress may be checked for development, as requested, but its pass must not authorize publishing or merging a different committed tree.
- Prevent concurrent writes during an authoritative gate, or gate an immutable checkout. A clean tree before and after is not proof of what intermediate files were tested.
- Bind both reviews to the same PR head and attempt. Ignore late results from canceled/replaced reviewer runs.
- Any subsequent push invalidates prior gate/review authority; a fix pass uses the same PR but new revision evidence.
- Recheck remote PR identity/head before merge and send the expected head SHA. The official [merge endpoint](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request) provides this guard.
- Distinguish accepted/enqueued merge requests from a confirmed merge if an asynchronous merge route is used.

Reserve a separate coordinator integration worktree/lane. Parallel features do not justify two concurrent merges into that worktree. Work assignment needs to refresh from the actual merged base, including when another team or a human advanced it.

The old repository already tests dirty/in-progress gates and exemptions for a run's own gate in [`tests/unit/gate_own_run.bats`][gate-tests]. Use those as risk evidence, not as mandatory compatibility tests for the new design.

### 8. Recovery needs persisted effects, not only session IDs

Atomic rename prevents readers seeing a partial state file. It does not make a state update atomic with a spawn, git merge, bus mutation, notification, or GitHub request.

**Add to features `2`, `4`, and `6`:** persisted operation identities and intent/result records for externally visible actions, with a documented recovery rule per action. A small outbox or explicit pending-operation table is enough; an event-sourcing framework is unnecessary.

Required cases include daemon death after spawn but before recording the PID/session, delegation claimed before launch, PR created before saving its number, merge accepted before saving completion, and an answer saved before its resume is acknowledged. Repeating a client request after socket disconnection must not create another team or answer the next question round.

Process groups cover ordinary cancellation while the daemon is alive. Specify what happens on abrupt daemon death: use systemd's service process containment for its children, or another tested way to prove the old model tree is gone before resuming. A stored PID alone is not durable ownership because it can be reused. Child groups that deliberately detach require an explicit limitation or stronger containment.

Recovery must distinguish manually stopped/paused tasks, pending questions, timed pauses, active runs, completed-but-unpublished runs, and missing/unusable sessions. Do not resume everything that is not terminal. Persist UTC wake deadlines; use a monotonic clock for elapsed watchdog time while running.

### 9. Define gate scope, nesting, and cancellation

A FIFO inside one daemon is one queue **per home**, not necessarily one gate per machine. The isolation requirement permits multiple homes, and Bash/Rust coexist before cutover. The existing [`tools/test-lock.sh`][test-lock] uses a machine-wide lock and can route `make gate` through the installed harness.

**Amendment:** specify whether “machine” means one user on one host or all users, name the shared native lock, and decide how the new queue participates alongside the existing gate. Do not replace the lock while another implementation still uses it.

Prevent recursion when the configured gate command itself routes through `agent gate`. Define the nesting marker and which process owns the lock. Exempt only the requesting run while its queued/running gate is live; a stuck/canceled gate must not disable the watchdog indefinitely. Include timeout, stop, daemon restart, and a disconnected requesting client.

### 10. The parser, templates, and questions need exact grammars

- Check in a canonical, human-approved copy of the task list as a parser fixture before feature `2` uses “this file.” Preserve its introductory prose and target-layout block.
- Distinguish feature headers from numbered prose; specify indentation, code fences, blank lines, CRLF, tabs, trailing text, `[x]`, duplicate/mismatched IDs, forward dependencies, `none`, missing references, and cycles.
- Define whether plain inline text with no feature/item syntax creates a synthetic item. The examples promise `agent run ... "tasks"`; treating that as only a brief leaves nothing to schedule.
- Resolve the positional ambiguity between multiple agent names and inline task text. Test the actual `clap` grammar, including one-word tasks that equal configured agent names, `-f`, and `--`.
- Render templates once. Inserted task text, code, answers, and checkpoints containing `{{name}}` must remain data, not recursively rendered templates.
- Define malformed-placeholder behavior and literal escaping. Unknown supplied variables and unknown placeholders are distinct errors.
- Validate each installed prompt pack's actual catalog; future feature templates must not make an earlier feature's startup fail.
- Treat the human-editable questions file as an input artifact, not as a second writer of daemon state. Watch the directory so atomic editor replacement works; reconcile missed/duplicate notifications.
- Key an answer to task, question round, and content revision. Reject stale/conflicting answers from editor, CLI, or web. Validate all verdicts before accepting the final `**ANSWERED**` marker; define exact `ok` recommendation expansion and multiline answers.

The old [`live/web_questions.go`][questions] parser and [`bin/agent`][bash-agent] `questions_note` show the existing format, but do not settle the new strict grammar. The existing [`prompts/clarification.md`][clarification] is used by the live Bash prompt builder; avoid silently replacing its protocol during coexistence.

### 11. Review capacity and verdicts require a durable scheduler

Reserve the pair of external investigators atomically; two teams must not each reserve one reviewer and wait forever for a second. Release reservations on pause, quota, failure, timeout, and cancellation according to a specified policy. Keep the configured two-outside-reviewers admission rule separate from dynamic capacity: busy reviewers cause a wait, while an agent already running elsewhere cannot be launched again under the same bus identity.

Define a strict verdict grammar from the missing review rule pack. Absent, conflicting, or truncated verdicts are not approvals. Count fix passes at a defined transition: the initial pair is not a fix pass, and the default `5` means at most five completed attempts to fix findings before asking the human. Keep that counter across restart and duplicate callbacks.

The existing [`prompts/review.md`][review] explicitly uses GitHub `COMMENT` reviews because the PR and reviews share an account. Preserve the distinction between an internal investigator approval and a GitHub approval event; the new review protocol must not assume independent GitHub identities just because it has two model runs. Reviewers should return structured findings to the daemon, which publishes them once under its own durable action identity.

### 12. Specify process, secret, and resource bounds as acceptance criteria

The pure adapter contract is appropriate, but “everything bounded” needs named limits for output records, stderr tails, sockets, queues, file sizes, subscriptions, retries, and subprocess duration. Unknown output events should not crash the run; an oversized/incomplete record should have a bounded, typed handling path. A poisoned activity stream must not keep an otherwise hung run alive indefinitely.

Separate config parsing from filesystem validation: pure core code can validate values, while runtime code checks ownership, file type, permissions, and safe paths. A secrets type should redact `Debug` and never be included in state snapshots, web responses, or raw argv logs. Specify per-process environment allowlists rather than passing all daemon secrets to models, git, notification commands, and editors.

The subprocess policy also needs an explicit boundary: git can invoke SSH/signing/hooks, model CLIs can invoke tools, and configured gate/editor/notification programs have their own children. Serena is requested in feature `5` but omitted from both the process allowlist and the detailed configuration list. Add a typed Serena executable/argv configuration and state which layer starts it. The restrictions can sensibly govern harness-created processes; they cannot honestly describe every descendant unchanged third-party tools may launch.

The four adapters should have separate PRs with sanitized, version-labeled fixtures already available or supplied by the human. Do not run paid CLIs merely to manufacture fixtures. Include UTF-8 split across reads, CRLF, malformed JSON, missing final message, unexpected exit status, quota/rate-limit tails, large prompts sent without exceeding argv limits, and resumed-session failures.

### 13. The web task needs a defined identity and browser boundary

Tailscale Serve supplies identity headers and strips client-supplied versions; its docs recommend loopback binding and explicitly note the remaining trust in local services. See [Tailscale Serve identity headers](https://tailscale.com/docs/features/tailscale-serve#identity-headers).

The plan additionally requires LocalAPI validation. Specify which forwarded client address is checked, how it is authenticated as coming through the trusted proxy, and how its returned login must match the allowlist/header. Checking only the loopback TCP peer identifies the proxy, not the human. A LocalAPI lookup does not itself authenticate an attacker-supplied forwarding header.

Define direct-local access policy, fail-closed behavior on LocalAPI errors, tagged-device behavior under a login-only policy, Host/Origin validation for mutations, CSRF protection, and HTML escaping of model output. Add bounded SSE queues with a snapshot/reconnect path for slow readers; a browser must never block the daemon's owner loop. Specify how the dashboard propagates the authenticated actor through the socket protocol.

Use new owned static assets, with a pinned/vendored htmx asset and documented update path. Do not make the new dashboard depend on assets that feature `10` deletes from the old dashboard.

### 14. Cutover is a deployment contract and a mechanical-removal exception

Task `10.2` cannot fit a general `5,000`-line changed-line ceiling if deletions count: [`bin/agent`][bash-agent] alone is `11,641` lines at the base (`wc -l bin/agent`). The absent `PR-6` may specify an exemption; until supplied, no such exemption can be assumed. Record the exact mechanical-deletion exception or an approved split before coding it.

Deletion also affects support files, not only three source directories. The [`Makefile`][makefile] names the old Go module in lint, test, generation, and parity tasks; [`tools/gate.sh`][gate-script] has its own explicit step list; the [CI workflow][ci] calls separate Make targets. Removing a module without updating all of these breaks the final gate. Inventory harness tests, parity ledgers, staging scripts, install references, old prompts/assets, and documentation links before removal. Keep the error generator and documentation checker if the retained bus still needs them.

**Add a cutover rehearsal before deletion:** stage installation in a temporary prefix/home, verify all prompt/static/config assets and units, execute a stub-model lifecycle, restart the daemon, and verify rollback instructions. Treat installing unit files separately from enabling/restarting live services. No investigator or implementation run should replace the binaries/services hosting itself.

No old-state migration is requested. State the new format/home and explicit old-state refusal in installation documentation, retain old data untouched, and tell the operator how to finish or stop old runs before switching. No legacy test should be removed before equivalent new acceptance evidence exists for the behaviors actually retained by this plan.

## Proposed replacement task breakdown

These are proposed task IDs, not implemented features. They retain the original feature dependency model: all items within a feature are sequential; a later feature uses only merged prerequisites. Each row is one PR candidate. Estimate its actual changed lines before coding; split further before the approximately `4,000`-line trigger. The smaller rows do not waive full-gate verification or test-first work.

**Preparation:** obtain the rule pack; approve the bus callback/process-policy exceptions, no-merge branch semantics, and mechanical-removal accounting; store the canonical task-list fixture. This is prerequisite specification work, not guessed implementation.

| Feature / dependencies | Item | Deliverable and acceptance boundary |
|---|---|---|
| `1` / none | `1.1` | Four-crate workspace, version commands, pinned toolchain/lockfile, lint baseline, Rust gate wired into all relevant gate/CI entry points, architecture and instruction links. Empty crates remain honestly inactive. |
| | `1.2` | Validated identifiers, vocabulary, and injected ID generation. Negative serde/parse cases; collision behavior and numeric item ordering specified. No scheduler yet. |
| `2` / `1` | `2.1` | Typed configuration and examples, including process vectors, Serena, secret redaction, reviewers, limits, home/path resolution, and invalid-name cases. Runtime permission checks remain an explicit later boundary. |
| | `2.2` | Task-list grammar/parser, canonical fixture, dependency validation, inline-text behavior, and source-line diagnostics. |
| | `2.3` | Versioned harness state, operation identities, bus mappings, control requests/responses, snapshot/event sequencing, and answer-revision types. Round trips and malformed/unknown versions. |
| | `2.4` | Questions grammar, prompt catalog/strict one-pass renderer, human-provided role rules, language fallback, and pure adapter interface. All entries introduced in this item render in tests. |
| `3` / `2` | `3.1` | Team formation, coordinator precedence, global capacity inputs, pair reservations, and worker eligibility as pure decisions. Solo and investigator-only cases are explicit. |
| | `3.2` | Item/feature scheduling and round transitions, no-merge branch dependencies, exact-revision evidence, cancellation and retry policy. Model tests cover duplicate/late events and fix-pass limits. |
| | `3.3` | Run outcome classifier and watchdog/quota/rate-limit decisions using sanitized fixture data and injected time. Missing status is specified, not accidental success. |
| `4` / `2` | `4.1` | Private isolated state store, schema/version checks, durable pending effects, append logs, startup prompt loading, filesystem/secret validation, fault-injection tests. |
| | `4.2` | Bounded process supervisor, whole-group cancellation/reaping, environment policy, notification and editor execution. Controlled child fixtures; no real model calls. |
| | `4.3` | Typed git operations, fetch/base refresh, worktree/branch ownership, isolated identity/signing settings, conflict/dirty-tree results, and real temporary-repository tests. |
| | `4.4` | GitHub transport with the approved ready-transition choice, pagination, revision guards, classified failures, capped retries, and reconciliation primitives. Local fake server only. |
| | `4.5` | Real bus subprocess wrapper plus documented callback DTOs/type mapping and recovery probes. Tests use a newly built unchanged bus and temporary homes; no Bash metadata dependency. |
| `5` / `2` | `5.1` | Claude adapter: request construction, streaming parser, MCP files, resume, turn limits, fixture tests. |
| | `5.2` | Codex adapter with the same contract, flags/config/event/session fixtures. |
| | `5.3` | Antigravity adapter with timeout/effort/MCP/session fixtures. |
| | `5.4` | OpenCode adapter with provider/model, bash-timeout, MCP/session fixtures; cross-adapter contract tests. |
| `6` / `3, 4, 5` | `6.1` | Daemon single-owner event loop, home lock, private bounded Unix protocol, idempotent requests, status client, subscriptions, systemd unit, joined shutdown. Unavailable commands return a typed unsupported-stage result. |
| | `6.2` | Run engine: render/materialize/start/stream/classify/apply. Fake CLI end-to-end execution and precise watchdog/gate-wait event interface. |
| | `6.3` | Restart reconciliation, stale-process containment, missing sessions, timed pauses and resume, callback-to-daemon routing. Crash-boundary tests before enabling production scheduling. |
| `7` / `6` | `7.1` | Gate queue and client, machine/user lock scope, recursion policy, immutable authoritative result, timeout/cancellation/recovery, own-run watchdog exemption. |
| | `7.2` | Public `run` grammar, team/bus/worktree setup, item dispatch, serialized coordinator integration, worker dispatch/results, and lifecycle reconciliation. Completion cannot pretend a PR has merged before feature `8`. |
| | `7.3` | Questions, editor/file/web answer ingestion contract, stale-answer rejection, stop/pause/unpause/resume/say, actor audit and notifications. |
| `8` / `7` | `8.1` | Durable push/PR publication/update/ready workflow from a gated revision; templates and recovery from uncertain writes. |
| | `8.2` | Atomic reviewer-pair scheduling, read-only revision-pinned review runs, strict verdict ingestion and one-time publication. |
| | `8.3` | Bounded fix passes, guarded merge and external-merge reconciliation, no-merge stacked progression, final handover/completion notification. |
| `9` / `7` | `9.1` | Authenticated read-only dashboard, LocalAPI/proxy contract, escaped rendering, owned htmx assets, bounded SSE, service unit. Optional PR data can render when feature `8` supplies it. |
| | `9.2` | Revision-checked answer/control forms, CSRF/Origin checks, authenticated actor propagation, failure/reconnect tests. |
| `10` / `8, 9` | `10.1` | Staged installer, asset packaging, explicit new configuration/home, operator cutover/rollback documentation, new README. No live service replacement by the running development harness. |
| | `10.2` | Offline complete-lifecycle acceptance and staged restart/rollback rehearsal, including unchanged real bus. Record evidence required before removal. |
| | `10.3` | Exact mechanical-removal manifest and approved size exception; remove legacy source/tests/support targets only after the new gate stands alone. Retained bus and its tooling remain green. |

This produces `32` implementation items, plus preparation. The increase isolates independently failing contracts; it is not a forecast of final line counts. Where a row still approaches the split threshold, split along its stated acceptance boundary before implementation.

## Acceptance scenarios to attach to the relevant PRs

| Area | Required concrete scenario |
|---|---|
| Isolation | A temporary home supplies every bus path and callback environment; no default path reaches the running installation. |
| Team formation | Investigator coordinator plus normal member, investigator-only team, solo, duplicate name, worker coordinator, fewer than two external reviewers, and same-name concurrent admission. |
| Scheduling | Two independent features run concurrently; next same-feature item waits; missing/cyclic dependencies fail at parse/validation time. |
| No-merge | Successor includes approved predecessor code; downstream feature remains blocked until actual merges; external merge/close is reconciled after restart. |
| Bus boundary | Real `agentbus` wakes a sleeping member and queues a delegate through the Rust callback path; duplicate/missed callbacks never start duplicate runs. |
| Process ownership | Cancel a fixture with children and full stdout/stderr pipes; restart after abrupt daemon loss without overlapping the resumed model. |
| State durability | Fail before/after rename and before/after each external action; recover without duplicate assignments, PRs, reviews, or answers. |
| Gate evidence | Dirty merge-in-progress gate can run but cannot authorize a later commit; head changes during gate/review invalidate the evidence. |
| Gate queue | Competing homes, FIFO cancellation, nested gate command, client disconnect, and restart preserve one active gate and a bounded watchdog exemption. |
| Reviews | Two teams compete for reviewers; pair reservation does not deadlock; stale verdict, malformed verdict, quota, and canceled attempt cannot approve a new head. |
| Fix limit | Initial review plus exactly the configured fix passes; the next change request asks the human once, including across restart. |
| Questions | Atomic file replacement, duplicate watcher events, simultaneous web/editor answers, stale question round, partial verdicts, multiline recommendation, and marker in a code block. |
| GitHub | Create/merge succeeds but response is lost; reconciliation prevents duplicate publication or premature completion; untrusted paginated content stays data. |
| Prompt/adapter | Inserted `{{name}}` remains literal; unknown variables fail; split UTF-8/JSON, huge records, unknown events, failed resume, and no final message are bounded. |
| Web | Disallowed login, forged forwarding data, missing LocalAPI, wrong Origin, model-supplied HTML, and slow SSE subscriber fail safely without blocking state changes. |
| Cutover | Temporary-prefix installation contains every required asset, new-home refusal of old formats is clear, rollback is rehearsed, and retained bus tests still pass. |

Tests belong to the item implementing each behavior: add them first and demonstrate the expected failure, then implement. Do not add skipped placeholders to early foundation PRs. Existing tests remain untouched until the explicitly authorized mechanical-removal item.

## Checks

- Targeted source inspection and official API-document checks completed.
- `wc -l bin/agent`: `11,641` lines at the base.
- `GATE_WAIT=1 make gate`: all `8/8` steps passed (`check-generated`, `lint-shell`, `lint-go`, `lint-docs`, `unit-go`, `unit-bats`, `harness`, `unit-agent-bats`); `0` lint findings.
- `git diff --check`: passed.
- No production changes, dependencies, or tests added or modified.
- `Test-First: not possible — documentation-only investigation; no executable behavior changed.`

## Skipped tests

- None added or changed. The repository gate is required before committing this report.

## Not done

- No Rust implementation, installation, migration, or live service changes.
- No real model CLI, paid integration test, hardware use, or PR creation.
- No invented replacements for the missing binding rule documents.
- No guarantee that all original items fit the line ceiling; the proposed splits require implementation-time estimates.
- No notebook entry: this independent phase has no agentbus tools; this report records the findings instead.

[agents]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/AGENTS.md
[review]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/prompts/review.md
[team]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/agentbus/team.go
[collab]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/agentbus/collab.go
[bus-main]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/agentbus/main.go
[bus-types]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/agentbus/types.go
[gate-tests]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/tests/unit/gate_own_run.bats
[test-lock]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/tools/test-lock.sh
[questions]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/live/web_questions.go
[bash-agent]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent
[clarification]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/prompts/clarification.md
[makefile]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/Makefile
[gate-script]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/tools/gate.sh
[ci]: https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/.github/workflows/test.yml
