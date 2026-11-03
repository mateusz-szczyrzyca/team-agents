# Which rules and prompts each model gets, in each mode

Investigator: `inv-opus` (claude/opus). Base: `6cb487f`.

Every line number below refers to commit `6cb487f`. `bin/agent` is long, so its references are given as
`bin/agent:<line>`. They are read from the code, not from the README.

## 0. Terms: what "worker", "normal" and "investigator" mean in the code

| Term in this report | Where it comes from | What decides it |
|---|---|---|
| **normal model** | an agent in `AGENT_TYPE` (`agents.conf`) that is not listed in `WORKER_AGENTS` | `agent_tier` → `normal` (`bin/agent:304-308`), stored in the task's `meta.env` as `TIER=normal` (`bin/agent:5159`) |
| **worker model** | an agent listed in `WORKER_AGENTS` | `agent_tier` → `worker`. When a normal model calls `delegate`, a worker also runs a *helper* task: `KIND=helper TIER=worker` (`bin/agent:4419`) |
| **investigator** | an agent in `INVESTIGATORS_TYPE` (or the legacy `INVESTIGATOR_*`; `bin/agent:385-395`) | always `KIND=investigation`. Investigators cannot be started with `agent run <name>` (`bin/agent:5718`) |

The prompt of a run is built by **one function**, `build_prompt` (`bin/agent:2653-2861`). It looks at the
task's `KIND` (`worker` / `helper` / `investigation`), `REVIEW`, `TEAM`, `INVESTIGATION_TEAM`, `INVESTIGATION_PHASE`
and `TIER`, and at the run mode:

- `new`: the first run of a task.
- `resume`: any later run in the same CLI session.
- `fresh`: a later run that starts a new CLI session.

The prompt files live in `prompts/`. `render` (`bin/agent:858-863`) fills in `{{PLACEHOLDERS}}` from the
`R[...]` table (`bin/agent:2655-2678`).

A prompt is only one of four channels of instructions a model gets:

1. **the prompt** built by `build_prompt` (sections 2-6);
2. **the instruction files the CLI reads by itself** (section 1.3);
3. **what the agentbus MCP server tells it**: the `instructions` string, the tool list and tool descriptions, and
   notes appended to tool replies (section 1.4);
4. **follow-up prompts** for later runs: answers, bounces, wake-ups, hangs (section 7).

---

## 1. Rules that ALWAYS apply (any model, any mode)

### 1.1 Placeholders that every role prompt fills the same way

Every first prompt (`protocol.md`, `team.md`, `helper.md`, `investigator.md`, `review.md`, `inv-work.md`,
`inv-merge.md`) contains these blocks, filled by the same code:

- **The commit identity and trailers.** Text: `{{GIT_NAME}}`, `{{COMMIT_TRAILERS}}`
  - `protocol.md:12-17`, `team.md:11-16`, `helper.md:8-13`, `investigator.md:9-14`, `inv-work.md:12-17`,
    `inv-merge.md:11-16`
  - `review.md` has none: reviews must not commit (`review.md:68`).
  - Trailers come from `commit_trailers`, `bin/agent:2607-2613`: `Agent`, `Agent-Model`, `Task`, `Team` (in a
    team), `Investigation` (when `INV_ID` is set).
  - The identity is **enforced** whatever the prompt says, through environment variables: `git_identity`,
    `bin/agent:2583-2604`.
- **`{{PRIVILEGES}}`: the sudo / missing-tools rule.** `privileges_text`, `bin/agent:2636-2644`. It is decided
  again on every run.
- **`{{AGENTS_MD}}`: the "Repository instructions" paragraph.** `agents_md_text`, `bin/agent:2616-2632`. It says
  to read every `AGENTS.md`, that they are binding, and not to change `AGENTS.md`/`CLAUDE.md`/`GEMINI.md`. If the
  repository has no `AGENTS.md`, a different paragraph says so (`bin/agent:2628-2631`).
  - Placement: `protocol.md:27`, `team.md:25`, `helper.md:20`, `investigator.md:24`, `review.md:14`,
    `inv-work.md:26`, `inv-merge.md:27`
