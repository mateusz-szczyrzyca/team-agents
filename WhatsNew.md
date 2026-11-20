# What is new in `agent`

Newest first. One entry per version, written by the pull request that raises it — the rule and the form are in
[`AGENTS.md`](AGENTS.md), "Commits and pull requests". Every line names in brackets the crates or modules the
change is in: `[agent]`, `[agent-core]`, `[agent-runtime]`, `[agent-web]`, `[agentbus]`. The entries start with
0.16.0; what came before is in the merged pull requests.

## 0.37.0 — 20.11.2026 — PR #154

### New features
- [agent] [agent-core] [agent-runtime] When GitHub refuses to merge an approved pull request because it conflicts with its base, the lead of the round is sent to merge the base into its branch, not told to you at once. The result is gated and pushed, and the merge is tried again; a change to files the task does not allow is verified by the reviewers first, and `agent status` shows `waits for the lead's correction` while the pass is idle. The new `[merge] conflict_passes` setting (default `3`) limits these passes per round; when they are used up, or the lead gives no result, the lead asks you once in its questions file. `conflict_passes = 0` keeps the old behaviour.

### Enhancements
- [agent-runtime] A refused merge is told apart as a conflict or another refusal from GitHub's answer; a rate limit or a failing GitHub is still retried, not told.

## 0.36.0 — 20.11.2026 — PR #153

### New features
- [agent] [agent-core] The coordinator's prompt now tells a lead when and how to report an item as delivered by another pull request of its team: the line `DELIVERED: #N` before the final `STATUS: DONE`, what the harness checks, and what a refusal means. A member is told what it means for its item.
- [agent] [agent-core] [agent-runtime] When the pull request named in an accepted report is closed without a merge, the item no longer waits for ever: the lead is told, the item goes back to its member in a new round and is published by its own pull request. `agent status` shows the old round as closed and the new one as open. A pull request that is closed and opened again before the daemon saw it closed is still waited for; one the daemon saw closed stays closed. A restart of the daemon at any point of this goes on from where it was. The returned item is published on a branch of its own, so it does not wait for the closed pull request.
- [agent] A failure to send the lead's notice no longer stops the daemon: it is logged as `bus_down` and sent at the next poll.

### Enhancements
- [agent-core] [agent-runtime] A closed delivering pull request is logged with its own `delivery_closed` warning event. **Action:** the history database migrates to version 14 on the next daemon start; an older `agent` cannot read it, so keep a backup from before the upgrade.

## 0.35.0 — 20.11.2026 — PR #152

### New features
- [agent] [agent-core] [agent-runtime] The daemon reads the plan a task keeps in `.agent/plan.md` at the end of every run and keeps each new version in the history; a missing plan and one that does not parse are recorded too.
- [agent] [agent-core] A solo agent, a member or a coordinator that ends `STATUS: DONE` while its plan is missing, does not parse, has a step still to do or in hand, or lost a step goes back to the same agent with the reason, at most `[plan] bounces` times per task (default 3, `0` only records).
- [agent] After that limit you are asked once, in the task's questions file, whether to go on as it is (`a` or `ok`) or send the run back once more.
- [agent] The first prompt of a solo agent, a member and a coordinator says how to keep the plan and its form, for example one step in hand and a step given up with its reason instead of deleted.

### Enhancements
- [agent] [agent-runtime] The plan stays out of git and out of the pull request: new worktrees ignore it, and a plan an agent commits by mistake is taken out of its branch by a commit "Keep the plan out of the delivery".
- [agent] Tasks that were open before this version are never sent back for their plan; their plan is recorded all the same.
- [agent-runtime] The history database migrates automatically to version 13 on the next daemon start.

## 0.34.0 — 20.11.2026 — PR #151

