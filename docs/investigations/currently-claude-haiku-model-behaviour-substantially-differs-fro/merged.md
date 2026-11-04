# Why `claude-haiku` works alone and runs ahead to `round + 1` — merged investigation report

Merged by `inv-gemini-argon` (`gemini`, model `gemini-4-argon-high`) from the Phase `1` results recorded in `/home/dev/agents/tasks/20261104-130412-investigation-team/phase1.json`.
Base commit: [`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f).

## Phase 1 results summary

| Investigator | Recorded Phase 1 result | What was taken | What was dropped and why |
|---|---|---|---|
| `inv-opus` | [`990b4a3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/990b4a3b1f2017e36624d56a81e7504928f3cf00) ($\color{red}{\textbf{RED}}$ [`3aad565`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3aad5657f3fb1087df47561b35be0c9e421899c4), $\color{green}{\textbf{GREEN}}$ [`3c872ce`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3c872ce7a6a3467fabaebcf5dee2c7f31befe254)), report [`inv-opus.md`](inv-opus.md) | All `3` commits unchanged: `CLAUDE_TOOL_SEARCH` (`false` by default), `gh_team_lead` dynamic coordinator lookup via `agentbus team-get`, member `gh pr create`/`merge`/`ready`/`close` refusal, `round_pr_note` coordinator-only merging, `round_text_restate` deduplicating stacked `--same-round` headings/PR notes, `round_pr_merged` follow-up heading, prompt updates, and `tests/unit/member_in_team.bats` (`7` tests) | None — taken in full as the branch foundation |
| `inv-gemini-argon` | [`2739d72`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2739d72c6d9ba0700edc9bd314bf11b80eaee41b) ($\color{red}{\textbf{RED}}$ [`60cd9a1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/60cd9a16d2b69983198e058f6ebbd8c0977a8ade), $\color{green}{\textbf{GREEN}}$ [`f002bee`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f002bee193c6828e747f553bd7b9c4f23894250b)), report [`inv-gemini-argon.md`](inv-gemini-argon.md) | Ported in $\color{red}{\textbf{RED}}$ [`c62d6df`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c62d6dffe33dff2472089a7d3620532aa8919c9b) and $\color{green}{\textbf{GREEN}}$ [`bbad7ca`](https://github.com/mateusz-szczyrzyca/team-agents/commit/bbad7ca1d08ef22c97c1f523385db697d04cfeed): `agentbus` member `checkpoint` isolation (`agentbus/collab.go`, `agentbus/team_sync_test.go`), self-correcting `agentbus` CLI error hints (`agentbus/main.go`, `agentbus/team.go`), member `gh pr reopen`/`edit` refusal, and coordinator-only `WORK_ROUND` recording on `STATUS: ROUND_DONE` (`cmd__runner`, `resume_run`) | Hardcoded `ENABLE_TOOL_SEARCH=false` (superseded by `inv-opus`'s configurable `CLAUDE_TOOL_SEARCH`), direct `state.json` read via `AGENT_GH_TASK` (superseded by `inv-opus`'s `gh_team_lead` via `agentbus team-get`), `AGENT_AUTOPILOT_MERGE=1` (superseded by `inv-opus`'s member-scoped `gh pr merge` guard), and `4` duplicate Bats tests in `team_member_sync.bats` (covered by `member_in_team.bats`) |

---

## Consolidated answers to points 1–5

### 1. Why `claude-haiku` acts alone and fails to synchronize with the team

Both investigations inspected all historical `claude-haiku` team runs (`31` task directories in `gemini-1,codex-1,claude-haiku` teams) and converged on the same causes:

- **Deferred `agentbus` MCP tools in Claude Code (`ENABLE_TOOL_SEARCH`):** Claude Code `2.1.289` defaults to `"tst"` (Tool Search Tool) mode when `ENABLE_TOOL_SEARCH` is unset, deferring MCP tool schemas behind `ToolSearch`. In `11` of `31` team runs (`12` of `30` completed runs), `claude-haiku` made `0` `agentbus` MCP tool calls: in `9` runs it called `ToolSearch`, received `tool_reference` objects, and then invoked `mcp__agentbus__team_status` or `agentbus team-get` as **Bash shell commands**. Because `agentbus team-get` without `--team` exited `3` with `agentbus: team-get: no team`, `claude-haiku` concluded no team existed and worked solo for the rest of the run.
- **Oversized prompt with stacked contradictory `--same-round` headings:** In `2` later runs (`20261104-121211-claude-haiku` and `20261104-122828-claude-haiku`), `prompt-1.md` grew past `111 KB` (`> AGENT_PROMPT_ARG_MAX=100000`) because every `--same-round` resume appended a new `## Round N of M continues: its PR is not merged` block and a new `## The round's PR` block (`15` headings and `15` PR notes stacked in `task.md`). Reading `.agent-prompt.md` in partial slices, `claude-haiku` saw stale earlier headings and never called `ToolSearch`.
- **Missing member guards and outdated host installation:** `agent_gh` only blocked `gh pr create` when `AGENT_GH_COORDINATOR` was known at run start (which is empty in Run `1` before `claim_lead`), did not block `gh pr merge`/`ready`/`close`/`reopen`/`edit`, and `~/.local/bin/agent` on the host (`Nov 3 18:26`) had not been reinstalled after [`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f).

### 2. What fixes team synchronization

- **Load all tools up front in Claude Code (`CLAUDE_TOOL_SEARCH=false`):** `run_claude` now sets `env.ENABLE_TOOL_SEARCH` in `claude-settings.json` from `CLAUDE_TOOL_SEARCH` (default `false`), disabling `ToolSearch` deferral so `claude-haiku` receives all `mcp__agentbus__*` tool schemas on turn `1`.
- **Self-correcting `agentbus` CLI errors (`agentbus/main.go`, `agentbus/team.go`):** `CliUnknownSubcommand` and `CmdTeamGetNoSuchTeam` now explicitly tell the caller that inside an agent run, `team_status`, `read_messages`, `claim_lead`, `board_list`, `send_message`, etc. are MCP tools (`mcp__agentbus__<name>`), not shell commands, and that `team-get` requires `--team <id>`.
- **Prompt protocol (`prompts/team.md`):** Explicitly states that `agentbus` tools are CLI tool calls (never shell commands) and that if they are unavailable, the agent must end with `STATUS: FAILED` rather than working alone.

### 3. Why `claude-haiku` starts `round + 1` before the current round finishes, and how it is fixed

Four bugs chained together in Rounds `3`–`5`:

1. **A member merged the round's PR (`#53`) mid-fix-pass:** In `20261104-094433-claude-haiku` (Fix Pass `1` on Round `4`'s PR [#53](https://github.com/mateusz-szczyrzyca/team-agents/pull/53)), `claude-haiku` inspected only the last review (`jq '.reviews[-1]'`, which was `Ready to merge`, ignoring `inv-astra`'s `Needs changes` review) and ran `gh pr ready 53` and `gh pr merge 53 --squash`. Seeing PR [#53](https://github.com/mateusz-szczyrzyca/team-agents/pull/53) merged, it immediately branched from `origin/main` and started Round `5`.
   - **Fix:** `agent_gh` now refuses `gh pr create`, `merge`, `ready`, `close`, `reopen`, and `edit` for any non-coordinator team member, querying the bus dynamically via `gh_team_lead` (`agentbus team-get`).
2. **A member's `checkpoint` overwrote the shared team checkpoint (`checkpoints/team-<id>.md`) with Round `5` plans:** In `20261104-103821-claude-haiku` (Fix Pass `2` on Round `4`), `claude-haiku` called `checkpoint` with summary `"Round 5 configuration loading initialized..."`. Because `agentbus` wrote every member's checkpoint to `checkpoints/team-<id>.md`, the next `--same-round` resume injected `claude-haiku`'s Round `5` checkpoint as the team's "last checkpoint" to continue from.
   - **Fix:** `checkpoint` in `agentbus/collab.go` now isolates non-coordinator checkpoints (`t != nil && t.Lead != "" && t.Lead != s.me`) to `checkpoints/task-<task>.md` without overwriting `checkpoints/team-<id>.md` or broadcasting `EventCheckpoint`, and `prompts/resume-checkpoint.md` instructs teams to follow the coordinator's checkpoint.
3. **`--same-round` stacked stale headings and reported a merged PR as "not merged":**
   - **Fix:** `round_text_restate` drops older `--same-round` headings and PR notes so only the newest remains, and `round_pr_merged` checks whether the round's PR is already merged — if so, it states that the run is a follow-up for the current round and that `Round N+1 has not started: do none of its work`.
4. **Non-coordinator `STATUS: ROUND_DONE N/M` could override `WORK_ROUND`:**
   - **Fix:** `cmd__runner` only records `WORK_ROUND` / `WORK_ROUNDS` when the caller is not a subordinate member of an elected coordinator, and `resume_run` prefers the coordinator's `WORK_ROUND`.

### 4. Enforcing `1` PR per round and coordinator-only branch merging

- **Root cause in `round_pr_note`:** Previously, `round_pr_note` in `bin/agent` wrote into every resumed round:
  `Every commit of this round goes there: a member — or a new coordinator — merges origin/<branch> into its own branch and pushes it there (git push origin HEAD:<branch>).`
  `claude-haiku` followed that instruction literally and pushed directly to `agents/gemini-1/...`.
- **Fix:** `round_pr_note` and `prompts/team.md` now instruct members to push only their own branch and notify the coordinator, while **only the coordinator** merges members' branches into its branch and pushes to the round's single PR. `agent_gh` dynamically queries `gh_team_lead` on every PR mutation command (`create`, `merge`, `ready`, `close`, `reopen`, `edit`), refusing any member (including in Run `1` before `claim_lead` is called).

### 5. Verification

- `env -u AGENT_GITHUB_NAME_COLORS -u AGENT_GITHUB_COLORS make gate` passes all `7` steps (`check-generated`, `lint-shell`, `lint-go`, `lint-docs`, `unit-go`, `unit-bats` `561/561`, `harness` `20/20`, `0` lint findings).
