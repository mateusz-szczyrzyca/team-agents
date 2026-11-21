# What is new in `agent`

Newest first. One entry per version, written by the pull request that raises it — the rule and the form are in
[`AGENTS.md`](AGENTS.md), "Commits and pull requests". Every line names in brackets the crates or modules the
change is in: `[agent]`, `[agent-core]`, `[agent-runtime]`, `[agent-web]`, `[agentbus]`. The entries start with
0.16.0; what came before is in the merged pull requests.

## 0.42.0 — 21.11.2026 — PR #160

### New features
- [agent] [agent-core] **Action:** a task whose new session goes bad too can now be taken over by another agent: list them in `[replacement] agents = ["claude-2", "inv-opus"]`. The first one that is free, not in the task's team and of the same or a higher place carries on in the same worktree and on the same branch, briefed on where the work stands. `[replacement] replacements = 1` says how many times a task may be taken over. Without `agents` nothing changes.
- [agent] [agent-core] When no listed agent fits, a member's task stops and its team's lead gets the member's work in a message on the bus, with its branch and worktree; the item is published when the lead ends `DONE`. `[replacement] mode = "lead"` does this at once; `mode = "off"` fails the task as before.
- [agent] [agent-core] `agent status` says from whom a task was taken over, for example `claude-2  queued, in a new session, for claude-1`.

## 0.41.0 — 21.11.2026 — PR #159

### New features
- [agent-core] A pure table decides who takes over a task whose own agent cannot go on: the first idle agent of the same or a higher place (`[replacement] agents`, in the order you name them), otherwise the team's lead, otherwise you. The place of an agent is its tier: a worker below a normal agent below an investigator.
- [agent-core] The state keeps who held a task before the agent that holds it now, with the time, so that a task given to another agent still shows whose work it is. This part only decides and records; nothing acts on it yet. No migration: an older `state.json` loads as before.

## 0.40.0 — 20.11.2026 — PR #156

### New features
- [agent] [agent-core] [agent-runtime] A task whose session went bad goes on in a new session of the same agent: after the two retries of a run that ended without a `STATUS:` line, after the watchdog's last strike for a hung run and after the last turn-limit resume, the task is started once more in a fresh session instead of failing. Its first prompt is a briefing: why it was started anew, the task's whole prompt, the commits of its branch, the files it left uncommitted (kept in a snapshot first), its plan of steps and the messages of its team it had not read. When the new session goes bad too, the task fails as before.
- [agent-core] The new key `[replacement] new_sessions` says how many such new sessions a task gets: `1` by default, `0` keeps the old behaviour. The count is kept in `state.json`, so a restart of the daemon does not reset it.
- [agent] [agent-core] `agent status` says it on the task's line: `claude-1   queued, in a new session`.

### Bug fixes
- [agent] An item no longer stands behind a member the human stopped: when the lead ends `STATUS: DONE` with a passed gate, the item is published with the lead's branch.

## 0.39.0 — 20.11.2026 — PR #154

### New features
- [agent] [agent-core] [agent-runtime] When GitHub refuses to merge an approved pull request because it conflicts with its base, the lead of the round is sent to merge the base into its branch, not told to you at once. The result is gated and pushed, and the merge is tried again; a change to files the task does not allow is verified by the reviewers first, and `agent status` shows `waits for the lead's correction` while the pass is idle. The new `[merge] conflict_passes` setting (default `3`) limits these passes per round; when they are used up, or the lead gives no result, the lead asks you once in its questions file. `conflict_passes = 0` keeps the old behaviour.

### Enhancements
- [agent-runtime] A refused merge is told apart as a conflict or another refusal from GitHub's answer; a rate limit or a failing GitHub is still retried, not told.

## 0.38.0 — 20.11.2026 — PR #157

### New features
- [agent] [agent-core] [agent-runtime] A coordinator now proposes a split itself: it writes `split-proposal-<item>.json` in its task directory (the parts with their checks, dependencies, an estimate and a tier, and whether the scope changes) and ends its run with the line `SPLIT: 1.1`. A proposal that reads, has checks on every part, has no dependency cycle, stays within the size limit and keeps the scope is approved at once with no questions file, and its parts are recorded whole; the round goes on as the first part.
- [agent] [agent-core] A proposal with a fault goes back to the coordinator with its own text naming the part (does not read, a part without checks, a cycle, a part above the limit), at most `[split] corrections` times (default 2).
- [agent] [agent-core] You are asked once, in the task's questions file, when a proposal changes the scope and when the corrections are used up; the question says which condition failed and holds the proposal as the example of the verdict. A verdict `split` with the plan, as before, still works.
- [agent] A member that finds its item too big tells its coordinator, not you.
- [agent-core] New key `[split] unit` (default `"lines"`): the unit the size limit `part_limit` is applied to. Use `pages` or `images` for work that is not code. **Action:** none, the default is today's behaviour.

### Enhancements
- [agent-core] [agent-runtime] The history keeps a record of every decision on a split proposal. **Action:** the history database migrates to version 15 on the next daemon start; an older `agent` cannot read it, so keep a backup from before the upgrade.
- [agent] A restart of the daemon between a proposal and its decision, or between a correction and the next proposal, loses neither.

## 0.37.0 — 20.11.2026 — PR #155

### New features
- [agent-core] A split can now be written as a proposal: for each part its title, what it delivers, its checks, its dependencies, an estimate (a number and its unit, for example `900 lines`) and the tier of agent it asks for (`investigator`, `normal` or `worker`; none means `normal`), and whether the split changes the scope of the item. The approving verdict of today, without the new fields, is still read as before.
- [agent-core] The decision on a proposal exists: approve it with no human, send it back with the fault named by part (it does not read, a part without checks, a dependency cycle, a part above the limit), or ask the human when it changes the scope or the corrections are used up. The daemon does not use it yet.
- [agent-core] New keys `[split] part_limit` (the largest estimate a part may have, default 1500, `0` checks no limit) and `[split] corrections` (how many times a proposal goes back, default 2) in `agents.toml`; a file without `[split]` gets the defaults.

### Enhancements
- [agent-core] A proposal and its corrections are kept in `state.json` with the item, so a restart loses neither. The schema number stays 14 and the new fields are optional, so a stored split plan written by 0.36.0 loads, with its parts asking for `normal`. **Action:** a plan written by this version has fields an older `agent` rejects, so keep a backup of `state.json` from before the upgrade.

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