### New features
- [agent] [agent-core] [agent-runtime] The harness can check a coordinator's report that an item is delivered by
  another pull request of its team (`DELIVERED: #N`). It accepts it only when that pull request belongs to the team and
  the item's branch holds nothing outside it or the base branch; otherwise the lead is told why. An accepted item opens
  no pull request, `agent status` shows `delivered by PR #N, waits for its merge`, and when #N is merged the item is
  done as `done, delivered by PR #N`. A stop and a resume of the team in between change nothing: the item still
  waits, opens no second pull request and closes at the merge, also when it is a part of a split item. Coordinators are not told about the report yet:
  that and the return to work when #N is closed without a merge come in the next version.

### Enhancements
- [agent-core] The state file is now schema 14 and still reads every state written by earlier versions; an older `agent`
  cannot read a state written by this one, so keep a backup from before the upgrade.

## 0.33.0 — 20.11.2026 — PR #150

### New features
- [agent] [agent-runtime] When the daemon starts, every task whose run was cut by a crash or a stop of the daemon gets a snapshot of its worktree if its round is still open: all uncommitted, staged, deleted and untracked files (binary ones too; ignored files are left out) are kept as a commit under `refs/agents/snapshots/<round>/<task>/<n>`, without changing the worktree, its index or its files.
- [agent] `agent status <round>` shows the last snapshot of each task at the end of the round view, for example `snapshot task-… number 1 at 14:05:10, 4096 bytes`.
- [agent-core] [agent-runtime] The new optional `[snapshot] warn_mb` setting (default `200`) sets the size in megabytes above which a snapshot is logged with a `snapshot_large` warning, and the history database migrates automatically to version 12 on the next daemon start.

### Enhancements
- [agent-runtime] Snapshots are recorded in the history database (`snapshots`) and can be read back by round.

## 0.32.0 — 20.11.2026 — PR #149

### Enhancements
- [agent-core] The core can now read, write back and compare the plan of steps a task will keep in its worktree;
  nothing the daemon or the CLI does changes yet.

## 0.31.0 — 20.11.2026 — PR #148

### New features
- [agent] [agent-core] An explicit split approval records every part together. Each part gets its own round and pull
  request when its dependencies have merged; the item stays open until its last part merges, also after a restart.
- [agent] [agent-core] `agent answer` reports which split parts were recorded or why approval was refused, and
  `agent status` shows each part as done, in progress, waiting for a dependency, or to do.

### Enhancements
- [agent] [agent-core] The CLI negotiates socket protocol 4 for split details. Older clients keep protocol 3, and
  the new CLI falls back to protocol 3 when talking to an older daemon.

## 0.30.0 — 20.11.2026 — PR #147

### Bug fixes
- [agent] [agent-runtime] [agentbus] An agent that works in two open teams gets a message from a team, and wakes for
  it, only in that team: its session in the other team no longer reads it and acts on it, and reading in one team
  no longer marks the other team's messages as read. A running or configured agent is no exception.
- [agent] [agent-runtime] [agentbus] `agent say <task>` to a solo task reaches that task's session only, also when the
  agent is in an open team.
- [agentbus] A reservation belongs to the task that took it: the same agent's run in another team can no longer
  release it or free it when that run ends.

### Enhancements
- [agent] [agent-core] `agent run` says which of its agents are also in another open team, for example
  `codex-1 is also in the open team team-… (task task-…): its messages there do not reach this team`.
- [agent] [agentbus] A message from a team to a name that is not in that team is refused with the list of the team's
  members. The history of `read_messages` and `read_message` show a session nothing of the agent's other teams.
- [agent] Team members are told that only their own worktree and branch are theirs to change, and that a message
  naming another task's branch is not for them.

## 0.29.0 — 20.11.2026 — PR #146

### Enhancements
- [agent-core] Team state can retain complete split plans with lettered parts, deliverables, checks and dependencies.
  Automatic execution of those parts will follow in the next part; answers do not activate splits yet.
- [agent-core] **Action:** State files now use schema 13 and older states migrate automatically without changing
  their history. Keep a backup before upgrading: older binaries cannot read the upgraded state.

## 0.28.0 — 20.11.2026 — PR #145

### Bug fixes
- [agent] [agent-core] When a round has findings only you may allow and findings the team can fix, you are asked
  about your part before the team starts its next pass. Your question uses no team pass and does not give the team
  more passes than it had.
