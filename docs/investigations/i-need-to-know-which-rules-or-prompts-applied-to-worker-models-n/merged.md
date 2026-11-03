# Which rules and prompts reach which model, in which mode — merged result (inv-opus, Phase 2)

Combines both Phase 1 results, audited at base
[`6cb487f`](https://github.com/mateusz-szczyrzyca/team-agents/tree/6cb487f0caf967b8085c326e6298a9306559741c).
**Every line number refers to that commit.** The Phase 1 files [`inv-opus.md`](inv-opus.md) and
[`inv-astra.md`](inv-astra.md) are kept unchanged. `inv-astra.md` has a GitHub permalink for most references.
Tags such as *(opus)* and *(astra)* mark findings that only one investigator reported. Everything else was
found by both. I re-checked every reference that came from only one investigator against the source before
including it.

## 1. Short answer

- **One function builds every prompt:** `build_prompt` (`bin/agent:2653-2861`). It picks the files from the task's
  `KIND`, `REVIEW`, `TEAM`, `INVESTIGATION_TEAM`/`INVESTIGATION_PHASE` and the session mode (`new`, `resume`,
  `fresh`). **The model or CLI type (Claude, Codex, Gemini, OpenCode) never selects a prompt.** All four CLIs get the
  same text (`bin/agent:4647-4650`).
- **Normal and worker models get the same files.** That is `protocol.md` for a solo task and `team.md` for a team.
  The difference is made by conditional paragraphs inside those files, by `{{TIER}}`, and by the agentbus tools
  each tier gets.
- **A worker called through `delegate` gets `helper.md` instead.** This holds only for a local helper; a remote
  delegate gets `protocol.md` (§4.5).
- **"Investigator" means four different prompts:**
  - `investigator.md` answers a `request_investigation`;
  - `review.md` is for every PR review;
  - `inv-work.md` is Phase 1 of `--investigation`;
  - `inv-merge.md` is Phase 2.
- **`--verify` and `--autopilot` do not change anyone's first prompt.** They add a reviewer (an investigator with
  `review.md`) and follow-up prompts. `--autopilot` adds `autopilot-questions.md` when questions are bounced.
- **What applies always:** only the blocks every base template fills the same way (§3.1), the repository's
  `AGENTS.md` (§3.2), the instruction files the CLI loads by itself (§3.3) and a few runner mechanisms (§3.4).
  `clarification.md`, `tdd-rounds.md` and `writing.md` are **not** universal.

## 2. Terms

| Dimension | What decides it | What it changes |
|---|---|---|
| **Tier** `normal` / `worker` | membership in `WORKER_AGENTS` → `agent_tier` (`bin/agent:304-308`), stored as `TIER` (`bin/agent:5159-5163`). Investigator runs have no `TIER` and default to `normal` | `{{TIER}}` text, worker/normal paragraphs, agentbus tools (`delegate`, `claim_lead`, board) |
| **Kind** `worker` / `helper` / `investigation` | `KIND` in `meta.env`. **`KIND=worker` is every ordinary task, including normal-tier ones.** Helper: `KIND=helper TIER=worker` (`bin/agent:4419-4423`). Investigator/reviewer: `bin/agent:4279-4287`; independent phases: `bin/agent:3699-3705` | the base prompt |
| **Investigator** | an entry in `INVESTIGATORS_TYPE` (legacy `INVESTIGATOR_*`, `bin/agent:385-395`). Cannot be started with `agent run <name>` (`bin/agent:5718`) | always `KIND=investigation` |
| **Bus role** `worker` / `investigator` | `KIND=investigation` → `investigator`, everything else `worker`, **even a normal-tier task** (`bin/agent:2870-2871`) | agentbus tool list (§6) |
| **CLI type / model** | `AGENT_TYPE`/`AGENT_MODEL`, `INVESTIGATORS_TYPE`/`INVESTIGATORS_MODEL` (`config/agents.conf:7-65`) | executable, MCP config, which instruction files the CLI loads by itself (§3.3). Not the prompt |
| **Session mode** | `new` = first run; `resume` = later run in the same CLI session; `fresh` = later run in a new session | whether the full first prompt or only a follow-up is sent (§5) |

`config/agents.conf` is a template. It does not show which agents are workers on a given host.

---

## 3. ALWAYS — any model, any mode

### 3.1 Blocks in every base template

There are seven base templates: `protocol.md`, `team.md`, `helper.md`, `investigator.md`, `review.md`, `inv-work.md`
and `inv-merge.md`. Each has these blocks:

