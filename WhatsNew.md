# What is new in `agent`

Newest first. One entry per version, written by the pull request that raises it — the rule and the form are in
[`AGENTS.md`](AGENTS.md), "Commits and pull requests". Every line names in brackets the crates or modules the
change is in: `[agent]`, `[agent-core]`, `[agent-runtime]`, `[agent-web]`, `[agentbus]`. The entries start with
0.16.0; what came before is in the merged pull requests.

## 0.33.0 — 20.11.2026 — PR #151

### New features
- [agent] [agent-core] [agent-runtime] The harness can check a coordinator's report that an item is delivered by
  another pull request of its team (`DELIVERED: #N`). It accepts it only when that pull request belongs to the team and
  the item's branch holds nothing outside it or the base branch; otherwise the lead is told why. An accepted item opens
  no pull request, `agent status` shows `delivered by PR #N, waits for its merge`, and when #N is merged the item is
  done as `done, delivered by PR #N`. A stop and a resume of the team in between change nothing: the item still
  waits, opens no second pull request and closes at the merge, also when it is a part of a split item. Coordinators are not told about the report yet: that and the return to work when
  #N is closed without a merge come in the next version.

### Enhancements
- [agent-core] The state file is now schema 14 and still reads every state written by earlier versions; an older `agent`
  cannot read a state written by this one, so keep a backup from before the upgrade.

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