- [agent] [agent-core] What you allowed for a file is asked about again when the team changes that file afterwards.
  Allowing a protected file no longer allows a lint suppression or another finding of the team's added to it.

## 0.27.0 — 20.11.2026 — PR #143

### Bug fixes
- [agent] [agent-core] [agent-runtime] A pause of the network or of GitHub no longer stops the daemon: the round waits
  and the step is tried again by itself, 30 seconds after the failed call, then after a pause that doubles up to 10
  minutes, while the other teams go on.
- [agent] A failure of git while the daemon reads a worktree's commit (before a merge, after a fix, or the commit a
  reviewer verified) or prepares a second opinion no longer stops the daemon: the step waits.
- [agent-runtime] **Action:** The history learns the daemon's newest log events (it refused them before, and
  `build_directory_removed` on histories made before 0.19.0); `history.db` is migrated to version 11 at the start of
  the daemon.

### New features
- [agent] [agent-core] `agent status` says what a round waits for at GitHub, for example `waits for GitHub: refresh the
  description of the pull requests, tried 4 times, next try 01:07`; the log has one `github_step_waiting` warning when
  a step starts to wait and one `github_step_resumed` when it goes through.
- [agent] When GitHub or git refuses a step for good (a 404, a token that may not do it) you are told once while it
  waits, and `agent resume <team>` tries it again at once without starting any task of the team.

## 0.26.0 — 20.11.2026 — PR #142

### New features
- [agent] [agent-core] `agent run --allow-unpushed-base` permits an unpublished base explicitly, prints its
  commit count and first commit, and retains that evidence across daemon restarts in every pull request.

### Bug fixes
- [agent] [agent-runtime] `agent run` fetches the base from origin before starting and refuses unpublished
  commits or a failed fetch, so teams no longer inherit unpushed commits silently.

## 0.25.0 — 20.11.2026 — PR #141

### Enhancements
- [agent] **Action:** The first implementation is removed from the repository: the Bash `agent`, its unfinished Go
  version, the terminal panel and the old web interface (`agent live`, `agent web`), their tests and the installer.
  The Rust `agent` and `agentbus` are what is left. An installation of the old version can no longer be installed
  or updated from this repository; git's history has it.
- [agent] `make gate-full` checks `agentbus` and the Rust workspace only. `make unit-bats` does nothing and stays
  until the CI workflow stops calling it.
- [agent] The README describes the Rust `agent` alone: how to build it, set up its home and start the daemon.

## 0.24.0 — 20.11.2026 — PR #139

### Bug fixes
- [agent] A coordinator or member whose run sleeps can review another team while remaining excluded from its own.
- [agent-core] Reviewer stories no longer claim that deterministic assignments were random.

### Enhancements
- [agent] [agent-core] Reviews, verifications and second opinions choose investigators in a fixed order: first those who have not worked for the team, then those asked longest ago, with work for other teams breaking ties.
- [agent] [agent-core] A round without enough reviewers, a second opinion nobody can take or a failed reviewer nobody can replace says it waits for a free independent investigator.

## 0.23.0 — 20.11.2026 — PR #140

### New features
- [agent] [agent-core] `agent status --investigators` lists every configured investigator with its last two activities in the teams that are not closed, newest first, for example `task-…  reviewer  inv-opus  for team-…  done at 12:15:15, 2nd review finished`; one with nothing to do shows `free, no activity in an open team`.

### Enhancements
- [agent] [agent-core] A reviewer's line in `agent status` and `agent status --running` no longer says `waiting` between reviews: it says `working`, `done, 1st review finished 12:08:17`, or `stopped at 12:10:00, reason: the human stopped it`.

## 0.22.0 — 19.11.2026 — PR #137

### New features
- [agent] [agent-core] The daemon remembers each investigator's work for a team: crew membership, reviews, verifications and second opinions, including when they were requested, reported, stopped or failed.

### Enhancements
- [agent-core] **Action:** state files move to schema 11. Older files migrate automatically, rebuilding investigator activity history from the crew, reviews, fix passes and second opinions they retain.