| Rule | Text source | Where it sits in each template |
|---|---|---|
| Unattended, unprivileged user | opening paragraph of each template | `protocol.md:3-5`, `team.md:3-5`, `helper.md:5`, `investigator.md:4-6`, `review.md:5`, `inv-work.md:4-6`, `inv-merge.md:4-5`. "The human does NOT answer live" is in all but `helper.md` (its "human" is the requester) and `review.md` |
| Language: `{{REPO_LANGUAGE}}` for the repository and GitHub, `{{LANGUAGE}}` for what only the human reads | `AGENT_LANGUAGE`/`REPO_LANGUAGE`, `bin/agent:327-331` | `protocol.md:8-10`, `team.md:7-9`, `helper.md:5-6`, `investigator.md:6-7`, `review.md:5-6`, `inv-work.md:8-10`, `inv-merge.md:7-9` |
| **Repository instructions** `{{AGENTS_MD}}`: read every `AGENTS.md`; they are binding for code and checks; do not change `AGENTS.md`/`CLAUDE.md`/`GEMINI.md` | `agents_md_text`, `bin/agent:2616-2632`. Without a root `AGENTS.md` a different paragraph says to follow the existing conventions (`bin/agent:2628-2631`) | `protocol.md:27`, `team.md:25`, `helper.md:20`, `investigator.md:24`, `review.md:14-17`, `inv-work.md:26`, `inv-merge.md:27` |
| Sudo / missing tools `{{PRIVILEGES}}` | `privileges_text`, `bin/agent:2636-2644`. It is decided again on every run | `protocol.md:25`, `team.md:23`, `helper.md:18`, `investigator.md:22`, `review.md:12`, `inv-work.md:24`, `inv-merge.md:25` |
| Commit identity and trailers `{{GIT_NAME}}`, `{{COMMIT_TRAILERS}}` | `commit_trailers`, `bin/agent:2607-2613`: `Agent`, `Agent-Model`, `Task`, `Team` in a team, `Investigation` when `INV_ID` is set. The identity is **enforced** by environment variables, whatever the prompt says (`git_identity`, `bin/agent:2583-2604`) | `protocol.md:12-17`, `team.md:11-16`, `helper.md:8-13`, `investigator.md:9-14`, `inv-work.md:12-17`, `inv-merge.md:11-16`. **Not in `review.md`**, which forbids commits (`review.md:68`) |

### 3.2 This repository's `AGENTS.md` (binding for every role through §3.1)

The prompt tells the model to read `AGENTS.md`; it does not paste the file's contents. In this repository the
file covers:

| Rules | `AGENTS.md` lines |
|---|---|
| "Never" rules: tests, instruction files, the running installation, real CLIs, generated files, CI, secrets, dependencies, compatibility | 39-71 |
| Setup, `make gate` before every commit, zero findings | 73-109 |
| Go: typed errors, enums, comments, logging, style, MCP tool checklist | 111-184 |
| Bash | 186-198 |
| Prompts | 200-207 |
| Tests: test first (RED → GREEN → REFACTOR) | 209-223 |
| Documentation, commits and PRs, rounds (3,000-line limit), definition of done | 225-262 |

For independent investigators the "no bus" text of their own prompt overrides the `notebook_append` hint in
`{{AGENTS_MD}}` (§8, item 8).

### 3.3 Instruction files the CLI loads by itself *(opus)*

Each CLI starts in the worktree (`run_claude` `bin/agent:3009`, `run_codex` `:3037`, `run_gemini` `:3076`,
`run_opencode` `:3128`). Its own discovery of instruction files therefore applies on top of the prompt. No code in
this repository controls it.

