# Which instructions reach each model in team-agents?

Investigator: `inv-astra` · Date: 2026-11-03 · Audited commit: [`6cb487f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6cb487f0caf967b8085c326e6298a9306559741c).

This report maps **the instructions actually assembled by this repository** to agent roles, model tiers, CLI types, and run modes. All source links are pinned to the audited commit, so their line numbers remain meaningful after later changes. This is an investigation report; it changes no runtime behavior.

## 1. Direct answer

- **Normal and worker models normally receive the same base prompt.** Solo tasks use `protocol.md`; ordinary team tasks use `team.md`. Conditional sections inside those files distinguish normal models, workers, coordinators, and members.
- **A local delegated worker receives `helper.md`**, which has narrower permissions than the ordinary worker task prompt.
- **“Investigator” has several different prompt paths.** An investigation requested by another agent uses `investigator.md`; `--investigation` Phase 1 uses `inv-work.md`; its merger uses `inv-merge.md`; PR reviewers use `review.md`.
- **`--autopilot` does not replace the initial team prompt.** Its special decision-making prompt is delivered when questions are bounced back to the agent. Its review/merge machinery is separate from that prompt.
- **Repository `AGENTS.md` instructions apply to every role**, when the target worktree has a root `AGENTS.md`. There is no single shared Markdown prompt that is appended in every mode. In particular, `clarification.md`, `tdd-rounds.md`, and `writing.md` are **not universal**.
- The dispatcher is the primary authority: [bin/agent:2653–2765](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2653-L2765). The provider is selected only after the prompt has been built: [bin/agent:4647–4650](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4647-L4650), [bin/agent:4675–4680](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4675-L4680).

## 2. “Model,” “worker,” and “mode” mean different things

| Dimension | What determines it | Effect on instructions |
|---|---|---|
| Agent name | A configured entry such as `codex-1` or `inv-opus` | Supplies identity, branch, attribution, and configuration lookup keys. Names themselves do not select a prompt. |
| CLI type / provider | `AGENT_TYPE` or `INVESTIGATORS_TYPE`: `claude`, `codex`, `gemini` (Antigravity/`agy`), `opencode` | Selects the executable and MCP configuration, not a different role template. |
| Actual model | `AGENT_MODEL` or `INVESTIGATORS_MODEL`; empty means CLI default | Passed to the CLI. No `if MODEL == ...` routing exists in `build_prompt`. |
| Tier | Membership in `WORKER_AGENTS` means `worker`; otherwise `normal` | Selects conditional worker/normal behavior and some bus permissions. Merely being absent from `NORMAL_AGENTS` does not make an agent a worker. |
| Task kind | `KIND=worker`, `helper`, or `investigation` | Selects the base prompt, together with team/review metadata. **`KIND=worker` includes ordinary normal-tier tasks.** |
| Team/phase | `TEAM`, or `INVESTIGATION_TEAM` plus `INVESTIGATION_PHASE` | Distinguishes cooperative teams from the independent investigation pipeline. |
| Session mode | Internal `new`, `resume`, `fresh` | Controls whether the base prompt, only continuation instructions, or a complete handoff is delivered. This is distinct from CLI flags such as `--autopilot`. |

Configuration definitions: [config/agents.conf:7–65](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/config/agents.conf#L7-L65). Tier calculation: [bin/agent:304–308](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L304-L308). Ordinary task metadata: [bin/agent:5159–5163](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L5159-L5163). Local helper metadata: [bin/agent:4419–4423](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4419-L4423). Queued investigator/reviewer metadata: [bin/agent:4279–4287](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4279-L4287). Independent merger/reviewer metadata: [bin/agent:3699–3705](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3699-L3705).

The committed configuration is a **template**, not evidence of the host's active model choices. It illustrates normal `claude-1`, `codex-1`, and `gemini-1`, an empty worker list, and investigator `inv-opus`. The active installation may configure different names, providers, models, and extra CLI arguments. Consequently, “applies to Codex” below means **any configured Codex model running in that role**, equally with Claude, Gemini, and OpenCode.

## 3. Initial-prompt matrix: which file applies in which mode?

In this table, `+` means appended in order; “task” means the generated task's `task.md`, not necessarily the original human text alone. All rows also inherit the repository-instruction rule in section 4.

| Invocation / situation | Who receives it | Base and appended files on `new` | Selection evidence |
|---|---|---|---|
| `agent run <normal-agent> ...` | Normal solo model | `protocol.md` + `tdd-rounds.md` + `writing.md` + task + `clarification.md` + `ask-solo.md` | [bin/agent:2738–2764](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2738-L2764) |
| `agent run <worker-agent> ...` | Worker-tier solo model | Same as normal solo; obey worker branch inside `protocol.md` | [bin/agent:2673–2674](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2673-L2674), [bin/agent:2738–2764](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2738-L2764) |
| `agent run --team a,b ...` | Normal coordinator, normal members, worker members | `team.md` + `tdd-rounds.md` + `writing.md` + task + `clarification.md` + `ask-team.md` | [bin/agent:2733–2737](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2733-L2737), [bin/agent:2756–2764](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2756-L2764) |
| `agent run --team a,b --verify ...` | The same ordinary team | Same initial team prompt; later a queued investigator gets the review row below | [bin/agent:2723–2764](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2723-L2764), [bin/agent:2153–2184](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2153-L2184) |
| `agent run --team a,b --autopilot ...` | The same ordinary team | Same initial team prompt; later `autopilot-questions.md` if questions are bounced; two investigator reviews | [bin/agent:2723–2764](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2723-L2764), [bin/agent:2166–2172](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2166-L2172), [bin/agent:2796–2801](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2796-L2801) |
| Local `delegate(...)` request | Selected worker, `KIND=helper` | `helper.md` + request/context + `clarification.md`; **no** `ask-solo.md` | [bin/agent:2730–2732](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2730-L2732), [bin/agent:2756–2764](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2756-L2764), [bin/agent:4411–4423](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4411-L4423) |
| `request_investigation(...)` | Selected investigator, outside the requesting team | `investigator.md` + question/context + `clarification.md` + `ask-solo.md` | [bin/agent:2727–2729](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2727-L2729), [bin/agent:2756–2764](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2756-L2764), [bin/agent:4270–4287](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4270-L4287) |
| Queued PR review from `--verify` or `--autopilot` | Investigator, `KIND=investigation`, `REVIEW=1` | `review.md` + generated PR request/task + `clarification.md` + `ask-solo.md` | [bin/agent:2724–2726](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2724-L2726), [bin/agent:2756–2764](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2756-L2764), [bin/agent:4263–4268](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4263-L4268) |
| `agent run --investigation --team inv-a,inv-b ...`, Phase 1 | Each independent investigator | `inv-work.md` + `# TOPIC` + task | [bin/agent:2679–2703](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2679-L2703) |
| Same investigation, Phase 2 | Selected investigator acting as merger | `inv-merge.md` + topic/task | [bin/agent:2680–2689](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2680-L2689) |
| Same investigation, Phase 3 | Selected investigator acting as PR reviewer | `review.md` + topic/task, including independent-review instructions | [bin/agent:2686–2690](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2686-L2690), [bin/agent:3683–3687](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3683-L3687) |
| Same investigation, Phase 4 | No model is launched for the handoff itself | Script sends the handoff/notification and records completion; no new phase-specific prompt | [bin/agent:3630–3647](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3630-L3647) |
| `agent run --resume <investigation-parent> ...`, Phase 5 | Original investigator cohort, advisory reviewers | `review.md` + advisory instruction + original task and optional resume request | [bin/agent:2686–2690](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2686-L2690), [bin/agent:3354–3376](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3354-L3376) |

`ask-solo.md` and `ask-team.md` are optional on disk: the dispatcher checks that they exist. Other listed base/shared files are directly rendered/read. A generated skipped-tests note can follow ordinary prompts, but is explicitly suppressed for `KIND=investigation`: [bin/agent:1059–1068](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L1059-L1068).

**Important ordering:** independent investigation children are detected before any ordinary role selection. Their path returns at line 2703. Thus even though Phase 3/5 use `review.md`, they do **not** receive the ordinary review route's clarification/question appendices. `TEAM` alone does not identify an independent investigation; the special condition is `KIND=investigation` plus nonempty `INVESTIGATION_TEAM`: [bin/agent:2973–2982](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2973-L2982).

### Valid combinations and remote execution

- `--autopilot` implies `--verify`. New verified/autopilot tasks require `--team`. Ordinary teams require at least two members. Direct ordinary runs of investigator entries are rejected: [bin/agent:5633–5635](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L5633-L5635), [bin/agent:5698–5719](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L5698-L5719).
- `--investigation` is a separate pipeline. Combining it with `--autopilot`, `--verify`, `--need`, `--on`, `--last-team`, `--resume`, or `--same-round` is rejected. Resuming its parent later uses `--resume` without `--investigation`: [bin/agent:5644–5656](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L5644-L5656); lifecycle documentation: [README.md:169–178](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/README.md#L169-L178).
- `--same-round` and `--last-team` modify continuation/selection; they do not name additional Markdown templates. `--need` and `--on` route execution rather than create a special provider prompt: flag parsing at [bin/agent:5621–5642](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L5621-L5642).
- Remote node jobs are created as ordinary tasks; a remote team assignment additionally sets `TEAM`, `REMOTE_TEAM`, and tier. Therefore a remote standalone job selects the solo prompt and a remote team member selects the team prompt: [bin/agent:5241–5253](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L5241-L5253), [bin/agent:5159–5163](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L5159-L5163).
- **Remote delegated workers are an exception to “delegate means helper.md.”** The remote helper becomes a worker-selector job containing request/context, and node preparation uses ordinary task creation. It therefore selects `protocol.md`, not the local `helper.md`: [agentbus/helper_remote.go:68–76](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/agentbus/helper_remote.go#L68-L76), [bin/agent:5241–5253](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L5241-L5253). Local helper restrictions should not be assumed to reach that worker through `helper.md`.
- Remote team members get the shared `team.md`, but some tools it mentions are unavailable through the relay. The documented exclusions include resource reservations, coordinator election, delegation, investigator requests, and `wait_for_message`: [README.md:1015–1025](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/README.md#L1015-L1025). These exclusions are separate from tier.

## 4. Rules that ALWAYS apply, with the limits made explicit

“Always” means every supported model/role in a team-agents task, including independent investigators and reviewers. On a continuation, some rules survive through session history rather than being pasted again. It does not mean every rule requires an action in every job: for example, commit rules do not authorize a reviewer to commit.

### 4.1 Repository instructions: common to all seven base templates

The runner substitutes an **instruction to read** `AGENTS.md`; it does not inline the file's contents. When the root file exists, read it and the applicable nested `AGENTS.md` files; the nearest applies to its directory. Runtime prompts govern communication; repository instructions govern code and checks. Do not change `AGENTS.md`, `CLAUDE.md`, or `GEMINI.md` unless explicitly asked. If root `AGENTS.md` is absent, the generated text instead says to follow existing code/build/CI conventions, mention the absence, and not create the file unless asked. Source: [bin/agent:2616–2633](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2616-L2633).

Every base template contains this substitution:

| Base template | Exact location |
|---|---|
| Ordinary solo | [prompts/protocol.md:27](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/protocol.md#L27) |
| Ordinary team | [prompts/team.md:25](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L25) |
| Local helper | [prompts/helper.md:20](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/helper.md#L20) |
| Requested investigator | [prompts/investigator.md:24](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/investigator.md#L24) |
| Independent Phase 1 | [prompts/inv-work.md:26](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-work.md#L26) |
| Independent merger | [prompts/inv-merge.md:27](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-merge.md#L27) |
| Any PR reviewer | [prompts/review.md:14–17](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/review.md#L14-L17) |

For **this repository**, the binding root file gives these common rules. They are not automatically imposed on unrelated repositories lacking equivalent instructions:

| Rules in this repository | File and lines |
|---|---|
| Never modify existing tests/instructions/live installation; no real agent CLI tests; generated files, CI, secrets, dependencies, and compatibility constraints | [AGENTS.md:39–71](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/AGENTS.md#L39-L71) |
| Required tools and `make gate` before every commit; zero findings; how to handle failed checks | [AGENTS.md:73–109](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/AGENTS.md#L73-L109) |
| Go error types, enums, comments, logging, concurrency, MCP changes | [AGENTS.md:111–184](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/AGENTS.md#L111-L184) |
| Bash conventions and error handling | [AGENTS.md:186–198](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/AGENTS.md#L186-L198) |
| Prompt consistency and placeholders | [AGENTS.md:200–207](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/AGENTS.md#L200-L207) |
| Test-first development and deterministic tests | [AGENTS.md:209–223](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/AGENTS.md#L209-L223) |
| Documentation, commit/PR conventions, round-size limits, definition of done | [AGENTS.md:225–262](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/AGENTS.md#L225-L262) |

In this checkout only the root `AGENTS.md` was present. Other target repositories can have nested rules. For independent investigations, explicit no-bus/no-notebook instructions override the common repository text's suggestion to use `notebook_append`; that suggestion cannot grant an unavailable tool.

### 4.2 Common context, without a universal appended file

All seven base templates identify the role/worktree, give language instructions, include the repository-rule substitution, and describe tool-installation privileges. See their introductions: [prompts/protocol.md:1–27](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/protocol.md#L1-L27), [prompts/team.md:1–25](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L1-L25), [prompts/helper.md:1–20](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/helper.md#L1-L20), [prompts/investigator.md:1–24](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/investigator.md#L1-L24), [prompts/inv-work.md:1–26](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-work.md#L1-L26), [prompts/inv-merge.md:1–27](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-merge.md#L1-L27), [prompts/review.md:1–17](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/review.md#L1-L17).

The values come from [bin/agent:2655–2677](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2655-L2677); language and environment-file configuration are at [config/agents.conf:93–103](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/config/agents.conf#L93-L103). Installation privilege is checked for each run, with different text for passwordless sudo versus no sudo: [bin/agent:2636–2644](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2636-L2644). Review/helper language wording is role-specific; do not assume every result artifact uses the same language as a code comment.

Commit identity and trailers are generated centrally at [bin/agent:2581–2613](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2581-L2613). Templates that allow commits include the trailers; `review.md` instead forbids commits/pushes/merges. Hardware-note references are present in several base templates, but the local helper introduction has no such reference; its shared-resource use requires explicit request authorization.

**Not universal:** bus startup, notebook/reservations, quizzes, delegation, opening a PR, pushing commits, the standalone `clarification.md` text, `tdd-rounds.md`, and `writing.md`. Their scope follows the matrix and the role rules below.

## 5. The actual behavioral rules by role

### 5.1 Ordinary solo normal model and solo worker

Both receive [prompts/protocol.md:1–190](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/protocol.md#L1-L190).

| Section | Applies to | Rules |
|---|---|---|
| [prompts/protocol.md:29–64](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/protocol.md#L29-L64) | Both tiers | Bus startup, communication, repository notebook, reservation/release of shared resources. |
| [prompts/protocol.md:66–84](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/protocol.md#L66-L84) | Both tiers | Work on own branch; establish a draft PR; push/checkpoint work; do not work on the base branch. |
| [prompts/protocol.md:86–106](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/protocol.md#L86-L106) | Conditional | Normal models delegate mechanical work; workers work economically and precisely and may request an investigator for difficult work. Remote delegation has its own snapshot/push instructions. |
| [prompts/protocol.md:108–128](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/protocol.md#L108-L128) | Both tiers | Request an investigation when warranted, suspend appropriately, then assess its answer and record the decision. |
| [prompts/protocol.md:130–166](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/protocol.md#L130-L166) | Both tiers | Numbered human questions, recommendation and empty verdict, clarification stop. |
| [prompts/protocol.md:168–190](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/protocol.md#L168-L190) | Both tiers | Repository report and final status/result format. |

A solo worker therefore does **not** automatically inherit the local helper's “no commit/push unless requested” rule. Tier and local helper kind must not be conflated.

### 5.2 Ordinary cooperative team, including `--verify` and `--autopilot`

All members receive [prompts/team.md:1–214](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L1-L214); the same file contains coordinator/member and normal/worker branches.

| Section | Applies to | Rules |
|---|---|---|
| [prompts/team.md:27–35](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L27-L35) | Everyone; normal-only election | Shared solution, no idle member while work exists, workers do simple tasks; normal model claims coordinator. |
| [prompts/team.md:37–53](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L37-L53) | Coordinator and other members in different roles | Qualification quiz before task work; coordinator verifies reference answer and grades; failed members excluded and human asked. |
| [prompts/team.md:55–100](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L55-L100) | Coordinator | Split work into `easy` versus `normal`; normal models should not do simple items themselves; get worker summaries before reading unfamiliar code; integrate one shared PR and manage rounds. |
| [prompts/team.md:102–105](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L102-L105) | Everyone | Avoid unnecessary wakeups; batch messages. |
| [prompts/team.md:107–129](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L107-L129) | Members | Workers take only `easy`; normals take `normal`; verifiable results; own-branch commits/pushes; no separate member PR; questions go to coordinator; check board before sleeping. |
| [prompts/team.md:131–139](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L131-L139) | Everyone | Work while waiting; sleeping/status protocol; peer worktrees may be read but not modified. |
| [prompts/team.md:141–158](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L141-L158) | Role-dependent | External investigators/helpers, repository notebook, and resources. |
| [prompts/team.md:160–191](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L160-L191) | Coordinator, with member consultation | Consult normal members before sending consolidated questions to the human. |
| [prompts/team.md:193–214](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team.md#L193-L214) | Everyone | Final statuses and progress format. |

Neither workers nor investigators acquire coordinator status just because their CLI/model is powerful. Local workers are rejected by `claim_lead`; independent investigators never enter this team protocol.

### 5.3 Local delegated worker

[prompts/helper.md:24–30](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/helper.md#L24-L30) requires exactly the requested work, economical inspection, no changes to the requester's worktree, no commits or pushes unless requested, and no shared resources unless requested. Results must be precise and verifiable.

Its clarification target is the **requesting agent**, not the human: message the requester, wait up to 120 seconds, and report `UNCLEAR` if no answer. Its whole final message is delivered to that requester: [prompts/helper.md:32–47](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/helper.md#L32-L47). `clarification.md` is appended, but neither human-facing `ask-*` appendix is normally appended.

### 5.4 Investigator requested through `request_investigation`

[prompts/investigator.md:28–49](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/investigator.md#L28-L49) asks for empirical investigation, permits bus communication/reservations/notebook, disallows nested investigator requests, and permits local experimental commits while forbidding pushing or opening a PR. Human questions and the structured answer/result protocol are at [prompts/investigator.md:53–107](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/investigator.md#L53-L107).

This is **not** the current Phase 1 protocol. A requested investigator reports back to a requester; an independent investigator publishes a branch/report for later integration.

### 5.5 Independent `--investigation` phases

| Phase | Precise instructions |
|---|---|
| 1: investigate independently | [prompts/inv-work.md:32–38](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-work.md#L32-L38): no collaboration, quizzes, peer worktree access, bus, helpers, or nested investigations. [prompts/inv-work.md:39–51](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-work.md#L39-L51): test/code rules, commit and push, findings under `docs/investigations/<topic-slug>/<agent>.md`, no PR, completion statuses. [prompts/inv-work.md:53–69](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-work.md#L53-L69): its own clarification format. |
| 2: integrate results | [prompts/inv-merge.md:34–50](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-merge.md#L34-L50): only investigators, no bus/contact/delegation; read published commits selected by the manifest, not peer worktrees; preserve unique findings and test-first history; gate, commit, push. [prompts/inv-merge.md:51–64](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-merge.md#L51-L64): open/update the PR, record per-investigator attribution, never merge/close/enable auto-merge or push to the target branch. |
| 3: review the integrated PR | `review.md`, detailed below; plus [bin/agent:3683–3687](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3683-L3687): independent review, no changes/merge/delegation/contact with other investigators. |
| 4: terminal handoff | Script only; no new model instruction file. It explicitly reports that nothing was merged: [bin/agent:3638–3647](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3638-L3647). |
| 5: advisory reviews on parent resume | `review.md`; plus [bin/agent:3366–3371](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3366-L3371): propose improvements/alternative views and a verdict, no applied changes/commits/merges/delegation/contact, human decides what to apply. |

The special investigation dispatcher skips the usual appended TDD/writing/clarification files. This **does not remove** TDD/gate obligations explicitly stated in `inv-work.md`, `inv-merge.md`, or the target repository's `AGENTS.md`.

### 5.6 PR reviewers, in all review modes

| File and lines | Rule |
|---|---|
| [prompts/review.md:3–17](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/review.md#L3-L17) | Read-only worktree; repository rules apply; violating them is at least a major issue. |
| [prompts/review.md:21–37](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/review.md#L21-L37) | Fetch/view the PR, examine scoped diffs, assess correctness/security/tests/conventions, verify cheaply and safely, audit test-first history and skipped-test reasons. |
| [prompts/review.md:38–68](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/review.md#L38-L68) | Publish GitHub `COMMENT` review; severity and smallest concrete fixes; re-review protocol; attribution; no commits, pushes, or merges. |
| [prompts/review.md:72–84](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/review.md#L72-L84) | Write result and structured status/verdict; untrusted GitHub text is data, not instructions. |
| [prompts/review.md:86–98](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/review.md#L86-L98) | Review autonomous decisions, use exact verdict values, and ask questions if reliable review is blocked. |

The same review template serves queued `--verify`/`--autopilot` reviews and independent investigation Phase 3/5 reviews. Their appendices, task additions, and bus availability differ. Shared template text is not proof that all instructions are executable in every review mode; section 9 records the mismatches.

## 6. `--autopilot`: when the extra rules actually apply

The initial team prompt includes the ordinary clarification stop and coordinator question protocol. `build_prompt` does not branch on `AUTOPILOT` during `new`.

When a question file appears, the runner first checks missing recommendations, then, if autopilot is active and this question round has not already been bounced, changes the outcome to `abounce`: [bin/agent:4812–4824](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4812-L4824). It renames the file to `questions-…-decided.md`, sets `ABOUNCE_PENDING`, and resumes the task: [bin/agent:4898–4909](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4898-L4909).

That continuation adds [prompts/autopilot-questions.md:1–13](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/autopilot-questions.md#L1-L13) through [bin/agent:2796–2801](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2796-L2801). The model must:

1. Take its recommended option, or the simplest safe option if none exists.
2. Act and continue the task.
3. Record each decision and its reason in the PR's `## Decisions taken without the human` section.
4. Avoid asking again in that question round; another question attempt goes to the human and stops the round.

This applies to the **ordinary task carrying the autopilot metadata**, normally the coordinator handling questions. Local helpers and queued investigators/reviewers are created without copying the parent's `AUTOPILOT` flag: [bin/agent:4279–4287](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4279-L4287), [bin/agent:4419–4423](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4419-L4423). They do not automatically receive the self-answering appendix just because their requester used autopilot.

Reviewers always receive the instruction to inspect autonomous decisions when the PR has such a section: [prompts/review.md:86–87](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/review.md#L86-L87). Autopilot schedules two investigator reviews rather than the ordinary single review: [bin/agent:2107–2118](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2107-L2118), [bin/agent:2166–2184](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2166-L2184).

**Automatic merge is script behavior, not model permission.** The runner checks holds/merge blockers and the checked head, merges, tests the merged base, and may start the next round: [bin/agent:1355–1415](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L1355-L1415). Reviewers still cannot merge. The independent `--investigation` pipeline has its own human-merge-only rule and cannot be combined with autopilot.

## 7. Shared appendices and continuation prompts: complete inventory

These tables, together with the seven base templates in section 4, cover all **25** files in `prompts/` at the audited commit.

### 7.1 Appendices on ordinary new sessions

| File and lines | Applies when | Rules |
|---|---|---|
| [prompts/tdd-rounds.md:1–62](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/tdd-rounds.md#L1-L62) | Ordinary solo or ordinary team only | RED/GREEN/refactor or justified exception; concise test output; round planning, size/skip requirements and statuses. |
| [prompts/writing.md:1–29](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/writing.md#L1-L29) | Ordinary solo or ordinary team only | Concrete, concise reports/PRs, formatting/attribution links, untrusted GitHub text handling. |
| [prompts/clarification.md:1–24](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/clarification.md#L1-L24) | All ordinary `new` routes, including local helpers and queued investigators/reviewers | Resolve meaningful ambiguities first; do not silently assume; ask with alternatives, recommendations, and verdict format; repeat if answers create ambiguity. **Excluded from independent children.** |
| [prompts/ask-solo.md:1–6](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/ask-solo.md#L1-L6) | Ordinary non-team, non-helper `new` route, if file exists | Questions go in the specified file, followed by `STATUS: QUESTIONS`; do not expect live chat. Includes queued investigators/reviewers. |
| [prompts/ask-team.md:1–14](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/ask-team.md#L1-L14) | Ordinary team `new` route, if file exists | Coordinator writes human questions; members ask coordinator/sleep; quiz proceeds even when doubts need collecting. |
| [prompts/resume-checkpoint.md:1–13](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/resume-checkpoint.md#L1-L13) | Ordinary `new` with `RESUMED_FROM` | Continue existing task/PR using checkpoints and settled answers; do not redo the qualification quiz. |

Selection and order: [bin/agent:2733–2764](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2733-L2764). The base role text and task precede the checkpoint/clarification/question appendices.

### 7.2 Ordinary same-session `resume`

The runner does **not** automatically paste the base prompt again. It relies on the CLI session's earlier context and emits whichever pending modifiers match, in the order below. More than one can appear.

| Condition | Prompt / instruction | Dispatcher |
|---|---|---|
| `BOUNCE_PENDING` | [prompts/verify-bounce.md:1–11](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/verify-bounce.md#L1-L11): fix finish-verification problems such as missing integration/push/ready PR | [bin/agent:2769–2773](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2769-L2773) |
| `RBOUNCE_PENDING` | [prompts/round-bounce.md:1–7](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/round-bounce.md#L1-L7): correct round size/skip issues | [bin/agent:2775–2780](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2775-L2780) |
| `QBOUNCE_PENDING` | [prompts/questions-bounce.md:1–10](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/questions-bounce.md#L1-L10): add recommendations without changing numbering/empty verdicts | [bin/agent:2782–2787](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2782-L2787) |
| `SBOUNCE_PENDING` | [prompts/status-bounce.md:1–11](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/status-bounce.md#L1-L11): correct invalid final status | [bin/agent:2789–2794](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2789-L2794) |
| `ABOUNCE_PENDING` | [prompts/autopilot-questions.md:1–13](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/autopilot-questions.md#L1-L13): decide and document questions autonomously once | [bin/agent:2796–2801](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2796-L2801) |
| `RESTART_PENDING` | [prompts/restart-resume.md:1–10](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/restart-resume.md#L1-L10): inspect actual git/team state and continue interrupted work | [bin/agent:2803–2808](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2803-L2808) |
| `PAUSE_RELEASED` | Inline instruction: released board items must be claimed again before continuing them | [bin/agent:2810–2813](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2810-L2813) |
| `WAKE_PENDING` | [prompts/team-wake.md:1–11](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/team-wake.md#L1-L11): inspect messages/board, continue or sleep as appropriate | [bin/agent:2815–2819](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2815-L2819) |
| `ANSWERS_PENDING` | [prompts/resume.md:1–11](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/resume.md#L1-L11) + expanded human answers | [bin/agent:2821–2830](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2821-L2830) |
| `INV_PENDING_DELIVERY` | [prompts/inv-result.md:1–18](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-result.md#L1-L18) + investigator result/final: verify findings, decide and record the decision | [bin/agent:2832–2850](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2832-L2850) |
| Nonempty `note-<run>.md` | Additional human note | [bin/agent:2852–2855](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2852-L2855) |
| No pending modifier | One sentence to continue under the first message's rules | [bin/agent:2857–2859](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2857-L2859) |

An eligible skipped-tests reminder follows at [bin/agent:2860](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2860). The runtime clears the consumed pending fields after writing the prompt: [bin/agent:4651–4660](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4651-L4660).

### 7.3 Independent child resume, fresh sessions, and watchdog recovery

- **Independent child `resume`:** repeats the phase's base prompt and topic. With `ANSWERS_PENDING`, appends [prompts/inv-resume.md:1–10](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-resume.md#L1-L10) and answers; it also accepts a human note. It returns before the ordinary modifiers table: [bin/agent:2679–2703](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2679-L2703). Therefore ordinary `resume.md`, `team-wake.md`, and the other ordinary pending appendices must not be assumed to reach it.
- **Fresh session:** `build_prompt(new)` + [prompts/fresh-session.md:1–16](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/fresh-session.md#L1-L16) + last report/diff and eligible team checkpoint + `build_prompt(resume)`: [bin/agent:2705–2721](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2705-L2721). For independent children, those two nested builds both use the independent route, so the role/topic can appear twice; answers are added by the resume half.
- **Hang recovery:** [prompts/hang-resume.md:1–10](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/hang-resume.md#L1-L10) tells an eligible model to avoid blindly repeating the hanging operation, inspect state, and proceed differently. It is emitted outside `build_prompt`: [bin/agent:4694–4701](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4694-L4701). Team workers, local helpers, and all independent investigation children are excluded from automatic hang resume; solo workers are not excluded merely by tier: [bin/agent:4665–4671](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4665-L4671).

Do not equate `agent run --resume <task/team>` with a bare internal `resume` prompt: the public command can create a continued task with `RESUMED_FROM`, reapply full base instructions plus checkpoints, or dispatch the investigation parent's Phase 5. The relevant instructions remain the routes above.

### 7.4 Instructions generated outside Markdown templates

| Situation | Text source and effect |
|---|---|
| Ordinary pre-review checks fail | [bin/agent:1702–1708](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L1702-L1708): resume same round with failure details, fix on same branches, finish for review again. |
| Review requires a fix pass | [bin/agent:1764–1781](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L1764-L1781): fix/dispute critical and major findings, fix minor only if trivial, attach review text, resume team. |
| Autopilot starts next planned round | [bin/agent:1404–1409](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L1404-L1409): new task addition says prior PR is merged and names the next round. |
| Independent Phase 3 or Phase 5 | [bin/agent:3683–3687](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3683-L3687), [bin/agent:3366–3371](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3366-L3371): role-specific independence/advisory instructions in `task.md`. |
| Independent merger precheck bounce | [bin/agent:3819](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3819): append `# Checks to fix`; merger's existing protocol explains how to update the PR. |

The original human task, any allowed task additions, answer files, checkpoints, investigator results, and notes can all contribute instructions/context beyond `prompts/*.md`. A complete audit of a particular run must inspect the rendered prompt, not only a template.

## 8. CLI adapters and MCP instructions: what changes by model type?

The runner passes the same assembled prompt to all four CLI types. Their launch/configuration differences are:

| CLI type | Ordinary task, helper, queued investigator/reviewer | Independent investigation child, including merger and reviewer | Source |
|---|---|---|---|
| Claude | Configured `agentbus` MCP; `--model` if set; print/session mode | Empty MCP servers plus `--strict-mcp-config` | [bin/agent:2985–3003](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2985-L3003) |
| Codex | Configured `agentbus` MCP command/args; `-m` if set; exec/resume | Same server command declaration, explicitly `enabled=false` | [bin/agent:3013–3038](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3013-L3038) |
| Gemini / Antigravity | Workspace `.agents/mcp_config.json` contains agentbus; `--model` if set | Empty workspace MCP servers | [bin/agent:3053–3078](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3053-L3078) |
| OpenCode | Per-run configuration contains enabled agentbus; `-m` if set | Same complete server schema with `enabled=false` | [bin/agent:3095–3102](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3095-L3102), [bin/agent:3118–3131](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L3118-L3131) |

Each adapter also accepts configured `EXTRA_ARGS` and uses its noninteractive/automatic permission flags. Those flags are execution settings, **not** substitutes for role rules such as no pushing or no merging. These source statements establish how team-agents configures its own agentbus connection; they do not establish that every CLI has no other user-configured tools or hidden instructions.

For bus-enabled runs, additional model-facing text is supplied by MCP initialization: call `list_agents` and `read_messages`, reserve/release resources, use investigators for difficult problems, and read the notebook: [agentbus/main.go:1080–1082](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/agentbus/main.go#L1080-L1082). Tool descriptions/schema are another instruction surface.

The bus distinguishes **role** from **tier**: ordinary tasks/helpers use bus role `worker` even when their tier is normal; `KIND=investigation` changes that role to investigator: [bin/agent:2863–2875](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2863-L2875). Capability filtering and enforcement include:

| Rule | Enforcement / scope |
|---|---|
| Investigator does not get investigation-request or ordinary collaboration/consultation tools | Tool list assembly: [agentbus/main.go:1086–1095](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/agentbus/main.go#L1086-L1095); investigation tools excluded for investigator role: [agentbus/invest.go:382–385](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/agentbus/invest.go#L382-L385). |
| Worker cannot become coordinator | [agentbus/team.go:202–215](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/agentbus/team.go#L202-L215). |
| Worker can claim only `easy` board items | [agentbus/collab.go:478–499](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/agentbus/collab.go#L478-L499). The inverse “normal should leave easy items to workers” is a prompt instruction; this function does not symmetrically reject normal-tier easy claims. |
| Worker/investigator cannot call `delegate` | [agentbus/collab.go:599–602](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/agentbus/collab.go#L599-L602). |
| Investigator cannot request another investigator | [agentbus/invest.go:259–262](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/agentbus/invest.go#L259-L262). |
| Remote team has a reduced tool set | [agentbus/main.go:1093–1095](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/agentbus/main.go#L1093-L1095); user-visible exclusions: [README.md:1022–1025](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/README.md#L1022-L1025). |

Independent investigation children have no agentbus connection, so its startup instructions and tool descriptions do not reach them through that connection. Reviewer “read-only” behavior is principally a prompt contract; these adapters launch the same general coding CLIs rather than constructing a distinct OS-level read-only reviewer sandbox.

## 9. Confirmed gaps and inconsistencies affecting the answer

These findings describe the audited implementation. No fixes are included in this report.

1. **Autopilot reviewer independence and re-review context are constructed but omitted from the review task.** The queue question says not to read the other review before publishing one's own; on fix passes it includes the previous slot review: [bin/agent:2127–2144](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2127-L2144). `start_investigation` then builds reviewer `task.md` from only the PR number/URL and parent's task, dropping `.question`: [bin/agent:4263–4268](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4263-L4268). The review prompt consumes that task: [bin/agent:2724–2744](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2724-L2744). Ordinary `--verify` re-review questions at [bin/agent:2175–2182](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2175-L2182) are also omitted. Thus these generated queue instructions cannot honestly be listed as delivered prompt rules. `review.md` has a conditional re-review rule, but the normal delivery path drops the explicit request that identifies the pass. An isolated execution probe confirmed the omission. Recommendation: deliver the review question/context and test the resulting reviewer prompt, not merely the queue record.

2. **Independent reviewers are told to reserve resources without having reservation tools.** `review.md` requires `reserve`/`release` at [prompts/review.md:28–29](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/review.md#L28-L29), while independent Phase 3/5 use this template with agentbus disabled. The common `AGENTS_MD` text also suggests `notebook_append` at [bin/agent:2620–2625](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2620-L2625). Phase 1/2 explicitly forbid notebook/reservations; the shared review template does not adapt that language. Recommendation: make these clauses conditional on bus availability or provide an independent review variant; do not re-enable coordination to satisfy stale text.

3. **The independent answer prompt references a clarification section it did not receive.** [prompts/inv-resume.md:8–10](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/prompts/inv-resume.md#L8-L10) cites “CLARIFICATION RULES point 6,” but [bin/agent:2679–2703](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2679-L2703) bypasses `clarification.md`. The independent templates do contain their own clarification rules, so clarification itself is not absent. Recommendation: make `inv-resume.md` self-contained or reference the applicable independent section.

4. **Generic MCP startup text recommends a tool hidden from investigator roles.** [agentbus/main.go:1080–1082](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/agentbus/main.go#L1080-L1082) recommends `request_investigation` to every bus client; the investigator tool list excludes it and the handler rejects it. Recommendation: generate role-aware startup text. This affects queued investigators/reviewers, not independent children with no bus.

5. **Local and remote delegation do not deliver the same helper restrictions.** Local requests select `helper.md`; remote jobs select the ordinary solo route, as traced in section 3. This difference affects push/PR behavior and who is asked for clarification. Recommendation: explicitly define the remote helper contract and propagate enough role metadata to select it, or document the intended distinction.

6. **`AGENTS.md` and `CLAUDE.md` can give different repository instructions.** In this commit they are distinct regular files, not a symlink. For example, `AGENTS.md` requires `make gate` and explicit TDD/round rules ([AGENTS.md:83–109](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/AGENTS.md#L83-L109), [AGENTS.md:209–249](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/AGENTS.md#L209-L249)); `CLAUDE.md` has the older `make lint test` workflow and a shorter tests section ([CLAUDE.md:83–104](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/CLAUDE.md#L83-L104), [CLAUDE.md:204–215](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/CLAUDE.md#L204-L215)). README recommends a symlink for interactive Claude sessions: [README.md:685–690](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/README.md#L685-L690). The runner explicitly instructs all models to read `AGENTS.md`; whether a particular installed CLI also loads `CLAUDE.md` is outside this source audit. Recommendation: reconcile through a separately authorized instruction-file change. Neither file was modified here.

A useful cross-cutting improvement would be a prompt-content matrix test covering all roles/phases, including absence of unavailable tool instructions and delivery of generated task context. Existing route-specific tests do not eliminate the mismatches above.

## 10. How to identify the exact rules for an actual run

1. Check task metadata: `TYPE`, `MODEL`, `EXTRA_ARGS`, `KIND`, `TIER`, `TEAM`, `REVIEW`, `INVESTIGATION_TEAM`, `INVESTIGATION_PHASE`, `RESUMED_FROM`, `AUTOPILOT`, and session/run identifiers. Use the route table, not the agent name, to classify it.
2. Read `tasks/<id>/prompt-<run>.md`. The runner saves precisely the assembled text there: [bin/agent:4647–4650](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4647-L4650). Hang retries have their own `prompt-<run>.h<attempt>.md`: [bin/agent:4698–4701](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L4698-L4701). For resumed sessions, also inspect earlier prompt files because the continuation may contain only an addition.
3. Inspect that task's `task.md`, applicable answer/note/history/checkpoint artifacts, and root/nested repository instructions in its worktree. Prompt records show what was sent, not proof that a model read every referenced file or obeyed it.
4. Compare the **installed** prompt templates and active configuration with this pinned source. Defaults are `AGENTS_HOME=$HOME/agents`, configuration at `agents.conf`, prompts under `$AGENTS_HOME/prompts`: [bin/agent:202–207](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L202-L207). The installer copies templates into that directory: [install.sh:136–143](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/install.sh#L136-L143). Editing a checkout's `prompts/` does not itself update the active installation.
5. For a large prompt, the adapter may pass a read-file instruction pointing to `.agent-prompt.md` instead of all text in the CLI argument: [bin/agent:2949–2970](https://github.com/mateusz-szczyrzyca/team-agents/blob/6cb487f0caf967b8085c326e6298a9306559741c/bin/agent#L2949-L2970). The saved generated prompt remains the appropriate content record.

I did not inspect other investigators' worktrees, private active configuration, model-provider internals, or global CLI instruction files. No claim here establishes a vendor's complete system prompt, automatic native instruction-file discovery, or hidden policy hierarchy. This report covers the repository-controlled instruction and execution layers.

## 11. Verification, compatibility, and remaining work

- Read the repository instructions, README's mode/configuration sections, all 25 prompt templates, the prompt dispatcher, role/task construction, CLI adapters, and relevant bus permission paths at the pinned commit.
- Ran isolated shell probes against the extracted, unchanged `build_prompt` function, stubbing environment helpers and rendering template names. All **14** combinations passed: solo normal/worker, team normal/worker/autopilot, local helper, queued investigator/reviewer, independent phases 1/2/3/5, autopilot question continuation, and independent human-answer continuation. The probes also checked ordinary-versus-independent clarification inclusion. These are control-flow checks, not live-model compliance tests.
- Executed the extracted, unchanged `start_investigation` in a temporary directory with git/bus/runner side effects stubbed. A sentinel queue question was absent from the generated reviewer task while the parent task was retained, confirming finding 1. No real agent CLI or live service was started by these probes.
- Ran `make gate`: **all 7 steps passed** (`check-generated`, `lint-shell`, `lint-go`, `lint-docs`, `unit-go`, `unit-bats`, `harness`). Logs are in the local ignored `build/gate/` directory. Existing tests/helpers/stubs were not modified. The repository suite uses isolated temporary homes and stub agent CLIs.
- The change is documentation only: no on-disk format, CLI/MCP interface, prompt, permission, dependency, or configuration behavior changed; no new skips or lint suppressions. No README/config update is needed for runtime behavior because none changed. No PR was opened, per the investigation protocol.
- Test-first implementation is not applicable to the report itself. The findings above are recommendations for separately scoped, test-first fixes; they have not been implemented. No device access or installation changes were needed.

Confidence: **high** for the prompt dispatch and source-backed differences at this commit; actual installed configuration, CLI-native additions, and whether a model follows the instructions require a run-specific audit using section 10.