## 0.21.0 — 19.11.2026 — PR #132

### New features
- [agent] [agent-core] When free disk space falls below `disk.low_free_gb` (default 100 GB), `agent status` shows one line with the free space and the limit, and you get one notification listing the five largest worktrees.
- [agent] [agent-core] **Action:** a new optional `[disk]` section with `low_free_gb` and `min_free_gb`; without it the defaults apply. When free disk space falls below `min_free_gb` (default 20 GB), `agent run` is refused with the free space and what to remove; queued rounds wait and go on by themselves when space is back.

### Bug fixes
- [agent] When the state cannot be saved, the daemon first runs the notification command with the reason, then stops as before; before, it stopped without telling anyone.

### Enhancements
- [agent] [agent-runtime] Build directories (`target/`) are removed when their round closes, and a reviewer's is removed at the end of its run when free space is low, so builds no longer fill the disk.
- [agent] The findings comment of a pull request is a list with counters, and closed findings are folded in one block, instead of a table.

## 0.20.0 — 19.11.2026 — PR #136

### Bug fixes
- [agent] A reviewer's report that is right for one item of a pull request and wrong for another no longer stops the
  daemon: the report goes back to the reviewer with the refused line and the answers allowed there, and nothing of it
  is recorded.
- [agent] A lead's explanation of a finding is kept for every item of the pull request, also for an item that joined
  it later; before, it was kept for one item only and the verifiers' answers were refused for the other.
- [agent] [agent-core] At its start the daemon repairs a state in which the items of one pull request disagree on a
  finding, and logs each repaired finding as `restart_corrected`.
- [agent] After a restart, a reviewer whose report was not taken is asked again, and a round no longer says it
  waits for an answer of the human because a reviewer that was replaced had failed.

### Enhancements
- [agent] [agent-core] The verifier's brief names beside each open finding the answers it may get, for example
  `ANSWER: FINDING 28: ACCEPTED — why or REJECTED — why or RAISED [BLOCKER] — why`, and the report is read by the
  same list.

## 0.19.0 — 19.11.2026 — PR #134

### New features
- [agent] [agent-core] A task list can start with lines `Allow: <path or glob>`, for example
  `Allow: tests/golden/*.txt`. The checks before a review no longer ask you about changes to those paths.
  `AGENTS.md`, `CLAUDE.md` and `.github/workflows/` can never be allowed this way: `agent run` refuses such a task list
  and starts nothing.
- [agent] `agent run` prints the allowances when the team starts, and the pull request lists what the round changed
  under them in `## Allowed beforehand`.

### Enhancements
- [agent-core] Moving a file from one protected place to another now asks about both places, not only the old one.
- [agent-core] The state file is migrated to the new version when it is read; a brief that started with an `Allow:`
  line before is kept as it was.

## 0.18.0 — 19.11.2026 — PR #133

### New features
- [agent-core] `[build] incremental = true` in the configuration turns incremental compilation back on for agents
  and the gate.

### Enhancements
- [agent] [agent-core] [agent-runtime] [agent-web] The build directory of a task's worktree is about three times
  smaller (13 GB → 4 GB after one gate): less debug information in development and test builds.
- [agent] [agent-runtime] Agents and the gate build without incremental compilation.

## 0.17.0 — 19.11.2026 — PR #131

### Bug fixes
- [agent] A message sent to a lead during a fix pass no longer loses the lead's fix response.
- [agent] [agent-core] A lead is woken at most twice for the same message, also after a restart of the daemon.

## 0.16.0 — 19.11.2026 — PR #130

### Bug fixes
- [agent-core] A reason after `RESOLVED` or `FIXED` is accepted, for example `FINDING 3: FIXED — moved the check`;
  before, such a line was refused and the round stood still.
- [agent-core] A lead's answer to a finding that does not block (`[MINOR]`) is accepted instead of refused.

### Enhancements
- [agent] [agent-core] A correction sent to an agent quotes the refused line and says what is wrong with it.
