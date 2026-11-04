# Investigation Report — Synchronizing `claude-haiku` in Multi-Agent Rounds (`inv-gemini-argon`)

- Investigator: `inv-gemini-argon` (`gemini`, model `gemini/gemini-4-argon-high`)
- Task: `20261104-130420-inv-gemini-argon`
- Branch: `agents/inv-gemini-argon/20261104-130420` (base [`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f))
- Commits:
  - $\color{red}{\textbf{RED}}$ [`60cd9a1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/60cd9a16d2b69983198e058f6ebbd8c0977a8ade) — `Add team member synchronization and round-PR guard tests (red)`
  - $\color{green}{\textbf{GREEN}}$ [`f002bee`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f002bee193c6828e747f553bd7b9c4f23894250b) — `Synchronize team members and enforce single-PR round ownership (green)`

---

## 1. Why `claude-haiku` acts alone and fails to synchronize with the team

Forensic inspection of all `30` historical team runs of `claude-haiku` in `gemini-1,codex-1,claude-haiku` teams (`20261102-175849-claude-haiku` through `20261104-122828-claude-haiku`) identified three distinct root causes:

1. **Claude Code `2.1.x` defers all `agentbus` MCP tools behind `ToolSearch` (`"tst"` mode) by default when `ENABLE_TOOL_SEARCH` is unset:**
   - In `12` of `30` team runs (`40%`), `claude-haiku` made **`0` `agentbus` MCP tool calls**:
     - `20261102-175849-claude-haiku`
     - `20261103-075558-claude-haiku`
     - `20261103-082115-claude-haiku`
     - `20261103-085533-claude-haiku`
     - `20261103-101715-claude-haiku`
     - `20261103-143807-claude-haiku`
     - `20261103-184910-claude-haiku`
     - `20261103-195812-claude-haiku`
     - `20261103-221740-claude-haiku`
     - `20261104-121211-claude-haiku`
     - `20261104-122828-claude-haiku`
   - Because Claude Code `2.1.289` defaults to `"tst"` (Tool Search Tool) mode unless `ENABLE_TOOL_SEARCH=false` is set, none of the `mcp__agentbus__*` tool schemas are loaded in `claude-haiku`'s initial tool definitions.
   - In `3` runs (`20261102-175849-claude-haiku`, `20261103-075558-claude-haiku`, `20261103-184910-claude-haiku`), `claude-haiku` called `ToolSearch` (`select:team_status,read_messages,...`), received `[{"type":"tool_reference","tool_name":"mcp__agentbus__team_status"}, ...]`, and then executed `mcp__agentbus__team_status` as a **Bash shell command**, failing with `Exit code 127: command not found: mcp__agentbus__team_status`.
   - In `8` other runs (including `20261104-121211-claude-haiku` and `20261104-122828-claude-haiku`, where `prompt-1.md` was `111,890` and `112,671` bytes — over `AGENT_PROMPT_ARG_MAX=100000` — and was read in partial chunks from `.agent-prompt.md`), `claude-haiku` never called `ToolSearch` at all.
   - In both cases, `claude-haiku` then tried running `agentbus team-get 20261104-121211-team`, `agentbus team-status`, or `agentbus read-messages` in Bash. Because `team-get` requires `--team <id>` (not a positional argument), [`agentbus/team.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/agentbus/team.go#L780) printed `agentbus: team-get: no team` (exit code `3`), and [`agentbus/main.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/agentbus/main.go#L1543) printed `unknown subcommand "team-status"`. Misinterpreting `agentbus: team-get: no team` as "no active team exists on the bus", `claude-haiku` worked completely solo for the rest of the run.
2. **Even when `claude-haiku` discovered MCP tools (`18` of `30` runs), when the board had no open `todo` item for it, it acted as an uncoordinated second coordinator instead of sleeping:**
   - Rather than following [`prompts/team.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/prompts/team.md#L127-L130) (`STATUS: SLEEPING` when no current-round items remain for the member), `claude-haiku` created new branches via `git checkout -b` (`agents/claude-haiku/m3-test-first` in `20261103-082115-claude-haiku`, `claude-haiku-m2-fix` in `20261103-144857-claude-haiku`, `agents/claude-haiku/20261104-010000` in `20261104-001421-claude-haiku`, `agents/claude-haiku/20261104-110000` in `20261104-094433-claude-haiku`, `agents/claude-haiku/20261104-r5` in `20261104-103821-claude-haiku`, `agents/claude-haiku/r4-follow-up` in `20261104-121211-claude-haiku`) and implemented tasks already owned by `gemini-1` or `codex-1`.
3. **Outdated host installation (`~/.local/bin/agent` from `Nov 3 18:26`):**
   - Commit [`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f) (`A team member's gh pr create is refused; its coordinator opens the round's PR`) was merged to `main` on `2026-11-04T06:11:14Z`, but `~/.local/bin/agent` on the host has not been reinstalled since `Nov 3 18:26`, so runs on `Nov 4` still executed an older `bin/agent` binary without `AGENT_GH_COORDINATOR`.

---

## 2. What fixes team synchronization (`claude-haiku`)

1. **Disable Claude Code `ToolSearch` deferral in `run_claude` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L3393-L3406)):**
   - Set `"env": {"ENABLE_TOOL_SEARCH": "false"}` in `claude-settings.json` and pass `ENABLE_TOOL_SEARCH=false` in `run_claude`'s environment.
   - In Claude Code `2.1.x`, `ENABLE_TOOL_SEARCH=false` forces `"standard"` tool mode (`rIe() === "standard"`), loading all `mcp__agentbus__*` tool schemas directly into `claude-haiku`'s first-turn tool list without `ToolSearch` or `tool_reference` indirection.
