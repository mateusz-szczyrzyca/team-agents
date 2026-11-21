# What's new in team-agents

Newest first. One entry per version, written for the human who installs and runs the daemon.
Every line starts with the tags of the crates or modules whose code the
change is in: `[agent]`, `[agent-core]`, `[agent-runtime]`, `[agent-web]`, `[agentbus]`. The entries start with
0.16.0; what came before is in the merged pull requests.

## 0.47.0 — 21.11.2026 — PR pending

### New features
- [agent] [agent-core] [agent-runtime] Done plan steps with command checks are executed safely in the task's worktree with the agent environment, closed input, and timeout cleanup. `[plan] check_seconds` sets the command timeout (default 120; `0` disables commands and keeps them declared). Commands matching `[plan] check_refuse` prefixes (default `["agent gate", "make gate"]`) are refused, remaining declared with a once-only notice delivered on the next run.
- [agent] [agent-core] A run reporting done with a step check mismatch is sent back to the agent with the failed step, check command, and tail output, sharing the `[plan] bounces` budget. Upon bounce exhaustion, the daemon asks the human in the task's questions file.

### Enhancements
- [agent-runtime] Check outputs are written to `checks/<run_id>/<step_id>.log` under the task directory, and process groups are killed cleanly if a command check times out.

## 0.46.0 — 21.11.2026 — PR #162

### New features
- [agent] [agent-core] An item and every part of a split are now given only to an agent of at least the tier they ask for (investigators above normal agents above workers; no `Tier:` and no tier in the split means normal). The first part of a split is handed to a fitting agent as well, not only to whoever was already running the item. A part for an investigator in a team with one investigator and one normal agent goes to the investigator, and a part for a normal agent never starts a worker.
- [agent] [agent-core] When the team has no agent of the tier an item or a part asks for, it waits and nothing is given lower. The wait is kept across a restart of the daemon, is logged once as a `team_changed` event with `kind = tier_wait`, and `agent status` shows it. The one-time question to you about it follows in the next version.
- [agent-core] The assignment text now says which tier was asked for and why this agent was chosen.

### Enhancements
- [agent] [agent-core] A fix pass after a review that asks for changes goes to an agent of at least the tier of its item or part, too: the coordinator keeps it when it qualifies, otherwise a qualified member does it on the same pull request and round, and with nobody qualified the item waits. `agent status` shows the wait of a ready later part before its round opens.
- [agent] A coordinator that is not of the tier of a part is no longer told to take that part over, and an unassigned coordinator is not given the delivery scope of a part.

## 0.45.0 — 21.11.2026 — PR #163

### New features
- [agent] [agent-core] [agent-runtime] `agent status <task>` and `agent status <round>` show plan steps with their marks and a done step's settlement: `confirmed`, `declared` or `mismatch`. The daemon checks `exists PATH` inside the worktree and retains judgements across restarts; steps without checks, reviewer checks and command checks remain declared.
- [agent-runtime] **Action:** history upgrades automatically to schema 17 while retaining older records. Keep a backup before upgrading if you may need to return to an older binary.