| CLI | Loads by itself (from the CLI's documentation; real CLIs were not run, `AGENTS.md` rule 4) |
|---|---|
| `claude` | `CLAUDE.md` in the working directory and every parent directory. **Confirmed in both Phase 1 and this Phase 2 run:** `/home/dev/CLAUDE.md` and `<worktree>/CLAUDE.md` were loaded into the context |
| `codex` | `AGENTS.md` from the git root to the working directory, plus `~/.codex/AGENTS.md` (absent on this host) |
| `opencode` | the project's `AGENTS.md` (or `CLAUDE.md` as a fallback), plus `~/.config/opencode/AGENTS.md` (absent) |
| `agy` (Antigravity) | not verified |

**`CLAUDE.md` is a stale copy of `AGENTS.md`**: two regular files, not a symlink, although `README.md:685-690`
recommends a symlink. Claude models therefore get both the old rules (loaded automatically, twice) and the new
`AGENTS.md` (because the prompt says to read it):

| `CLAUDE.md` | `AGENTS.md` |
|---|---|
| `make lint test` before every commit (`CLAUDE.md:88`) | `make gate` (`AGENTS.md:88-95`) |
| "New code comes with new tests" | test first, RED → GREEN, `Test-First: not possible` (`AGENTS.md:211-215`) |
| every commit passes the checks | except the RED commit (`AGENTS.md:235-236`) |
| no "Rounds" section | "Rounds", 3,000-line limit (`AGENTS.md:242-250`) |

Codex and OpenCode see only `AGENTS.md`.

### 3.4 Runner mechanisms for every run

- **What was sent is saved** in `tasks/<id>/prompt-<run>.md` (`bin/agent:4647-4650`). A hang retry is saved as
  `prompt-<run>.h<n>.md` (`bin/agent:4698-4701`).
- **Long prompts.** A prompt over `AGENT_PROMPT_ARG_MAX` is written to `.agent-prompt.md`. The CLI then gets only a
  pointer to that file (`cli_prompt`, `bin/agent:2953-2971`).
- **Fresh session.** Any later run can become `fresh`. It then gets the full `new` prompt, `fresh-session.md` with the
  last report and diff, the team's checkpoint, and the `resume` follow-ups (`bin/agent:2705-2721`).
- **Hang.** An eligible run gets only `hang-resume.md` (`bin/agent:4694-4702`). Who is not eligible is in §5.

### 3.5 Not universal

The following belong to specific routes only (§4):

- the files `clarification.md`, `tdd-rounds.md`, `writing.md`, `ask-*.md` and the skipped-tests note;
- agentbus startup, the notebook, reservations, the board and quizzes;
- `delegate`, opening a PR and pushing.

---

## 4. Mode by mode

### 4.1 Solo: `agent run <agent> "task"` (also `--need`, `--on`)

`KIND=worker`, no `TEAM`. Assembled at `bin/agent:2737-2765`, in this order:

1. `prompts/protocol.md` (1-190)
2. `prompts/tdd-rounds.md` (1-62)
3. `prompts/writing.md` (1-29)
4. `# TASK` + `task.md`
5. if `RESUMED_FROM` is set: `resume-checkpoint.md`, the checkpoints and `history.md` (`bin/agent:2745-2756`)
6. `prompts/clarification.md` (1-24), copied with `cat`, so no placeholders
7. `prompts/ask-solo.md` (1-6)
8. `skip_note`: the repository's skipped tests (`bin/agent:1059-1069`)

| `protocol.md` lines | Normal | Worker |
|---|---|---|
| 1-27 context (§3.1) | ✔ | ✔ |
| 29-64 bus startup, communication, notebook, reservations | ✔ | ✔ |
| 66-84 own branch, draft PR, push, checkpoints | ✔ | ✔ |
| 88 "You are: {{TIER}}" | "normal model" | "worker (a cheap model for simple tasks)" |
| 90-103 "If you are a normal model": `delegate`, `requires=[...]`, remote delegation | ✔ | ✘ (no `delegate` tool) |
| 105-106 "If you are a worker": work economically, consider `request_investigation` | ✘ | ✔ |
| 108-128 `request_investigation`; it begins "You are a cheaper everyday model" (§8, item 6) | ✔ | ✔ |
| 130-166 questions for the human | ✔ | ✔ |
| 168-190 report and final `STATUS` | ✔ | ✔ |

- A solo worker does **not** inherit `helper.md`'s "no commit/push unless asked" rule. Tier is not kind *(astra)*.
- After the run, both tiers get the same checks: the round check (`bin/agent:4924`), the questions-without-
  recommendation bounce (`:4814-4820`) and the unknown-`STATUS` bounce (`:4913`).
- `--need` and `--on` run the same solo prompt on the node that takes the job. The node creates an ordinary task
  (`bin/agent:5241-5253`).

### 4.2 Team: `agent run --team a,b "task"`

`KIND=worker`, `TEAM` set. Assembled at `bin/agent:2733-2736` and `2757-2764`, in this order:

1. `team.md` (1-214)
2. `tdd-rounds.md`
3. `writing.md`
4. `# TEAM TASK` + `task.md`
5. for a resumed team: the checkpoint blocks
6. `clarification.md`
7. **`ask-team.md`** (1-14), instead of `ask-solo.md`
8. `skip_note`

| `team.md` lines | Normal, coordinator | Normal, member | Worker member |
|---|---|---|---|
| 1-28 context, `{{TIER}}`, members, workers | ✔ | ✔ | ✔ |
| 30-35 start; `claim_lead` (`:32-33`) | claims | may claim | ✘ "Workers do not volunteer"; `claim_lead` rejects them |
| 37-49 quiz, coordinator side | ✔ | | |
| 51-53 quiz, member side | | ✔ | ✔ |
| 55-105 coordinator: board (`easy`/`normal`), worker summaries, one shared PR, merging, checkpoints, rounds | ✔ | | |
| 107-129 member: board items, results, own-branch commits, no member PR, questions go to the coordinator | | ✔ `normal` items | ✔ **only `easy`** (`:110-111`) |
| 131-139 waiting, sleeping; peer worktrees are read-only | ✔ | ✔ | ✔ |
| 141-152 investigators; `delegate` (`:147-152`) | ✔ | ✔ | `request_investigation` only |
| 154-158 notebook, resources | ✔ | ✔ | ✔ |
| 160-191 questions for the human, after `consult_*` with normal members | ✔ **only** | answers consults | ✘ |
| 193-214 final status, `board_progress` | ✔ | ✔ | ✔ |
| `ask-team.md:8-9` | ✔ | | |
| `ask-team.md:10-14` | | ✔ | ✔ |
| `tdd-rounds.md:61-62` rounds in a team | plans them | keeps its part small | ✔ |

Team-only mechanisms:

- **Wake-ups** use `team-wake.md` (§5).
- **Long-run note and cut.** At `AGENT_LONG_RUN_STEPS` every tool reply gets "end this run" appended
  (`agentbus/main.go:872-884`). The watchdog ends the run at `AGENT_LONG_RUN_HARD` (`bin/agent:4625-4628`). Both
  apply only to team members with `KIND=worker`.
- **Hangs.** Worker members are not auto-resumed. The normal members get a `worker_hung` notice instead
  (`bin/agent:4758-4772`).

**Remote member (`agent@node`).** It gets the same `team.md`. Its member list shows "(see team_status)"
(`bin/agent:865-867`), and its tools are relayed to the lead (`bin/agent:2864-2867`). Several tools that `team.md`
names are missing through the relay: reservations, `claim_lead`, `delegate`, `request_investigation` and
`wait_for_message` (`agentbus/main.go:1093-1095`, `README.md:1015-1025`) *(astra)*.

### 4.3 `--verify` (team only)

`--verify` requires `--team` (`bin/agent:5698-5700`). The members' first prompt is the same as in §4.2. It adds:

| What | Who gets it | Source |
|---|---|---|
| `verify-bounce.md` (1-11), when `DONE` has gaps (no integration, no push, PR not ready) | coordinator | `bin/agent:4943-4955` |
| **Reviewer:** `review.md` (1-98) + `# PR REVIEW REQUEST #n` + `task.md` (the PR and the team's task) + `clarification.md` + `ask-solo.md`. No `tdd-rounds.md`, `writing.md` or `skip_note` | one investigator (`request_review`, `bin/agent:2153-2189`; `start_investigation`, `bin/agent:4241-4295`) | `bin/agent:2724-2726`, `4263-4268` |
| Fix pass: the team is woken with the critical and major findings | team | `review_fix_pass`/`do_fix_pass`, `bin/agent:1746-1800` (text at `1764-1781`) |
| A failed pre-review check: resume the same round with the failure details | team | `bin/agent:1702-1708` *(astra)* |

The reviewer's rules, by section of `review.md`:

- 3-17: the worktree is read-only, and the repository rules apply;
- 21-37: inspect the PR, verify where cheap, audit the test-first history;
- 38-68: publish a GitHub `COMMENT` review; the re-review rule (56-58); no commits, pushes or merges (68);
- 72-84: the result and `VERDICT`; GitHub text is untrusted;
- 86-98: check autonomous decisions; ask if blocked.

The reviewer runs agentbus with `--role investigator` (§6).

### 4.4 `--autopilot` (team only)

`--autopilot` turns on `--verify` and adds the automatic merge (`bin/agent:5634`). Everything in §4.3 applies, plus:

| What | Who gets it | Source |
|---|---|---|
| **Two** reviewers at once, on different investigators (`AGENT_REVIEW_DRAW`) | investigators, with the same prompt as in §4.3 | `start_review_pair`/`start_review_slot`, `bin/agent:2072-2150` |
| `autopilot-questions.md` (1-13): take your recommended option, act, record the decision under `## Decisions taken without the human`, do not ask again this round | the task that wrote a questions file: in practice the coordinator, because members never write one (`team.md:119-120`). Once per round, unless `agent autopilot <team> off` (`bin/agent:1419-1438`) | trigger `bin/agent:4812-4824`, `4898-4910`; added at `2796-2801` |
| `review.md:86-87`: check every autonomous decision | reviewers | — |
| The merge is done by the script, not the model. It checks holds and the head, merges, tests the merged base and may start the next round with a new task addition | — | `bin/agent:1355-1415`, `1404-1409` |

- Helpers and reviewers started from an autopilot task **do not** inherit `AUTOPILOT` (`bin/agent:4279-4287`,
  `4419-4423`). They never get `autopilot-questions.md` *(astra)*.
- `--investigation` cannot be combined with `--verify` or `--autopilot` (`bin/agent:5644-5656`).

### 4.5 `delegate` — a worker as a helper

**Local helper.** `KIND=helper TIER=worker` (`bin/agent:4419`). Assembled at `bin/agent:2730-2732` and `2757-2764`:

1. `helper.md` (1-47)
2. `# REQUEST H<n>` + `task.md`
3. `clarification.md`
4. no `ask-*.md` (`bin/agent:2761`)
5. `skip_note`, which is still added (§8, item 4)

`helper.md` contains these rules:

- 22-30: do **exactly** what was asked; be economical; do not change the requester's worktree; no commit or push
  and no shared resources unless the request says so; give a precise, verifiable result.
- 32-36: your "human" is the requester. Send questions to it and wait 120 s; if it does not answer, finish with
  `UNCLEAR`.
- 38-47: the whole final message goes to the requester.

The helper's tools are bus role `worker` at tier `worker`. It has no `delegate`, but it has `request_investigation`
and `checkpoint`, which `helper.md` does not mention. It is not auto-resumed after a hang (`bin/agent:4670`).

**Remote helper (`delegate` with `requires=[...]` that the local host lacks)** *(astra)*. The request becomes a job
with `Selector: worker` (`agentbus/helper_remote.go:68-76`). On the node, `create_task` sets no `KIND=helper`
(`bin/agent:5241-5253`). The remote worker therefore gets the **solo** prompt (§4.1), `protocol.md`, not
`helper.md`.

### 4.6 `request_investigation` — an investigator answers another agent

`KIND=investigation`, `REVIEW` empty. Assembled at `bin/agent:2727-2729` and `2757-2764`:

1. `investigator.md` (1-107)
2. `# INVESTIGATION REQUEST #<n>` + `task.md` (the question and context, `bin/agent:4270-4273`)
3. `clarification.md`
4. `ask-solo.md`
5. no `skip_note`

`investigator.md` contains these rules:

- 28-49: investigate empirically; the bus, reservations and notebook are allowed; "You cannot request
  investigations yourself" (`:37`); local experimental commits are allowed, but no push and no PR.
- 51-107: questions for the human; the answer and result protocol.

The requester then gets `inv-result.md` (§5).

### 4.7 `agent run --investigation --team inv1,inv2 "topic"` (investigators only)

Only investigators are allowed, and only on the lead (`validate_investigation_team`, `bin/agent:5583-5598`;
pipeline `cmd__investigation_team`, `bin/agent:4181`). Each run is `KIND=investigation` with
`INVESTIGATION_TEAM` set, `TEAM` empty and `TIER=normal`. `is_independent_investigation` (`bin/agent:2974-2983`)
makes `build_prompt` **return early** at `bin/agent:2679-2703`, so these runs get:

- **no** `clarification.md`, `ask-*.md`, `tdd-rounds.md`, `writing.md` or `skip_note`;
- **no agentbus**:
  - claude: an empty `mcp.json` with `--strict-mcp-config` (`bin/agent:2989-2991`);
  - codex: the server `enabled=false` (`3013-3021`);
  - agy: an empty `.agents/mcp_config.json` (`3057-3058`);
  - opencode: `enabled: false` (`3095-3102`).

| Phase | Who | Prompt | Extra text in `task.md` / rules |
|---|---|---|---|
| 1 | **every** investigator in `--team`, in parallel (`bin/agent:4139-4141`) | `inv-work.md` (1-69) + `# TOPIC` | 32-38: strict independence, no bus, no peer worktrees, no helpers. 39-51: tests, commit, push, findings file, no PR. 53-69: its own question format |
| 2 | **one** merger (`investigation_phase_start`, `bin/agent:3651-3720`) | `inv-merge.md` (1-90) + `# TOPIC` | 34-50: published commits from the manifest only; keep unique findings and the test-first history. 51-64: open or update the PR; never merge or close. A failed precheck appends `# Checks to fix` (`bin/agent:3819`) |
| 3 | **one** reviewer, after the precheck passed (`REVIEW=1`) | `review.md` + `# TOPIC` | "Review this PR independently … Do not apply changes, merge, delegate, or contact other investigators" (`bin/agent:3684-3687`) |
| 4 | nobody: hand-off to the human; nothing is merged | — | `bin/agent:3630-3647`, `3767-3770` |
| 5 | `agent run --resume <investigation>`: **every** original investigator, as an advisory reviewer (`bin/agent:3392-3394`, `3409-3462`) | `review.md` + `# TOPIC` | "Advisory review … with an explicit VERDICT … The human decides what to apply" + the topic + `resume-request.md` (`bin/agent:3366-3372`) |

Follow-ups and runner behaviour for these phases:

- **A later run repeats the phase's base prompt and topic.** After the human's answers it appends `inv-resume.md`,
  not `resume.md` (`bin/agent:2691-2698`). It also appends a note from the human, if any (`2699-2702`). No other
  follow-up from §5 reaches these runs.
- There are no bounces: the run ends as `done`, `waiting` or `failed` (`bin/agent:4744-4757`). There is no hang
  auto-resume (`bin/agent:4671`).
- The `TDD/gate` rules still apply, through the text of `inv-work.md` and `inv-merge.md` and through `AGENTS.md`.

---

## 5. Follow-up prompts in later runs (`resume`, `bin/agent:2768-2860`)

These follow-ups do not apply to the independent phases of §4.7. In a `resume` run the first prompt is **not**
sent again: the CLI session still has it. Every pending item below is added, separated by `---`. If none applies,
the run gets one line: continue "following the rules from the first message" (`bin/agent:2857-2859`).
`skip_note` comes last (`bin/agent:2860`).

| Prompt | Trigger (where it is set) | Who gets it |
|---|---|---|
| `verify-bounce.md` (1-11) | `BOUNCE_PENDING` (`bin/agent:4949`) | `--verify`/`--autopilot` coordinator |
| `round-bounce.md` (1-7) | `RBOUNCE_PENDING` (`bin/agent:4930`) | solo agent or team coordinator |
| `questions-bounce.md` (1-10) | `QBOUNCE_PENDING` (`bin/agent:4890`): questions without `**Recommended:**` | anyone who wrote questions |
| `status-bounce.md` (1-11) | `SBOUNCE_PENDING` (`bin/agent:4915`): unknown `STATUS` | any run |
| `autopilot-questions.md` (1-13) | `ABOUNCE_PENDING` (`bin/agent:4904`) | `--autopilot` task (§4.4) |
| `restart-resume.md` (1-10) | `RESTART_PENDING` (`bin/agent:6329`) | any task interrupted by a machine restart |
| inline: "your board items went back, claim them again" (`bin/agent:2810-2814`) | `PAUSE_RELEASED` (`bin/agent:2213`) | team member paused longer than `AGENT_RELEASE_PAUSE_MIN` |
| `team-wake.md` (1-11) | `WAKE_PENDING` | team members |
| `resume.md` (1-11) + the answers | `ANSWERS_PENDING` (`agent resume`, `bin/agent:5911`) | everyone who asked |
| `inv-result.md` (1-18) + the investigator's `result.md` | `INV_PENDING_DELIVERY` (`bin/agent:4257`, `4315`) | the agent that called `request_investigation` (solo agent, team member or helper) |
| `note-<run>.md` | a note from the human | any run |
| `hang-resume.md` (1-10), **replaces** the prompt | watchdog (`bin/agent:4694-4702`) | `auto_resume=1`: normal models, coordinators, solo tasks of **either** tier, investigators answering requests, reviewers. **Not:** workers in a team, helpers, or independent phases (`bin/agent:4669-4671`) |
| `fresh-session.md` (1-16) + the whole first prompt | `fresh_session_due` (`bin/agent:2705-2721`, `4600-4604`) | any resumed task |
| `resume-checkpoint.md` (1-13), inside the `new` prompt | `agent run --resume <task\|team>` (`RESUMED_FROM`) | solo and team tasks |

The runner clears the consumed `*_PENDING` fields after writing the prompt (`bin/agent:4651-4660`).

---

## 6. agentbus: the third channel of instructions

Every run except the independent phases (§4.7) gets agentbus. Its instructions reach the model in three ways:

- **The `instructions` string** in `initialize` (`agentbus/main.go:1080-1082`). It tells the model to call
  `list_agents` and `read_messages` at the start, to reserve shared resources, that `request_investigation` exists
  and that `notebook_read` exists. Every role gets the same text, including investigators (§8, item 5).
- **The tool list and the tool descriptions.** They depend on role, team and tier (`agentbus/main.go:1086-1096`):

| Tools | Defined at | Who gets them |
|---|---|---|
| `list_agents`, `send_message`, `read_messages`, `read_message`, `set_status`, `reserve`, `release`, `list_reservations` | `agentbus/main.go:967+` | everyone with agentbus |
| `wait_for_message`, `notebook_read`, `notebook_append`, `notebook_write_section` | `agentbus/team.go:632-646` | everyone with agentbus |
| `team_status`, `claim_lead`, `handover_lead` | `agentbus/team.go:648-653` | team members. `claim_lead` rejects workers (`agentbus/team.go:202-215`) |
| `request_investigation`, `record_decision`, `investigation_status` | `agentbus/invest.go:383-420` | everyone except bus role `investigator`, **including workers and helpers**. The handler also rejects investigators (`agentbus/invest.go:259-262`) |
| `quiz_*`, `team_exclude`, `board_*` | `agentbus/collab.go:702-729` | team members (not investigators). A worker may claim only `easy` items (`agentbus/collab.go:478-499`). A normal model claiming an `easy` item is **not** rejected; the prompt asks it not to *(astra)* |
| `checkpoint` | `agentbus/collab.go:731` | everyone except investigators |
| `delegate`, `delegation_status` | `agentbus/collab.go:736-743` | tier `normal`, not investigators (enforced at `agentbus/collab.go:599-602`) |
| `consult_start`, `consult_reply`, `consult_results` | `agentbus/consult.go:219-227` | team members (not investigators) |
| a smaller set through the relay | `agentbus/main.go:1093-1095` | remote team members (§4.2) |

- **Notes appended to tool replies.** One is the long-run note (§4.2).

The reviewer's read-only rule is only a prompt rule: it runs the same CLI with the same permission flags.
`EXTRA_ARGS` and the CLIs' automatic-approval flags are execution settings, not role rules *(astra)*.

---

## 7. Summary matrix

✔ = included, — = not included.

| Source | Solo normal | Solo worker | Team normal | Team worker | Local helper | Remote helper | Investigator (request) | Reviewer (`--verify`/`--autopilot`) | `--inv` ph. 1 | ph. 2 | ph. 3 / 5 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `protocol.md` | ✔ | ✔ (`:105-106`) | — | — | — | ✔ | — | — | — | — | — |
| `team.md` | — | — | ✔ | ✔ (worker lines) | — | — | — | — | — | — | — |
| `tdd-rounds.md`, `writing.md` | ✔ | ✔ | ✔ | ✔ | — | ✔ | — | — | — | — | — |
| `helper.md` | — | — | — | — | ✔ | — | — | — | — | — | — |
| `investigator.md` | — | — | — | — | — | — | ✔ | — | — | — | — |
| `review.md` | — | — | — | — | — | — | — | ✔ | — | — | ✔ |
| `inv-work.md` / `inv-merge.md` | — | — | — | — | — | — | — | — | ✔ / — | — / ✔ | — |
| `clarification.md` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | — | — | — |
| `ask-solo.md` / `ask-team.md` | solo | solo | team | team | — | solo | solo | solo | — | — | — |
| `skip_note` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | — | — | — | — | — |
| `autopilot-questions.md` (follow-up) | — | — | `--autopilot` | `--autopilot` | — | — | — | — | — | — | — |
| `inv-resume.md` (follow-up) | — | — | — | — | — | — | — | — | ✔ | ✔ | ✔ |
| agentbus | + `delegate` | no `delegate` | team tools + `delegate` | team tools, `easy` only, no `delegate`/`claim_lead` | no `delegate` | no `delegate` | investigator role | investigator role | none | none | none |
| `{{AGENTS_MD}}` → `AGENTS.md` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| CLI's own files (`CLAUDE.md`/`AGENTS.md`, §3.3) | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |

---

## 8. Inconsistencies found (none fixed here; the topic asks for a report)

1. **The reviewer never sees its review request.**
   - The request is built at `start_review_slot` (`bin/agent:2127-2144`): "one of two independent reviews: do not
     read other reviews …", "Re-review after fix pass N …" and the previous review. `request_review`
     (`bin/agent:2175-2178`) builds "re-review after fix pass N".
   - `start_investigation` writes the reviewer's `task.md` from only the PR number, the URL and the team's task
     (`bin/agent:4263-4268`). It uses `.question` only for ordinary investigations (`:4271`).
   - So a reviewer never learns that it must be independent, or that this is a re-review, while `review.md:56-58`
     relies on "the request says 're-review after fix pass N'". inv-astra confirmed this with an isolated probe of
     the unchanged `start_investigation`.
2. **`CLAUDE.md` is stale** (§3.3). The human has to decide, because `AGENTS.md` forbids agents to edit it: copy the
   file again, or make it the symlink the README recommends.
3. **Prompts name a tool that does not exist:** `investigate` (`team.md:165`, `tdd-rounds.md:52`). agentbus has only
   `request_investigation` *(opus)*.
4. **Helpers are told to fill skipped tests.** `skip_note` says "Fill every one … test first" and is added to helper
   prompts (`bin/agent:1060`, `2764`, `2860`). That contradicts `helper.md:24`, "Do exactly what was asked"
   *(opus)*.
5. **The agentbus `instructions` string offers `request_investigation` to investigators**
   (`agentbus/main.go:1080-1082`), but they do not have it (§6).
6. **`protocol.md:110` tells every solo model "You are a cheaper everyday model"**, normal models included
   *(opus)*.
7. **`helper.md` does not mention two tools helpers have:** `request_investigation` and `checkpoint` *(opus)*.
8. **The independent phases get text that assumes a team and a bus.**
   - In phase 3/5, `review.md:3-4` speaks of "a team of agents" and `review.md:54-55` of "a `--verify` fix pass"
     *(opus)*.
   - `review.md:28-29` requires `reserve`/`release` with no bus *(astra)*.
   - `{{AGENTS_MD}}` suggests `notebook_append` in all phases (`bin/agent:2620-2625`) *(astra)*.
   - The task text and `inv-work.md`/`inv-merge.md` override these where they say so. `review.md` has no
     independent variant.
9. **`inv-resume.md:8-10` cites "CLARIFICATION RULES point 6"**, but the independent phases never get
   `clarification.md`. Their templates have their own question rules *(astra)*.
10. **Local and remote delegation give different rules** (§4.5). A remote helper gets `protocol.md`, which allows a
    PR and pushing and says to ask the human, not the requester *(astra)*.
11. **Polish text in prompts:** "Recenzja PR" (`bin/agent:2127`, `2175`, `4265`). `AGENTS.md` requires English
    prompts *(opus)*.
12. **An independent phase in a fresh session gets its prompt twice.** `fresh` calls `build_prompt new` and
    `build_prompt resume`, and both take the early return at `bin/agent:2679`. Even a plain `resume` repeats the
    base prompt.

A test of the prompt contents for each role and phase would catch items 1, 3, 5, 8 and 9. It would check that no
prompt mentions a tool the role lacks and that the generated task context is delivered *(astra)*.

## 9. How to find the exact rules of an actual run *(astra)*

1. Read `tasks/<id>/meta.env`: `TYPE`, `MODEL`, `KIND`, `TIER`, `TEAM`, `REVIEW`, `INVESTIGATION_TEAM`,
   `INVESTIGATION_PHASE`, `RESUMED_FROM`, `AUTOPILOT`. Classify the run with §4, not by its name.
2. Read `tasks/<id>/prompt-<run>.md`: exactly what was sent (§3.4). For a `resume`, also read the earlier prompt
   files, because a follow-up contains only the additions.
3. Add `task.md`, the answers, notes, checkpoints, the worktree's `AGENTS.md` and the CLI's own files (§3.3).
4. The installed templates are in `$AGENTS_HOME/prompts` (`bin/agent:202-207`), copied by `install.sh:136-143`.
   Editing `prompts/` in a checkout does not change a running installation.

## 10. How this was established

- **Both Phase 1 investigators:**
  - read `build_prompt`, the runner's end-of-run handling, `cmd_run`, the investigation pipeline,
    `start_investigation`, `start_helper`, the `run_*` adapters, all 25 files in `prompts/` and the agentbus tool
    gating;
  - compared `CLAUDE.md` with `AGENTS.md`.
- **inv-astra also:**
  - ran isolated probes of the unchanged `build_prompt`. All 14 role and phase combinations rendered the expected
    templates.
  - probed `start_investigation`, which confirmed item 1.
  - ran `make gate`: 7/7 steps passed.
- **inv-opus also** checked which home-directory instruction files exist on this host (§3.3).
- **This merge:**
  - re-read every reference that only one investigator gave (`bin/agent` dispatch, review queue, remote job
    creation, watchdog, `bus_args_json`; `agentbus/main.go`, `helper_remote.go`; the cited prompt lines);
  - found them correct;
  - confirmed again that Claude loads the stale `CLAUDE.md`, which happened in this run's own context.
- No real agent CLI was run (`AGENTS.md` rule 4). What a CLI loads by itself is taken from its documentation, except
  for Claude, which was observed.