- **The language split.** `{{REPO_LANGUAGE}}` covers the repository and GitHub; `{{LANGUAGE}}` covers what only
  the human reads (`AGENT_LANGUAGE` / `REPO_LANGUAGE`, `bin/agent:327-331`).
  - `protocol.md:8-10`, `team.md:7-9`, `helper.md:5-6`, `investigator.md:6-7`, `review.md:5-6`, `inv-work.md:8-10`,
    `inv-merge.md:7-9`
- **"Unattended (headless), unprivileged user, the human does NOT answer live."** This is the first paragraph of
  every role prompt.

### 1.2 Mechanisms that apply to every run

- **Long prompts.** A prompt over `AGENT_PROMPT_ARG_MAX` bytes is passed as `.agent-prompt.md`, and the CLI gets
  only a one-line pointer to it (`cli_prompt`, `bin/agent:2953-2971`).
- **Hangs.** After a hang, a run with `auto_resume=1` gets only `prompts/hang-resume.md`
  (`bin/agent:4699-4702`). Section 7 lists who is *not* auto-resumed.
- **Fresh sessions.** Any `resume` can be turned into a `fresh` session (`fresh_session_due`). The prompt is then:
  the full `new` prompt + `prompts/fresh-session.md` (+ the team's checkpoint) + the `resume` follow-ups
  (`bin/agent:2705-2721`).

### 1.3 Instruction files the CLI reads by itself (no code in this repository decides this)

The runner starts each CLI with its working directory set to the worktree (`run_claude` `bin/agent:3009`,
`run_codex` `:3037`, `run_gemini` `:3076`, `run_opencode` `:3128`). So each CLI's own lookup of instruction files
applies on top of the prompt.

**On this host (verified):**

- the worktree has both `AGENTS.md` and `CLAUDE.md`;
- `/home/dev/CLAUDE.md` and `/home/dev/AGENTS.md` exist (copies of the repository's files);
- these do **not** exist: `~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`, `~/.gemini/GEMINI.md`,
  `~/.config/opencode/AGENTS.md`.

| CLI | Reads by itself (according to the CLI's own documentation; real CLIs were not run, `AGENTS.md` rule 4) |
|---|---|
| `claude` | `CLAUDE.md` in the working directory and in every parent directory. Confirmed for this run: `/home/dev/CLAUDE.md` and `<worktree>/CLAUDE.md` were both in my context |
| `codex` | `AGENTS.md` from the git root down to the working directory, plus `~/.codex/AGENTS.md` (absent) |
| `opencode` | `AGENTS.md` of the project (falls back to `CLAUDE.md`), plus `~/.config/opencode/AGENTS.md` (absent) |
| `agy` (Antigravity) | not verified: I could not confirm which files it loads by itself |

**Finding:** `CLAUDE.md` is an older copy of `AGENTS.md`, not the same file (`diff CLAUDE.md AGENTS.md`).
Claude models therefore get two different rule sets: `CLAUDE.md` loaded automatically (twice: the home copy and
the worktree copy) and `AGENTS.md` because the prompt says to read it. They contradict each other:

| `CLAUDE.md` says | `AGENTS.md` says |
|---|---|
| run `make lint test` before every commit (`CLAUDE.md:88`) | run `make gate` (`AGENTS.md:88-95`) |
| "New code comes with new tests" (`CLAUDE.md:206-207`) | tests come first, RED → GREEN, `Test-First: not possible` (`AGENTS.md:211-215`) |
| every commit passes the checks (`CLAUDE.md:227`) | the RED commit is the exception (`AGENTS.md:235-236`) |
| no "Rounds" section | "Rounds", 3,000-line limit (`AGENTS.md:242-250`) |
| the old "Definition of done" | the updated "Definition of done" (`AGENTS.md:254-256`) |

Codex and opencode only see `AGENTS.md`. See section 9.

### 1.4 What agentbus gives every model that has it

Who has agentbus: everyone except the phase-1/2/3/5 investigators of `--investigation` (section 6).

- **The `instructions` string** in the MCP `initialize` reply (`agentbus/main.go:1080-1082`). It tells every
  model to call `list_agents` and `read_messages` at the start, to reserve shared resources, and that
  `request_investigation` and `notebook_read` exist. Investigators get the same text, although they do not have
  `request_investigation` (section 9).
- **The tool list**, which depends on role and tier (`agentbus/main.go:1086-1096`):

| Tools | Defined at | Who gets them |
|---|---|---|
| `list_agents`, `send_message`, `read_messages`, `read_message`, `set_status`, `reserve`, `release`, `list_reservations` | `agentbus/main.go:967+` | everyone with agentbus |
| `wait_for_message`, `notebook_read`, `notebook_append`, `notebook_write_section` | `agentbus/team.go:632-646` | everyone with agentbus |
| `team_status`, `claim_lead`, `handover_lead` | `agentbus/team.go:648-653` | team members only. `claim_lead` refuses workers (`agentbus/team.go:39`, `:210`) |
| `request_investigation`, `record_decision`, `investigation_status` | `agentbus/invest.go:383-420` | everyone except `--role investigator` (`bin/agent:2870-2871`) — **workers and helpers included** |
| `quiz_*`, `team_exclude`, `board_*` | `agentbus/collab.go:702-729` | team members (not investigators). Workers may claim only `easy` items (`agentbus/collab.go:402`, `:493`) |
| `checkpoint` | `agentbus/collab.go:731` | everyone except investigators |
| `delegate`, `delegation_status` | `agentbus/collab.go:736-743` | tier `normal` only (not workers, not investigators) |
| `consult_start`, `consult_reply`, `consult_results` | `agentbus/consult.go:219-227` | team members (not investigators) |

- **The long-run note.** When a run passes `AGENT_LONG_RUN_STEPS`, every tool reply gets a note appended: "This
  run is long … end this run" (`agentbus/main.go:872-884`). This is armed only for team members with
  `KIND=worker` (`bin/agent:4625-4628`).

---

## 2. Solo task: `agent run <agent> "task"` (also `--need` / `--on` remote jobs)

`KIND=worker`, no `TEAM`. The first prompt is assembled at `bin/agent:2737-2765`.

| Order | Content | Applies to |
|---|---|---|
| 1 | `prompts/protocol.md` (whole file, lines 1-190) | normal **and** worker |
| 2 | `prompts/tdd-rounds.md` (1-62) | normal and worker |
| 3 | `prompts/writing.md` (1-29) | normal and worker |
| 4 | `# TASK` + `task.md` | |
| 5 | if resumed (`RESUMED_FROM`): `prompts/resume-checkpoint.md` + checkpoints + `history.md` (`bin/agent:2745-2756`) | |
| 6 | `prompts/clarification.md` (1-24), included with `cat` — no placeholders | |
| 7 | `prompts/ask-solo.md` (1-6) | |
| 8 | `skip_note` — the repository's skipped tests (`bin/agent:1059-1069`) | |

**Normal versus worker in solo mode.** Both get the same files. The difference is made by text that depends on
the model's tier:

- `protocol.md:88` — "You are: {{TIER}}" (`R[TIER]`, `bin/agent:2673`).
- `protocol.md:90-103` — "**If you are a normal model**": `delegate`, `requires=[...]`. This applies to the normal
  model only; workers do not have the `delegate` tool.
- `protocol.md:105-106` — "**If you are a worker**": work economically, consider `request_investigation`. This
  applies to the worker only.
- `protocol.md:108-128` — "Investigator" (`request_investigation`). This applies to both. It begins "You are a
  cheaper everyday model", which also normal models read.

**After the run**, these checks apply to both tiers:

- the round check (`bin/agent:4924`);
- the questions-without-recommendation bounce (`bin/agent:4814-4820`);
- the unknown-STATUS bounce (`bin/agent:4913`).

A hung **solo** worker *is* auto-resumed (`bin/agent:4670`: only workers in a team or on a helper request are not).

`--need` / `--on` (`bin/agent:5663-5694`) run the same solo prompt on the node that takes the job. The
`normal|worker` selector only picks the tier.

---

## 3. Team: `agent run --team a,b[,…] "task"`

`KIND=worker`, `TEAM` set. The first prompt is assembled at `bin/agent:2733-2736` and `2757-2764`.

| Order | Content | Applies to |
|---|---|---|
| 1 | `prompts/team.md` (1-214) | every member, normal and worker |
| 2 | `prompts/tdd-rounds.md` (1-62) | every member |
| 3 | `prompts/writing.md` (1-29) | every member |
| 4 | `# TEAM TASK` + `task.md` | |
| 5 | (resumed team) `resume-checkpoint.md` + the team's checkpoints + `history.md` | |
| 6 | `prompts/clarification.md` | |
| 7 | `prompts/ask-team.md` (1-14), instead of `ask-solo.md` (`bin/agent:2759-2760`) | |
| 8 | `skip_note` | |

Which parts of `team.md` apply to which role:

| Lines | Normal model, coordinator | Normal model, member | Worker member |
|---|---|---|---|
| `team.md:1-28` (context, `{{TIER}}`, members, workers) | ✔ | ✔ | ✔ |
| `team.md:30-35` start; `claim_lead` (`:33-34`) | ✔ claims | ✔ may claim | ✘ "Workers do not volunteer" |
| `team.md:39-49` quiz, coordinator side | ✔ | | |
| `team.md:51-53` quiz, member side | | ✔ | ✔ |
| `team.md:55-105` coordinator's role (board, PR, merging, checkpoints, rounds) | ✔ | | |
| `team.md:107-129` member's role | | ✔ `normal` items (`:111-112`) | ✔ **only `easy` items** (`:110-111`) |
| `team.md:131-139` waiting / sleeping | ✔ | ✔ | ✔ |
| `team.md:141-152` investigators; `delegate` (`:147-152`) | ✔ | ✔ | `request_investigation` only — no `delegate` tool |
| `team.md:154-158` notebook, resources | ✔ | ✔ | ✔ |
| `team.md:160-191` questions for the human | ✔ **coordinator only** | ✘ (asks the coordinator, `:119-124`) | ✘ |
| `team.md:193-214` last message, `board_progress` | ✔ | ✔ | ✔ |
| `ask-team.md:8-9` | ✔ | | |
| `ask-team.md:10-14` | | ✔ | ✔ |
| `tdd-rounds.md:61-62` (rounds in a team) | ✔ plans the rounds | ✔ "Test first", keep your part small | ✔ |

Team-only mechanisms:

- **Wake-ups** use `prompts/team-wake.md` (section 7).
- **Long-run cut** at `AGENT_LONG_RUN_HARD` steps (`bin/agent:4625-4628`).
- **Hangs.** Worker members are not auto-resumed after a hang. The team's normal members get a `worker_hung`
  notice instead (`bin/agent:4758-4772`).
- **Round check** for the coordinator only (`bin/agent:4924`).

A remote team member (`agent@node`) gets the same `team.md`. Its member list shows "(see team_status)"
(`bin/agent:865-867`), and its bus tools are relayed to the lead (`bin/agent:2864-2867`).

---

## 4. `--verify` and `--autopilot` (team only)

`--verify` is allowed only with `--team` (`bin/agent:5698-5700`). `--autopilot` turns on `--verify` and adds the
automatic merge (`bin/agent:5634`). `--investigation` cannot be combined with either (`bin/agent:5645-5647`).

### 4.1 Team members (normal and worker)

They get the **same first prompt as in section 3**. The flags add only follow-up prompts:

| Follow-up prompt | Trigger | Who gets it | Mode |
|---|---|---|---|
| `prompts/verify-bounce.md` (1-11) | the coordinator says `DONE` but there are gaps (`bin/agent:4943-4955`) | coordinator | `--verify`, `--autopilot` |
| a fix pass: the team is woken with the review's findings (`review_fix_pass` / `do_fix_pass`, `bin/agent:1746-1800`) | review with critical/major findings | team | `--verify`, `--autopilot` |
| `prompts/autopilot-questions.md` (1-13) | a run ends with a questions file; once per round (`bin/agent:4819-4820`, `4900-4910`) | whoever wrote questions — in practice the coordinator, because members never write them (`team.md:119-120`) | `--autopilot` only, unless `agent autopilot <team> off` (`bin/agent:1419-1438`) |

### 4.2 The reviewer: an investigator

Started by `request_review` (`bin/agent:2153-2189`) or, in `--autopilot`, by two reviews at once
(`start_review_pair` / `start_review_slot`, `bin/agent:2072-2150`). `start_investigation` (`bin/agent:4241-4295`)
creates `KIND=investigation REVIEW=1`, with no `TEAM` and no `TIER` (so the tier defaults to `normal`).

| Order | Content |
|---|---|
| 1 | `prompts/review.md` (1-98) (`bin/agent:2724-2726`) |
| 2 | `# PR REVIEW REQUEST #<n>` + `task.md`: the PR number and URL and the team's task (`bin/agent:4263-4268`) |
| 3 | `prompts/clarification.md` |
| 4 | `prompts/ask-solo.md` — because `TEAM` is empty and `KIND` is not `helper` (`bin/agent:2761-2762`) |
| — | no `skip_note` (it skips `KIND=investigation`, `bin/agent:1060`); no `tdd-rounds.md` or `writing.md` (the review checks TDD itself: `review.md:30-37`) |

- agentbus with `--role investigator`: only the basic tools and the `wait_for_message`/notebook tools.
- `review.md:86-87` (decisions taken without the human) matters only for `--autopilot` PRs.
- In `--autopilot`, the two reviews run on two different investigators (`AGENT_REVIEW_DRAW`).

**Finding:** the instructions in the review request never reach the reviewer. Section 9.1 has the details.

---

## 5. Requests from inside a run: `delegate` (worker) and `request_investigation` (investigator)

### 5.1 Helper: a worker model on a `delegate` request

`KIND=helper TIER=worker` (`bin/agent:4419`). The prompt is assembled at `bin/agent:2730-2732` and `2757-2764`:

1. `prompts/helper.md` (1-47)
2. `# REQUEST H<n>` + `task.md`
3. `prompts/clarification.md`; `helper.md:32-36` redirects "the human" to the requester
4. no `ask-*.md` (`bin/agent:2761`)
5. **`skip_note`**: it still runs (`bin/agent:2764`; `bin/agent:1060` excludes only investigations)

Other rules for helpers:

- Its tools: role `worker`, tier `worker`. So it has no `delegate`, but it **does** have `request_investigation`
  and `checkpoint`, which `helper.md` never mentions.
- It is not auto-resumed after a hang (`bin/agent:4670`). A turn without a `STATUS` line is returned to the
  requester as the result.

### 5.2 Investigator answering `request_investigation`

`KIND=investigation`, `REVIEW` empty. The prompt is assembled at `bin/agent:2727-2729` and `2757-2764`:

1. `prompts/investigator.md` (1-107)
2. `# INVESTIGATION REQUEST #<n>` + `task.md` (the question and context, `bin/agent:4270-4273`)
3. `prompts/clarification.md`
4. `prompts/ask-solo.md`
5. no `skip_note`

Its tools: role `investigator`, which is what `investigator.md:37` says ("You cannot request investigations
yourself"). The requester then gets `prompts/inv-result.md` (section 7).

---

## 6. `agent run --investigation --team inv1,inv2 "topic"` (investigators only)

Allowed only with investigators, on the lead (`validate_investigation_team`, `bin/agent:5583-5598`). The pipeline
is `cmd__investigation_team` (`bin/agent:4181`). Every run is `KIND=investigation`, has `INVESTIGATION_TEAM` set,
`TEAM` empty and `TIER=normal`.

`is_independent_investigation` (`bin/agent:2974-2983`) makes `build_prompt` **return early**, at
`bin/agent:2679-2703`. So **none** of these is added:

- `clarification.md`
- `ask-*.md`
- `tdd-rounds.md`
- `writing.md`
- `skip_note`

There is also **no agentbus**:

- `claude`: an empty `mcp.json` with `--strict-mcp-config` (`bin/agent:2989-2991`)
- `codex`: the server disabled (`bin/agent:3013-3021`)
- `agy`: an empty `.agents/mcp_config.json` (`bin/agent:3057-3058`)
- `opencode`: `enabled: false` (`bin/agent:3095-3102`)

| Phase | Who | Prompt | Task text under `# TOPIC` |
|---|---|---|---|
| 1 | **every** investigator named in `--team`, in parallel (`bin/agent:4139-4141`) | `prompts/inv-work.md` (1-69) | the topic |
| 2 | **one** investigator, the merger (`investigation_phase_start`, `bin/agent:3651-3720`) | `prompts/inv-merge.md` (1-90) | the topic; after a failed precheck, a `# Checks to fix` section is appended (`bin/agent:3819`) |
| 3 | **one** investigator, the reviewer, after the precheck passed (`REVIEW=1`) | `prompts/review.md` (1-98) | "Review this PR independently. …" + the topic (`bin/agent:3686-3687`) |
| 4 | nobody: hand-off to the human (`bin/agent:3767-3770`) | — | — |
| 5 | `agent run --resume <investigation>`: **every** original investigator, as an advisory reviewer (`bin/agent:3392-3394`, `3409-3462`) | `prompts/review.md` | "Advisory review of the existing PR … with an explicit VERDICT. …" + the topic + `resume-request.md` (`bin/agent:3369-3372`) |

Follow-ups:

- After the human answers, the follow-up is `prompts/inv-resume.md`, not `resume.md` (`bin/agent:2691-2698`).
- A note from the human is appended (`bin/agent:2699-2702`).
- No bounces: an independent run ends only as `done`, `waiting` or `failed` (`bin/agent:4744-4757`).
- No hang auto-resume (`bin/agent:4671`).

For phase 3 and 5 reviewers, parts of `review.md` describe a different situation:

- `review.md:3-4`: "A team of agents has finished …"
- `review.md:54-55`: "a `--verify` fix pass"

The task text overrides them: no changes, no merge.

---

## 7. Follow-up prompts in later runs (`resume` mode, `bin/agent:2768-2860`)

These come after the first prompt of the session. Several can appear in one run, separated by `---`. If none
applies, the run gets "Continue task … following the rules from the first message" (`bin/agent:2857-2859`).
`skip_note` is appended at the end (`bin/agent:2860`).

| Prompt | Trigger (who sets it) | Applies to |
|---|---|---|
| `verify-bounce.md` | `BOUNCE_PENDING` (`bin/agent:4949`) | `--verify`/`--autopilot` coordinator |
| `round-bounce.md` | `RBOUNCE_PENDING` (`bin/agent:4930`) | solo agent or team coordinator, `KIND=worker` |
| `questions-bounce.md` | `QBOUNCE_PENDING` (`bin/agent:4890`) | anyone who wrote questions without `**Recommended:**`, except independent investigators |
| `status-bounce.md` | `SBOUNCE_PENDING` (`bin/agent:4915`) | any non-independent run with an unknown `STATUS` |
| `autopilot-questions.md` | `ABOUNCE_PENDING` (`bin/agent:4904`) | `--autopilot` team |
| `restart-resume.md` | `RESTART_PENDING` (`bin/agent:6329`) | any task interrupted by a machine restart |
| inline text: "board items went back" (`bin/agent:2810-2814`) | `PAUSE_RELEASED` (`bin/agent:2213`) | team member paused longer than `AGENT_RELEASE_PAUSE_MIN` |
| `team-wake.md` | `WAKE_PENDING` | team members |
| `resume.md` | `ANSWERS_PENDING` (`agent resume`, `bin/agent:5911`) | everyone except independent investigators (they get `inv-resume.md`) |
| `inv-result.md` + the investigator's `result.md` | `INV_PENDING_DELIVERY` (`bin/agent:4257`, `4315`) | the agent that called `request_investigation` (solo agent, team member, or helper) |
| `hang-resume.md` (replaces the prompt) | watchdog (`bin/agent:4699-4702`) | `auto_resume=1`: normal models, coordinators, solo agents (any tier), investigators answering requests, reviewers. **Not** workers in a team, helpers, or independent investigators (`bin/agent:4670-4671`) |
| `fresh-session.md` + the whole first prompt | `fresh_session_due` (`bin/agent:2705-2721`, `4600-4604`) | any resumed task |
| `resume-checkpoint.md` (inside the `new` prompt) | `agent run --resume <team\|task>` (`RESUMED_FROM`) | solo and team tasks |

---

## 8. Summary matrix

✔ = included, — = not included.

| Rule source | Solo normal | Solo worker | Team normal (coord./member) | Team worker | Helper (delegate) | Investigator (request) | Reviewer `--verify`/`--autopilot` | `--investigation` ph. 1 | ph. 2 | ph. 3 / 5 |
|---|---|---|---|---|---|---|---|---|---|---|
| `protocol.md` | ✔ | ✔ (`:105-106`) | — | — | — | — | — | — | — | — |
| `team.md` | — | — | ✔ | ✔ (worker lines) | — | — | — | — | — | — |
| `tdd-rounds.md`, `writing.md` | ✔ | ✔ | ✔ | ✔ | — | — | — | — | — | — |
| `helper.md` | — | — | — | — | ✔ | — | — | — | — | — |
| `investigator.md` | — | — | — | — | — | ✔ | — | — | — | — |
| `review.md` | — | — | — | — | — | — | ✔ | — | — | ✔ |
| `inv-work.md` / `inv-merge.md` | — | — | — | — | — | — | — | ✔ / — | — / ✔ | — |
| `clarification.md` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | — | — | — |
| `ask-solo.md` / `ask-team.md` | solo | solo | team | team | — | solo | solo | — | — | — |
| `skip_note` | ✔ | ✔ | ✔ | ✔ | ✔ | — | — | — | — | — |
| agentbus | full + `delegate` | no `delegate` | team tools + `delegate` | team tools, no `delegate`, `easy` items only | no `delegate` | investigator role | investigator role | none | none | none |
| `AGENTS.md` paragraph (`{{AGENTS_MD}}`) | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| CLI's own files (`CLAUDE.md` / `AGENTS.md`) | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |

`--verify` and `--autopilot` do not change the first prompt of any member. They add the reviewer column and the
follow-ups in section 4.1. `--autopilot` additionally gives `review.md:86-87` meaning.

---

## 9. Inconsistencies found while mapping (for the merger and the human)

These come from reading the code. None was fixed on this branch: the topic asks for a report.

1. **The review request text is dropped.**
   - `start_review_slot` builds the question (`bin/agent:2127-2139`): "one of two independent reviews: do not read
     other reviews …", "Re-review after fix pass N …", and the previous review.
   - `request_review` builds "re-review after fix pass N" (`bin/agent:2175-2178`).
   - For `kind == review`, `start_investigation` writes `task.md` from only the PR number, the URL and the team's
     task (`bin/agent:4263-4268`). `.question` is used only for ordinary investigations (`bin/agent:4271`).
   - Result: a reviewer never sees the independence rule, or that it is a re-review or which findings came
     before. Yet `review.md:56-58` relies on "the request says 're-review after fix pass N'".
2. **`CLAUDE.md` is stale.** Claude models get the old rules automatically, and the prompt also points them to
   the new `AGENTS.md`. The conflicts are in section 1.3. `AGENTS.md` itself forbids editing `CLAUDE.md`, so the
   human has to decide (copy it again, or make it a symlink).
3. **A tool the prompts name does not exist.** `team.md:165` and `tdd-rounds.md:52` tell agents to use
   `investigate`. agentbus has only `request_investigation`.
4. **Helpers are told to fill skipped tests.**
   - `skip_note` is appended to helper prompts (`bin/agent:1060`, `2764`, `2860`). It says "Fill every one …
     test first".
   - That contradicts `helper.md:24`: "Do exactly what was asked — nothing more".
5. **The agentbus instructions string advertises a tool investigators lack.** It mentions
   `request_investigation` to every role (`agentbus/main.go:1080-1082`), but investigators do not have that tool.
6. **`protocol.md` tells normal models they are cheap.** Its "Investigator" section, `protocol.md:110` ("You are
   a cheaper everyday model"), goes to normal-tier solo agents too.
7. **Helpers are not told about two tools they have.** They have `request_investigation` and `checkpoint`;
   `helper.md` does not mention either.
8. **Phase 3/5 review prompts describe a different setup.** `review.md` speaks of a team and a `--verify` fix
   pass in a pipeline that has neither (section 6).
9. **Leftover Polish text.** "Recenzja PR" appears in prompt text (`bin/agent:2127`, `2175`, `4265`), but prompts
   must be in English (`AGENTS.md`, "Prompts").
10. **The topic is printed twice in a fresh session of an independent investigator.** A `fresh` session of an
    independent run renders `inv-work.md` + `# TOPIC` twice: `build_prompt … new` and `build_prompt … resume`,
    both through the early return at `bin/agent:2679`. This happens only if a fresh session becomes due after
    the human's answers.

## How this was established

- Read `build_prompt` and its helpers, `cmd__runner`'s end-of-run handling, `cmd_run`'s flag parsing, the
  investigation pipeline, `start_investigation`, `start_helper`, and the `run_*` CLI starters in `bin/agent`.
- Read every file in `prompts/` and the agentbus tool gating (`main.go`, `invest.go`, `team.go`, `collab.go`,
  `consult.go`).
- Compared `CLAUDE.md` with `AGENTS.md` and checked which home-directory instruction files exist.
- No real CLI was run, so the CLI auto-load behaviour (section 1.3) is from the CLIs' documentation. For Claude
  it is confirmed by this run's own context.
