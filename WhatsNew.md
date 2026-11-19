# What is new in `agent`

Newest first. One entry per version, written by the pull request that raises it — the rule and the form are in
[`AGENTS.md`](AGENTS.md), "Commits and pull requests". The entries start with 0.18.0; what came before is in the
merged pull requests.

## 0.18.0 — 19.11.2026 — PR #133

### New features
- `[build] incremental = true` in the configuration turns incremental compilation back on for agents and the gate.

### Enhancements
- The build directory of a task's worktree is about three times smaller (13 GB → 4 GB after one gate).
