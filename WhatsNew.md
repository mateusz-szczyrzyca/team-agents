# What is new in `agent`

Newest first. One entry per version, written by the pull request that raises it — the rule and the form are in
[`AGENTS.md`](AGENTS.md), "Commits and pull requests". Every line names in brackets the crates or modules the
change is in: `[agent]`, `[agent-core]`, `[agent-runtime]`, `[agent-web]`, `[agentbus]`. The entries start with
0.16.0; what came before is in the merged pull requests.

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
