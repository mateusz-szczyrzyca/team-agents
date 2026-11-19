# What is new in `agent`

Newest first. One entry per version, written by the pull request that raises it — the rule and the form are in
[`AGENTS.md`](AGENTS.md), "Commits and pull requests". Every line names in brackets the crates or modules the
change is in: `[agent]`, `[agent-core]`, `[agent-runtime]`, `[agent-web]`, `[agentbus]`. The entries start with
0.16.0; what came before is in the merged pull requests.

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
