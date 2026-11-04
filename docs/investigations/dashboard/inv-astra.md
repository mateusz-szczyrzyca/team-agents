# Dashboard investigation — inv-astra

Independent Phase 1 result for `20261104-160248-inv-astra`, based on
[`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f).
Implementation: [`53b0b56`](https://github.com/mateusz-szczyrzyca/team-agents/commit/53b0b5601cfd0090d52518fda4190322a5c79791).
Tests first: [`2a80d29`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2a80d29cf26b4b2be6e0dcfdc8842d32594b05a2).

## Results

| Request | Result |
|---|---|
| `1`: compact dashboard | `/` and its SSE feed select at most `20` newest eligible local/remote entries by creation time, then retain the established active-first grouping. The menu links to `/dashboard/full`, with all history and a separate SSE stream. |
| `2`: stop team | `/teams` offers **Stop this team**, including finished teams that can still have review/merge work pending. It invokes `agent team <id> --stop`, records durable stop intent for members and delegated descendants, closes the bus team, stops local runs, cancels remote member jobs and disables autopilot. |
| `3`: investigation priority | Normal-team explicit resume, message wakes and answer resumes are blocked while a nonterminal top-level `--investigation` shares `REPO_ROOT`. The refusal identifies the investigation. Ordinary `request_investigation` work and other projects are exempt. |
| `4`: background work | Inventory, sources, triggers and default periods appear below. There are `8` lifecycle job families, plus `3` optional persistent service families and `1` explicit diagnostic job family; this is a count of kinds, not concurrent instances. |
| `5`: stopped versus sleeping | The local team-member web stop uses persistent `agent stop`; interrupted runner cleanup honors explicit stop intent instead of reporting failure and calling team follow-up. Team pages prefer persisted `stopped` over stale bus state. |
| `6`: Serena | All `4` CLI adapters already configure Serena when enabled. New/resumed/fresh/watchdog prompts now consistently include navigation guidance. The executable exists on this host, but the inspected installation configuration has no `AGENT_SERENA` assignment. No live setting was changed. |
| `7`: historical report | §§7–8 of [the earlier merged investigation][audit] are updated. `11` of `12` original inconsistencies were already fixed; remote-helper protocol differences remain partly relevant. |
| `8`: PR ownership | [One shared rule][ownership] reaches every role and continuation: only the coordinator opens a PR; reuse the subject's existing Draft or Open PR, including one on another branch. Role-specific creation instructions now agree. |

Main implementation sources: [`bin/agent`][script], [`live/web_views.go`][webviews],
[`live/web_jobs.go`][webjobs], [`live/web.go`][webroutes] and [web templates][templates].
User-facing semantics and configuration guidance are documented in [`README.md`][readme] and
[`config/agents.conf`][config].

## Background jobs: what can happen with no model running

Most orchestration executes in Bash around the model process. A runner finishing can start a separate
precheck, wait for capacity, process reviews, perform a fix pass, merge a reviewed PR or prepare a new round.
The synthetic independent-investigation parent is metadata, not another model process. Therefore “no model
running” does not mean “no orchestration running.” [Lifecycle implementation][script].

### Lifecycle job families (`8`)

Definitions below name functions in [`bin/agent`][script] unless another file is linked. Periods are defaults;
configuration can override them. Event-driven jobs do **not** have a fixed periodic schedule.

| Family / unit | Defined by | Trigger and period | Work and observable evidence |
|---|---|---|---|
| Runner: `agent-<task>-r<run>.service` | `start_runner`, `cmd__runner` | Task start, explicit resume, answers, message wake, delegated work or a pipeline phase | Builds prompts and MCP configuration; runs the CLI, normalizer and watchdog; interprets final status, queues follow-up work. Task runner/activity logs and `runner_starting` / `run_started`. |
| Supervisor: `agent-supervise.timer` / `.service` | [Installation units][install], `cmd__supervise` | `OnBootSec=3min`, then `OnUnitActiveSec=3min` | Detects orphaned runners after `120` seconds' grace, reboot recovery, stale answer watches, remote questions/member jobs, abandoned teams and review waits; advances eligible investigation handoffs/queues. It can start work even when no runner finishes. |
| Wake: `agent-<task>-wake-<timestamp>` | `schedule_wake`, `cmd__wake`, bus wake callbacks | One-shot timers or messages/board changes; timer accuracy `10` seconds | Provider reset when known, otherwise quota `30` minutes, rate/overload `5` minutes, network `15` minutes. Wake-loop cooldown `15` minutes after `4` idle runs or more than `30` runs in `10` minutes. |
| Answers: `agent-<task>-answers-<round>.path` and `.service` | `watch_answers`, `cmd__answers_saved` | `PathModified` on the question file; no periodic poll | Validates marked answers, resumes a local task or queues remote delivery. A draft save alone does not launch a model. |
| Remote question readiness: `agent-remote-questions-<task>@<node>.service` | `cmd__remote_questions`, `cmd__remote_questions_ready` | Mirrored question arrival; supervisor retries incomplete setup | Creates the editable question copy, editor/watch setup and notification outside sync handling. This is distinct from sending answers. |
| Precheck: `agent-<task>-precheck-<stamp-or-counter>.service` | `start_precheck`, `cmd__precheck`, `review_precheck`, independent pipeline | A coordinator finishes a verified round, or an independent merger publishes its branch/PR | Waits for the team, checks tests/PR structure, runs the repository gate in a temporary detached worktree, then queues review or returns findings. Gate timeout `30` minutes; progress refresh `2` seconds. At most `2` ordinary precheck returns before human handoff. |
| Fix pass: `agent-<task>-fixpass-<timestamp>.service` | `review_fix_pass`, `cmd__fixpass`, `do_fix_pass` | A review reports critical/major findings or incomplete work | Waits for members to settle, resumes the same round with findings, or asks the human after `2` fix passes. |
| Merge/next round: `agent-<task>-merge-<timestamp>.service` | `start_task_unit`, `cmd__merge` | Autopilot receives two ready-to-merge reviews | Applies merge blockers and checked-head matching, merges the PR, runs the post-merge gate, then resumes the next round. No periodic schedule. Autopilot-off or failures hand control to the human. |

`team_wait_settled` checks every `5` seconds for at most `120` checks (`10` minutes). The precheck/fix-pass/merge
paths use it before handing work back. Precheck progress and post-merge validation explain long intervals
without a model running. Precheck and fix-pass launch failures have synchronous fallback paths, so that work
can also remain inside the finishing runner. [Source][script].

### Persistent services (`3`, enabled according to the installation's role)

| Service | Source | Timing and purpose |
|---|---|---|
| `agentweb.service` | [Installation][install], [web server][webroutes], [views][webviews] | Serves the dashboard; the default live snapshot/SSE interval is `1` second. Observation alone does not start agents; explicit web actions do. Logs can refresh every `5` seconds. |
| `agent-lead.service` | [Installation][install], [lead server][lead], [remote controls][remote] | Handles nodes, remote jobs and control delivery. Absent-node checks every `30` seconds; default heartbeat TTL `90` seconds. Doorbell connections check pending work every `250` milliseconds and send keepalive comments every `20` seconds. |
| `agent-node.service` | [Installation][install], `cmd_node` in [the script][script] | Heartbeat/poll every `NODE_POLL_SEC=5` seconds; the lead's doorbell can wake it earlier. Starts assigned jobs and relays progress, answers, kicks and cancellations. |

There is also `agent-auth-<agent-or-provider>-<pid>`: `cmd_authenticated` / `auth_providers` run `_authcheck`
**only when explicitly requested**. It is a diagnostic family, not a periodic lifecycle job. It can make a paid
provider request; it was not executed for this investigation. [Source][script].

### Work that is not a separate scheduled job

- `dispatch` and `dispatch_helpers` run inline after events or from the supervisor. They consume queues, select
  available investigators/workers and advance independent phases. Their locks wait up to `120` and `60` seconds,
  respectively; these are contention timeouts, not scheduling periods. [Source][script].
- Pending review-pair rotation is revisited by the supervisor. The default busy-pair wait is `20` minutes;
  review-capacity notification starts after `30` minutes. Abandoned open-team notification defaults to `60`
  minutes. [Source and defaults][script].
- Each live runner's watchdog checks activity every `15` seconds. A default `30` minutes without activity is
  a hang; `3` consecutive hangs without progress exhaust recovery. Long team runs are nudged at `120` tool
  steps and cut at `200`. These are per-run mechanisms, not permanent global jobs. [Source][script].
- Blocking bus operations poll only while requested: reservation waits every `3` seconds; message, board and
  consultation waits every `2` seconds. [Reservation source][bus], [message source][busteam],
  [board source][collab], [consultation source][consult].
- Notification hooks run on events. Their configured external command can do additional work; it has no fixed
  cadence imposed by this repository. The terminal panel refreshes observations every `1` second. [Script][script],
  [panel][panel].
- `/jobs` and `agent jobs` list **remote job records**. They are not an inventory of local precheck, fix-pass,
  merge or wake units. [Remote jobs UI][webjobs].

A read-only unit snapshot during this investigation showed `2` active runner services, `1` active lead service,
`1` active web service, a waiting supervisor timer and its inactive service. It showed no active precheck,
fix-pass, merge or wake units at that instant. Counts change as tasks progress; there is no fixed number of
live job instances. No units were stopped or restarted for inspection.

To inspect the current work without changing it:

```bash
systemctl --user list-units --all 'agent-*' 'agentweb*'
systemctl --user list-timers --all 'agent-*'
journalctl --user -u agent-supervise.service --since '-30 min'
agent show <task>
agent log <task>
agent jobs
```

Use the coordinator's dashboard stage and its precheck/postmerge log to distinguish tests, waiting for review
capacity, and model work. The dashboard is not yet a unified local systemd-job inventory; this request asked
for a report, so no new jobs page was added.

## Serena: configured capability versus actual model use

[Upstream Serena](https://github.com/oraios/serena) is an MCP language-server tool suite.
Its [client documentation](https://oraios.github.io/serena/02-usage/030_clients.html) describes client integration;
this repository launches a separate server on **each run's worktree** with the appropriate context:

| CLI adapter | Context | Existing configuration |
|---|---|---|
| Claude Code | `claude-code` | Generated MCP JSON; independent investigators retain Serena under strict MCP isolation. |
| Codex | `codex` | `mcp_servers.serena` overrides; startup timeout `120` seconds, tool timeout `300` seconds. |
| Antigravity/Gemini | `antigravity` | Workspace MCP configuration. |
| OpenCode | `ide` | Enabled local MCP server with command and arguments. |

All use `--project <worktree> --open-web-dashboard false`; the project cache is excluded from Git.
[Adapter code][script] and [adapter regression tests][serenatests] establish the wiring. [The prompt][serena]
says to use symbol tools first for navigation and fall back to shell searches when unavailable, slow or unsupported.
This change also repeats that guidance on continuations, where an older session may not have received it.

The read-only host check found a Serena executable at `$HOME/.local/bin/serena`, but no `AGENT_SERENA` assignment
in the installation configuration. Installing Serena alone does not enable these adapters. The operator should
set `AGENT_SERENA` to its absolute executable path for future runs using [the documented configuration][config].
Repository changes do not update installed templates or an already-running model's MCP configuration.

The tests prove that clients receive the server configuration and models receive instructions to use it.
They **cannot prove actual model tool selection or successful language-server startup**. No real model calls,
package installs or live configuration changes were made. This is an operational follow-up for the merger/human,
not a claim that Serena is currently enabled for every process on this host.

## Compatibility and limits

- Existing bus JSON, task metadata, MCP schemas and exit-code contracts are unchanged. The patch reuses
  existing `stop-requested`, parent links and `AUTOPILOT_OFF`; old files remain readable.
- New interfaces are additive: `/dashboard/full`, `/events/dashboard/full`, `POST /team/{id}/stop`,
  `agent team <id> --stop`. Existing `--close` and `agent kick` semantics remain compatible.
- The local team-member web stop deliberately changes from resumable kick to persistent stop. Remote task
  controls retain the established run-scoped kick protocol; **whole-team stop** cancels remote member jobs.
- A newly stopped team cannot automatically wake or schedule later phases. A check already executing may
  finish cleanup; stop does not undo an already completed GitHub merge. Explicit `run --resume` creates a
  new cohort after investigation priority clears, preserving the old stop markers.
- Investigation priority applies to normal-team resumption. It does not forcibly stop normal teams already
  running, block new normal-team creation, or add a cross-host project lock. Blocked work requires explicit
  resumption after the investigation ends; no new automatic retry queue is introduced.
- Compact selection is by creation time, with ID as tie-breaker; it preserves prior acknowledgement eligibility
  and hides duplicate proxies/synthetic parents. The full page removes the history cutoff. Neither page is a
  paginated database query: snapshots still read all retained metadata.
- The incident's exact `20261104-122828-haiku` task directory was unavailable during inspection. The code path
  explains how a web kick intentionally produced `sleeping`; the patch prevents that for future local stops.
  Historical sleeping records without a persistent stop marker cannot safely be reclassified automatically.
- New structured script events: `team_stopped`, `automation_stopped`, `resume_blocked_investigation`.
- The new universal PR rule is prompt guidance. The existing `gh` wrapper already refuses ordinary
  team-member PR creation; this change does not add a GitHub access policy. Existing automatic duplicate-PR
  cleanup behavior was left unchanged to preserve its tested contract; this patch prevents models creating
  duplicates rather than replacing that cleanup mechanism.

## Checks

- $\color{red}{\textbf{RED}}$: [test-first commit](https://github.com/mateusz-szczyrzyca/team-agents/commit/2a80d29cf26b4b2be6e0dcfdc8842d32594b05a2)
  records the failing output: `4` new Go tests and `7` new Bats tests failed; the exemption control already passed.
- $\color{green}{\textbf{GREEN}}$: `env -u AGENT_GITHUB_NAME_COLORS make gate` — all `7` steps passed,
  `0` lint findings, `560/560` Bats tests and `20/20` harness tests. Go tests/vet passed for all `3`
  application modules; all module lint checks passed.
- `16/16` focused Bats tests passed: `6` lifecycle, `2` prompt and `8` existing Serena adapter tests.
- The complete existing live Go suite passed, including the `4` new dashboard tests (HTML, SSE, stop dispatch,
  team-stop validation and persisted stopped state).
- Existing investigation admission cancellation regression passed after preserving cancelled children as
  `stopped` rather than launch failures. No existing test, helper or stub was changed.
- One additional full run failed the existing “a leased parent without manifest keeps the lease while its tagged child is still running” [admission test][admissiontests] at `lease_now = 1`.
  The isolated test and the next full gate passed on this branch; `8/8` isolated runs on the immutable base also passed.
  The failure is intermittent, but its root cause is **not established**; no test, stub, parallelism setting or
  timeout was changed to hide it. The merger should review this observation before integration.
- The inherited runner sets `AGENT_GITHUB_NAME_COLORS`, which made an existing byline fixture observe a different
  value. Tests ran with only that inherited setting removed, retaining their own isolated home/configuration.
  This was reported rather than changing the fixture or its expectation.
- Round size: `954` lines (`+850` / `−104`) in `21` files against the immutable base, including tests and reports; below the `3,000`-line limit.
- No lint suppressions, new dependencies, real agent CLI calls or integration suite runs. The unversioned
  `python` command was absent; documentation tooling used the available `python3`.

## Skipped tests

None added. Existing placeholders are untouched.

## Not done

- No PR opened: Phase 2 owns integration.
- No running installation, service, model session or historical task state was changed.
- Serena still needs operator configuration and a later live-use check if actual model behavior must be proven.
- Remote-helper protocol differences and the local helper's Polish context heading remain findings for a separate change; see the
  updated [§8 audit][audit].
- No repository notebook entry was possible because this independent phase has no agentbus tools. This report
  records the findings instead.

[script]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/bin/agent
[webviews]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/live/web_views.go
[webjobs]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/live/web_jobs.go
[webroutes]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/live/web.go
[templates]: https://github.com/mateusz-szczyrzyca/team-agents/tree/53b0b5601cfd0090d52518fda4190322a5c79791/live/web/templates
[readme]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/README.md
[config]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/config/agents.conf
[install]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/install.sh
[lead]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/agentbus/lead_server.go
[remote]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/agentbus/remote_member.go
[bus]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/agentbus/main.go
[busteam]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/agentbus/team.go
[collab]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/agentbus/collab.go
[consult]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/agentbus/consult.go
[panel]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/live/ui.go
[serenatests]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/tests/unit/serena.bats
[serena]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/prompts/serena.md
[admissiontests]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/tests/unit/investigation_team_admission.bats
[ownership]: https://github.com/mateusz-szczyrzyca/team-agents/blob/53b0b5601cfd0090d52518fda4190322a5c79791/prompts/pr-ownership.md
[audit]: https://github.com/mateusz-szczyrzyca/team-agents/blob/35e13eb41a30f7f841e637eba180cc5b941bba70/docs/investigations/i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md#7-summary-matrix--audited-2026-11-04