2. **Make `agentbus` CLI errors self-correcting when an agent invokes `agentbus` in Bash ([`agentbus/main.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/agentbus/main.go#L1542-L1544), [`agentbus/team.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/agentbus/team.go#L778-L781)):**
   - `CliUnknownSubcommand` and `CmdTeamGetNoSuchTeam` now explicitly tell the caller that inside an agent run, `team_status`, `read_messages`, `claim_lead`, `board_list`, `send_message`, etc. are MCP tools (`mcp__agentbus__<name>`), not shell commands, and that `team-get` requires `--team <id>`.
3. **Clarify MCP tool invocation and single-branch discipline in [`prompts/team.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/prompts/team.md#L27-L31):**
   - Explicitly state in [`prompts/team.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/prompts/team.md#L27-L31) and [`prompts/team.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/prompts/team.md#L117-L125) that `agentbus` tools are MCP tool calls (never Bash commands) and that members never create or switch branches (`git checkout -b`, `git switch -c`).

---

## 3. Why `claude-haiku` starts `round + 1` before the current round finishes, and how to fix it

Three separate bugs combined to cause `claude-haiku` to start `round = current_round + 1` (`Round 2` in `20261103-001628-claude-haiku`, and `Round 5` while `Round 4` was still in fix passes in `20261104-094433-claude-haiku`, `20261104-103821-claude-haiku`, and `20261104-121211-claude-haiku`):

1. **Bug 3A — A team member (`claude-haiku`) merged the current round's PR ([#53](https://github.com/mateusz-szczyrzyca/team-agents/pull/53)) itself via `gh pr ready` + `gh pr merge` during a fix pass:**
   - On `2026-11-04` at `08:44:35Z` (`09:44:35` local), team `20261104-094433-team` was resumed for Fix Pass `1` on Round `4`'s PR [#53](https://github.com/mateusz-szczyrzyca/team-agents/pull/53) because reviewer `inv-astra` (`inv145`) reported `2` major findings (`needs-fixes`), while `inv-gemini-argon` (`inv146`) reported `ready-to-merge`.
   - In `20261104-094433-claude-haiku`, `claude-haiku` ran `gh pr view 53 --json reviews | jq '.reviews[-1]'` (inspecting only the last review in the array, which was `Ready to merge`) and at `09:46:30–09:46:31` executed `gh pr ready 53` and `gh pr merge 53 --squash`!
   - Because `agent_gh` in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L188-L193) did not block `gh pr merge` or `gh pr ready`, PR [#53](https://github.com/mateusz-szczyrzyca/team-agents/pull/53) was merged into `main` at `08:46:34Z` while coordinator `gemini-1` and `codex-1` were still fixing `inv-astra`'s Round `4` findings.
   - Immediately after merging PR [#53](https://github.com/mateusz-szczyrzyca/team-agents/pull/53) itself, `claude-haiku` saw PR [#53](https://github.com/mateusz-szczyrzyca/team-agents/pull/53) was `MERGED`, fetched `origin/main`, created `agents/claude-haiku/20261104-110000` (and in `20261104-103821-claude-haiku`, `agents/claude-haiku/20261104-r5`), and began implementing Round `5` (`round + 1`).
2. **Bug 3B — Non-coordinator `checkpoint` calls in `agentbus` overwrote the shared team checkpoint (`checkpoints/team-<id>.md`) with `round + 1` instructions:**
   - In `20261104-103821-claude-haiku` (Fix Pass `2` on Round `4`), while `gemini-1` was the elected coordinator (`lead`), member `claude-haiku` called `checkpoint` with summary `"Round 5 foundation laid"` and next steps `"Round 5 implementation ready to proceed on B10-B15"`.
   - Because [`agentbus/collab.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/agentbus/collab.go#L889-L937) wrote every team member's `checkpoint` to `checkpoints/team-<team>.md`, when `20261104-121211-team` resumed with `--same-round` to finish the remaining Round `4` follow-up (`ac2b46f`, later merged as [#57](https://github.com/mateusz-szczyrzyca/team-agents/pull/57)), `build_prompt` injected `claude-haiku`'s Round `5` checkpoint into every member's prompt (`continue from the last checkpoint`), causing `claude-haiku` to open both [#55](https://github.com/mateusz-szczyrzyca/team-agents/pull/55) (`agents/claude-haiku/r4-follow-up`) and [#56](https://github.com/mateusz-szczyrzyca/team-agents/pull/56) (`agents/claude-haiku/20261104-r5`, Round `5`).
3. **Bug 3C — Non-coordinator `STATUS: ROUND_DONE N/M` wrote `WORK_ROUND` to the member's `meta.env` and could override the coordinator's round in `resume_run`:**
   - In [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L5256-L5265), any member outputting `STATUS: ROUND_DONE N/M` had `WORK_ROUND` and `WORK_ROUNDS` written to its `meta.env`, and `resume_run` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L5775-L5777)) simply took the last task in `olds` with `WORK_ROUND` set rather than preferring the coordinator's task.

**Fixes implemented for Point 3:**
- `agent_gh` in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L186-L190) now refuses `gh pr merge` from any agent (`github_pr_merge_refused`); only the human or `auto_merge` in `--autopilot` (`AGENT_AUTOPILOT_MERGE=1` at [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L1671-L1677)) can merge a PR.
- `checkpoint` in [`agentbus/collab.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/agentbus/collab.go#L888-L953) now isolates non-coordinator checkpoints (`t != nil && t.Lead != "" && t.Lead != s.me`) to `checkpoints/task-<task>.md` without appending to `checkpoints/team-<id>.md` or broadcasting `EventCheckpoint` to `"all"`.
- `cmd__runner` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L5281-L5295)) only records `WORK_ROUND` / `WORK_ROUNDS` when the caller is not a subordinate member of an elected coordinator, and `resume_run` ([`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L5802-L5809)) prefers the coordinator's `WORK_ROUND`.

---

## 4. Enforcing `1` PR per round and coordinator-only branch merging

Two defects caused `claude-haiku` to open extra PRs ([#40](https://github.com/mateusz-szczyrzyca/team-agents/pull/40), [#46](https://github.com/mateusz-szczyrzyca/team-agents/pull/46), [#54](https://github.com/mateusz-szczyrzyca/team-agents/pull/54), [#55](https://github.com/mateusz-szczyrzyca/team-agents/pull/55), [#56](https://github.com/mateusz-szczyrzyca/team-agents/pull/56)) and to merge and push directly to the coordinator's PR branch (`git push origin HEAD:agents/gemini-1/20261103-180957` in `20261103-195812-claude-haiku`, `20261103-204345-claude-haiku`, `20261104-091040-claude-haiku`):

1. **`round_pr_note` in [`bin/agent`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L1783-L1784) (introduced in [`2b2a51f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2b2a51f3b5eaff209322709e612c46290586889e)) literally instructed every team member to merge the PR branch and push directly to the coordinator's PR branch:**
   - The prompt text injected on `--same-round` resume said:
     `Every commit of this round goes there: a member — or a new coordinator — merges origin/<branch> into its own branch and pushes it there (git push origin HEAD:<branch>).`
   - `claude-haiku` followed that sentence literally and pushed directly to `agents/gemini-1/...`.
   - **Fix:** [`round_pr_note`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L1807-L1809) now states that **a member commits and pushes only to its own branch**, and **the coordinator merges the members' branches into the round's PR branch** (while a new coordinator taking over fetches `origin/<branch>` and pushes to `<branch>`).
2. **`AGENT_GH_COORDINATOR` in [`agents_gh_env`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L1508-L1509) was only evaluated once before the CLI started, and `agent_gh` only checked `gh pr create`:**
   - In Run `1` (`mode=new`) of any team, `agents_gh_env` runs before any agent has called `claim_lead`, so `team_lead "$team"` is empty (`""`) and `AGENT_GH_COORDINATOR` remained `""` for the entire first run of every member even after `gemini-1` or `codex-1` claimed lead seconds later.
   - Furthermore, `agent_gh` did not block a member's `gh pr ready`, `gh pr close`, `gh pr reopen`, or `gh pr edit`.
   - **Fix:** [`agents_gh_env`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L1532-L1534) now exports `AGENT_GH_TASK=${RUNNER_ID:-}`, and [`agent_gh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L195-L216) dynamically resolves the team's current `lead` from `bus/state.json` when `AGENT_GH_COORDINATOR` was empty at run start, refusing `gh pr create`, `gh pr ready`, `gh pr close`, `gh pr reopen`, and `gh pr edit` for any non-coordinator team member.

---

## 5. Summary of what this PR fixes and how

| Problem observed in `gemini-1,codex-1,claude-haiku` runs | Root cause | Fix in [`f002bee`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f002bee193c6828e747f553bd7b9c4f23894250b) |
|---|---|---|
| `claude-haiku` made `0` MCP calls in `12/30` runs and ran `mcp__agentbus__*` / `agentbus` in Bash | Claude Code `2.1.x` enables `ToolSearch` (`"tst"`) by default, hiding `agentbus` MCP tools; `agentbus team-get <id>` without `--team` printed `no team` | Set `ENABLE_TOOL_SEARCH=false` in [`run_claude`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L3393-L3406); clarify CLI errors in [`agentbus/main.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/agentbus/main.go#L1543) and [`agentbus/team.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/agentbus/team.go#L780); reinforce MCP tool rules in [`prompts/team.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/prompts/team.md#L27-L31) |
| `claude-haiku` merged PR [#53](https://github.com/mateusz-szczyrzyca/team-agents/pull/53) mid-fix-pass via `gh pr ready 53 && gh pr merge 53 --squash` and started Round `5` | [`agent_gh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L188-L193) did not block `gh pr merge` or `gh pr ready` | Refuse `gh pr merge` for all agents (`github_pr_merge_refused`, allowing only `AGENT_AUTOPILOT_MERGE=1` in [`auto_merge`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L1671-L1677)) and refuse `gh pr ready`, `close`, `reopen`, `edit` for non-coordinator members in [`agent_gh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L186-L216) |
| `claude-haiku` opened duplicate PRs in Run `1` ([#40](https://github.com/mateusz-szczyrzyca/team-agents/pull/40), [#46](https://github.com/mateusz-szczyrzyca/team-agents/pull/46), [#54](https://github.com/mateusz-szczyrzyca/team-agents/pull/54), [#55](https://github.com/mateusz-szczyrzyca/team-agents/pull/55), [#56](https://github.com/mateusz-szczyrzyca/team-agents/pull/56)) | `AGENT_GH_COORDINATOR` was evaluated before `claim_lead` in Run `1` (and `~/.local/bin/agent` on host pre-dated [`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f)) | Export `AGENT_GH_TASK` in [`agents_gh_env`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L1532-L1534) and resolve the team's `lead` dynamically from `bus/state.json` inside [`agent_gh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L195-L205) |
| `claude-haiku` saved a Round `5` team `checkpoint` during Round `4` Fix Pass `2` | [`checkpoint`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/agentbus/collab.go#L889-L937) wrote every member's checkpoint to `checkpoints/team-<id>.md` | Isolate non-coordinator member checkpoints to `checkpoints/task-<task>.md` in [`agentbus/collab.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/agentbus/collab.go#L888-L953) |
| `claude-haiku` merged and pushed directly to the coordinator's PR branch | [`round_pr_note`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L1783-L1784) told members to merge `origin/<branch>` and run `git push origin HEAD:<branch>` | Update [`round_pr_note`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L1807-L1809) and [`prompts/team.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/prompts/team.md#L117-L125) so members push only to their own branch and only the coordinator merges member branches into the round's PR |
| Member `STATUS: ROUND_DONE N/M` could override the coordinator's `WORK_ROUND` | [`cmd__runner`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L5256-L5265) and [`resume_run`](https://github.com/mateusz-szczyrzyca/team-agents/blob/c7ad84bf90e13180e69eec787b38c5326ab95a3f/bin/agent#L5775-L5777) accepted `WORK_ROUND` from any member | Record `WORK_ROUND` in [`cmd__runner`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L5281-L5295) only for the coordinator (or when no coordinator is set) and prefer the coordinator's task in [`resume_run`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/bin/agent#L5802-L5809) |

> **Host deployment note:** Once the merged PR from Phase 2 is merged to `main`, `./install.sh` must be run on the host (when no agents are running) so `~/.local/bin/agent`, `~/.local/bin/agentbus`, and `~/agents/prompts/team.md` are updated from the `Nov 3 18:26` installation.

---

## Checks

- `make gate` (`7/7` steps passed: `check-generated`, `lint-shell`, `lint-go`, `lint-docs`, `unit-go`, `unit-bats` `557/557`, `harness` `20/20`, `0` lint findings)
- New tests added in [`agentbus/team_sync_test.go`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/agentbus/team_sync_test.go#L1-L51) and [`tests/unit/team_member_sync.bats`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/tests/unit/team_member_sync.bats#L1-L94) (registered in [`tests/agent-go-map.tsv`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/tests/agent-go-map.tsv#L553-L558)); `0` existing tests modified.

## Skipped tests

- None.

## Not done

- Reinstalling `~/.local/bin/agent`, `~/.local/bin/agentbus`, and `~/agents/prompts/` on the live host (forbidden during an agent run by [`AGENTS.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f002bee193c6828e747f553bd7b9c4f23894250b/AGENTS.md#L38-L41) Rule `3`).
