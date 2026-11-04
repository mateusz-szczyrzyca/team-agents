# Why `claude-haiku` works alone and runs ahead to the next round — inv-opus

Examined base: [`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f).
Branch: `agents/inv-opus/20261104-130420`. Fix commits:
$\color{red}{\textbf{RED}}$ [`3aad565`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3aad5657f3fb1087df47561b35be0c9e421899c4) and
$\color{green}{\textbf{GREEN}}$ [`3c872ce`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3c872ce7a6a3467fabaebcf5dee2c7f31befe254).

Evidence: every `claude-haiku` task of this installation, read-only (`~/agents/tasks/*-claude-haiku`: `meta.env`,
`task.md`, `prompt-*.md`, the CLI's event streams `out-*.jsonl`), the bus's teams (`~/agents/bus/state.json`) and
the PRs on GitHub. The team in question: `gemini-1` coordinating, `codex-1` and `claude-haiku` members (on the bus,
`claude-haiku` never was the coordinator). `claude-haiku` is a **normal** model in `NORMAL_AGENTS`, so it gets the
same team prompt as `gemini-1` and `codex-1`.

## Summary

- `claude-haiku` had **no team bus** in **`11` of its `31`** team runs (no agentbus tool called once). In `9` of
  them, Claude Code had **deferred the agentbus tools behind its tool search**. Haiku loaded them with
  `ToolSearch` and then ran them **in the shell** (`mcp__agentbus__team_status`, `agentbus team-status`,
  `agentbus team-get` → "no team"). After that it worked as a solo agent. The other `2` runs got a `111 KB`
  prompt as a file and went straight to work.
- Nothing stopped a member that went its own way. The agents' `gh` refused only `gh pr create`, and only when the
  coordinator was known at the run's start. The round's own text told every member to push to the coordinator's
  branch.
- **Round + 1** came from one chain: the `--same-round` text said a merged PR was "not merged". `claude-haiku`
  merged PR [#53](https://github.com/mateusz-szczyrzyca/team-agents/pull/53) itself and started round 5 alone.
  Its checkpoint ("Round 5 initialized") became the team's "last checkpoint", and every next prompt says to
  continue from that.
- The PR on this branch fixes:
  - tool search off → the agentbus tools are loaded up front;
  - a member's `gh pr merge`/`ready`/`close` refused, and the coordinator asked of the bus when the command runs;
  - the round's text: the coordinator alone merges members' branches, one current heading per round, and the
    heading says when the round's PR is merged already;
  - the prompts: a member never starts a round.

## 1. Why haiku acts alone and does not sync with its team

### 1a. The agentbus tools are deferred, and haiku runs them in the shell (root cause)

Claude Code (`2.1.289` on this host) defers MCP tools by default: their names are listed, and the model must load
them with `ToolSearch` before it calls them. The bundled code reads `ENABLE_TOOL_SEARCH`. Unset means "tool search
on". `false`/`0`/`off` means "standard": every tool is given up front. Haiku 4.5 is not on Claude Code's list of
models without tool search, so it gets the deferred tools.

Every `claude-haiku` team run, by its agentbus calls:

| runs | agentbus tool calls | what happened |
|---|---|---|
| `20` | `2`–`57` | `ToolSearch`, then called them as tools: worked with the team |
| `9` | `0` | `ToolSearch` → tool references → it called them **in the shell**, then worked alone |
| `2` | `0` | the `111 KB` prompt as a file (`.agent-prompt.md`); no attempt to reach the team at all |

The `9` runs: `20261102-175849`, `20261103-075558`, `20261103-082115`, `20261103-085533`, `20261103-101715`,
`20261103-143807`, `20261103-184910`, `20261103-195812`, `20261103-221740`. Two of them, as the event stream shows
them:

```text
20261103-101715 (round 2, a new team)
T[ToolSearch]: select:mcp__agentbus__team_status,mcp__agentbus__read_messages,…,mcp__agentbus__claim_lead
R: (tool references)
T[Bash]: agentbus team-get
R: Exit code 3 ⏎ agentbus: team-get: no team
… "Since round 1 is merged, I need to implement R02" … gh pr create --draft … "R02: Go rewrite agent module…" (#46)

20261103-221740 (round 3 → 4)
T[ToolSearch]: select:mcp__agentbus__team_status,…
T[Bash]: agentbus team-get
R: Exit code 3 ⏎ agentbus: team-get: no team
A: Starting round 4 after round 3 merge. … gh pr create --draft … "R04: Staged lanes infrastructure…" (#54)
```

`agentbus team-get` without `--team` answers "no team". To a model that just failed to reach its team, this reads
as "you are not in a team".

### 1b. The prompt is long and contradicts itself

The team prompt of the Go rewrite's later rounds is `111,890` bytes (`1,297` lines; above `AGENT_PROMPT_ARG_MAX` =
`100,000`, so it is delivered as a file). Most of it is the task text. Every `--same-round` resume appended a new
round heading and a new "The round's PR" note and kept the old ones: run `20261104-121211` had `15` headings
("Round 3 of 56 continues: its PR is not merged" with [#52](https://github.com/mateusz-szczyrzyca/team-agents/pull/52)
merged, then "Round 4 of 56 continues: its PR is not merged" with
[#53](https://github.com/mateusz-szczyrzyca/team-agents/pull/53) merged) and `15` PR notes naming `#52` and
`#53`. A small model reading this file in parts sees only a part of it, and the true part is the last one.

### 1c. Nothing enforced the team's rules

- The agents' `gh` refused a member's `gh pr create` only ([`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f)),
  and only when the bus named a coordinator **when the run started**. A new team's first runs start before anyone
  has called `claim_lead`: run `20261103-101715` (RUN=1) opened
  [#46](https://github.com/mateusz-szczyrzyca/team-agents/pull/46) that way.
- That commit is **not installed**: `~/.local/bin/agent` has no `AGENT_GH_COORDINATOR`. `claude-haiku`'s
  [#55](https://github.com/mateusz-szczyrzyca/team-agents/pull/55) and
  [#56](https://github.com/mateusz-szczyrzyca/team-agents/pull/56) (run `20261104-121211`, after the commit) went
  through.
- `gh pr merge`, `gh pr ready`, `gh pr close` were open to anyone (see 3).
- The script's own round text told every member to push to the coordinator's branch (see 4).

`claude-haiku`'s own PRs beside the round's:
[#36](https://github.com/mateusz-szczyrzyca/team-agents/pull/36),
[#46](https://github.com/mateusz-szczyrzyca/team-agents/pull/46),
[#54](https://github.com/mateusz-szczyrzyca/team-agents/pull/54),
[#55](https://github.com/mateusz-szczyrzyca/team-agents/pull/55) ("R04 follow-up"),
[#56](https://github.com/mateusz-szczyrzyca/team-agents/pull/56) ("R05"). All are closed now.

## 2. What can be done

| # | change | where | in this PR |
|---|---|---|---|
| 1 | Claude Code's tool search off: the agentbus tools are in the tool list from the start, nothing to load | `run_claude`, `CLAUDE_TOOL_SEARCH` (default `false`) | yes |
| 2 | a member's `gh pr merge`, `ready`, `close` refused like its `create` | the agents' `gh` | yes |
| 3 | who coordinates is asked of the bus when the PR command runs (new team, take-over during a run) | the agents' `gh`, `agents_gh_env` (`AGENT_GH_TEAM`, `AGENT_GH_SELF`) | yes |
| 4 | the round's text: a member pushes its own branch; only the coordinator merges members' branches | `round_pr_note` | yes |
| 5 | a `--same-round` resume keeps only the newest round heading and PR note | `round_text_restate` | yes |
| 6 | the heading says when the round's PR is merged, and that the next round has not started | `resume_run`, `round_pr_merged` | yes |
| 7 | prompts: the agentbus tools are tools, not shell commands; no bus → `STATUS: FAILED`, never alone | `prompts/team.md` | yes |
| 8 | prompts: a member never merges, never pushes to others' branches, never starts a round; the team's next steps are the coordinator's checkpoint | `prompts/team.md`, `prompts/resume-checkpoint.md`, `prompts/tdd-rounds.md` | yes |
| 9 | install the current `bin/agent`, so [`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f) and this PR take effect | `install.sh` — the human | no (rule 3) |

## 3. Why haiku starts round + 1 before the round is finished

Not one bug. A chain, in the order it happened in the Go rewrite's rounds 3–5:

1. **Rounds 3 and 4 ran on the same branches.** At 22:17 on 2026-11-03 the team was resumed with `--same-round`
   and the addition "Round 3 is merged, proceed to next rounds of this task". The script wrote above it "Round 3 of
   56 continues: its PR is not merged" (without checking) and "This round's PR is #52". Round 4's PR
   [#53](https://github.com/mateusz-szczyrzyca/team-agents/pull/53) was then opened from round 3's branch
   `agents/gemini-1/20261103-180957`. The text of the next resumes says "Round 4 continues" after "Round 3
   continues", and the human's additions ("Then proceed to next rounds of this task") stay in every later prompt.
2. **A member merged the round's PR.** In run `20261104-094433`, `claude-haiku` read inv-gemini-argon's
   "Ready to merge" review. `gh pr edit` failed (token scope), so it decided: "I'll proceed to merge the PR and move
   to the next round". It ran `gh pr merge 53 --squash` (merged at 08:46 UTC), branched
   `agents/claude-haiku/20261104-110000` from `origin/main`, and told `gemini-1` "Ready to proceed with Round 5".
3. **A member planned the next round on the team's board and checkpoint.** In run `20261104-103821` (the round 4
   fix pass), it asked the coordinator. With no answer within its `wait_for_message`, it went on "per checkpoint
   direction". It branched `agents/claude-haiku/20261104-r5` from `main`, added `7` board items "R05 B1"–"R05 B7"
   (`B10`–`B16`), committed R05's RED and GREEN, and saved the checkpoint "Round 5 configuration loading
   initialized and ready for team work".
4. **The resume prompt made that the team's direction.** The resume text says "continue from the 'next steps' of
   the last checkpoint", and the last one was `claude-haiku`'s. The board, R05 items included, is carried into the
   resumed team. Run `20261104-121211` continued R05 and opened
   [#56](https://github.com/mateusz-szczyrzyca/team-agents/pull/56) "R05" and
   [#55](https://github.com/mateusz-szczyrzyca/team-agents/pull/55) "R04 follow-up".

What stops it:

- the merge in step 2 is refused (change 2 and 3 above);
- a `--same-round` heading checks the round's PR and says the truth. Open: "Round N of M continues: its PR is not
  merged … Round N+1 has not started: do none of its work". Merged: "… its PR #X is merged: do only the
  follow-up … Round N+1 has not started" (change 6). Only the newest heading and PR note stay (change 5);
- the prompts: a member never starts a round, never adds next-round items or checkpoints; "go on with the next
  rounds" in an addition means after the merge, when the script starts the round (change 8);
- for the human: after a round's PR is merged, resume **without** `--same-round` (`agent run --resume <team>`).
  The next round then starts on fresh branches with "Round N+1 of M" and the old round-only text left out. Use
  `--same-round` only while the round's PR is open (review fixes) or for an explicit follow-up of a merged round.

## 4. One PR per round; merging the members' branches is the coordinator's job

What told members otherwise was the script itself. `round_pr_note` wrote into every resumed round:

```text
This round's PR is #53 (branch agents/gemini-1/20261103-180957). Every commit of this round goes there: a member —
or a new coordinator — merges origin/agents/gemini-1/20261103-180957 into its own branch and pushes it there
(git push origin HEAD:agents/gemini-1/20261103-180957).
```

`claude-haiku` did exactly that:
- `20261103-085533`: `git push origin fix-m1-merge:agents/gemini-1/20261102-221428`;
- `20261103-184910`: `git push origin pr-52-branch:agents/gemini-1/20261103-180957`;
- `20261103-195812`, `20261104-091040`: `git push origin HEAD:agents/gemini-1/20261103-180957`;
- in `20261103-160558` it even pushed `codex-1`'s branch from `codex-1`'s worktree.

Now the note says:

```text
This round's PR is #53 (branch agents/gemini-1/20261103-180957). Every commit of this round goes there, through the
coordinator: a member pushes its own branch and tells the coordinator what is on it (a board item or a message);
the coordinator merges the members' branches into its own and pushes it there
(git push origin HEAD:agents/gemini-1/20261103-180957) — a new coordinator first merges
origin/agents/gemini-1/20261103-180957 into its branch. A member never pushes to the coordinator's branch, and never
opens, merges or closes a PR. Never open another PR for this round: the checks send a round with two open PRs back.
```

The agents' `gh` refuses a member's `gh pr create`, `merge`, `ready` and `close`. It asks the bus who coordinates
when the command runs. A team with no coordinator yet means every member is refused. The coordinator itself, also
one that took over during the run, passes.

## 5. What the PR on this branch fixes, and how

| problem | fix | test (`tests/unit/member_in_team.bats`) |
|---|---|---|
| haiku cannot reach the bus: deferred tools run in the shell | `CLAUDE_TOOL_SEARCH` (default `false`) → `env.ENABLE_TOOL_SEARCH` in each run's Claude Code settings; `true`/`auto`/`auto:N` passed on, `default` = Claude Code's own | Claude Code gets the agentbus tools up front … |
| a member merges, readies or closes the round's PR | the agents' `gh` refuses them as it refuses `create` (`github_member_pr_refused`, `action=`) | a team member's gh pr merge, ready and close are refused … |
| a new team's first runs, or a take-over, are not guarded right | `AGENT_GH_TEAM`/`AGENT_GH_SELF` exported for a team task; `gh_team_lead` asks `agentbus team-get` at the call; `github_coordinator_unknown` when the bus does not answer (then the start-time coordinator counts) | the agents' gh asks the bus who coordinates …; a team task's gh knows its team and agent … |
| the round text tells members to push to the coordinator's branch | `round_pr_note`: members push their own branch; the coordinator merges | a round resumed tells a member to push its own branch … |
| `--same-round` stacks every old heading and PR note | `round_text_restate`: drops the round-only blocks a same-round resume writes anew; review findings and the human's additions stay | a round continued again has one heading and one PR note … |
| "its PR is not merged" when it is | `round_pr_merged` checks the round's PR by number; merged → a follow-up heading and PR note; both say the next round has not started | a round whose PR is merged continues as its follow-up … |
| prompts let a member wander | `team.md` (start: tools not shell, no bus → `FAILED`; member rule 5), `resume-checkpoint.md` (the coordinator's checkpoint), `tdd-rounds.md` (no next round, also when told "proceed to next rounds") | prompt text: Test-First not possible |

On the real task text of run `20261104-121211`, `round_text_restate` removes the `30` stale blocks:
`78,456` → `68,856` bytes, with every review and every addition of the human kept.

### Checks

```bash
bats tests/unit/member_in_team.bats    # 7/7
make gate                              # all 7 steps passed, 559/559 bats, 0 lint findings
```

### Compatibility

- New configuration variable `CLAUDE_TOOL_SEARCH` (default `false`): Claude runs now get every tool up front.
  This adds the agentbus tool schemas (`18`–`34` tools, about `7.5 KB` for a solo run) to each request's cached
  prefix. `default` restores Claude Code's own behaviour.
- The agents' `gh`: new refusals, for a team member only. The script's own PR commands run in their own units
  without these variables, and the runner clears them when its CLI ends. A solo task, an investigator and a
  coordinator are not affected. Log event `github_member_pr_refused` gains `action`; new event
  `github_coordinator_unknown`.
- Task text: `--same-round` resumes now rewrite the round heading and PR note instead of stacking them. Old task
  texts are read as before; the first resume cleans them up.

## Not done (proposals)

- **Install.** The installed `~/.local/bin/agent` predates
  [`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f);
  none of this takes effect until the human runs `install.sh` (agents must not).
- **The prompt's size.** `111 KB`, mostly earlier rounds' reviews carried by `--same-round` across rounds 3 and 4.
  A proper round start (without `--same-round`) leaves them out. A cap on "For this round only" blocks (the newest
  two, say) would be next.
- **`gh api` and `git push` are not guarded.** A member can still `gh api -X POST …/pulls`, or push to the
  coordinator's branch. A per-run `pre-push` hook (`core.hooksPath` through `GIT_CONFIG_*` in the run's
  environment) could refuse a member's push to any branch but its own.
- **A member's checkpoint and board items.** `checkpoint` and `board_add` accept a member's next-round plan. The
  prompts now forbid it. The bus could label a member's checkpoint as such in the resume prompt, or refuse a
  member's `normal` board items.
- **`agentbus team-get` without `--team`** answers "no team". A message that points an agent to the MCP tool
  `team_status` would have helped haiku once tool search failed it.
- **`--autopilot` after a merge.** After `claude-haiku` merged #53, the autopilot still ran fix pass 2 on it
  (run `20261104-103821`). A fix pass could check first that the round's PR is still open.
- Suggested `AGENTS.md` note: "Claude runs get every tool up front (`CLAUDE_TOOL_SEARCH`); a team member's PR
  commands are refused by the agents' gh".

## Checks: one failure outside this change

`make gate` in an agent's run fails `review_byline.bats` test 1 ("an investigator's review gets its byline at the
top …"), at the base [`c7ad84b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c7ad84bf90e13180e69eec787b38c5326ab95a3f)
too. The test inherits the runner's `AGENT_GITHUB_NAME_COLORS` and `AGENT_GITHUB_COLORS`, so the byline it expects
is coloured. Without them it passes (`env -u AGENT_GITHUB_NAME_COLORS -u AGENT_GITHUB_COLORS make gate`: all
`7` steps passed). The test (or `helpers.bash`) should unset them; that is a test change, for the human to allow.
