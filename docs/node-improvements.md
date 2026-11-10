# Improvements to the work of remote nodes, still to be made

A list of what the lead/node design ([multi-instance.md](multi-instance.md)) does not do yet. Each item says what
is missing, why it matters and what a change needs. Remove an item when it is done; add new ones at the end.

## 1. Usage is not synchronised from nodes

**What is missing.** A node's runs report their token usage into the node's own `activity.jsonl`. The lead's
reports — `agent tokens <team>`, `agent tokens --round <id>`, `agent spend` — read the tasks in the lead's
`~/agents/tasks/` only. A remote member's proxy task on the lead has no runs and no usage of its own, so a lead's
report does not see the runs of its remote members: the team's and the round's sums leave them out. The task mirrors
(`<bus>/remote/<node>/<task-id>/`) do carry the activity, but only its last 2,000 lines, and no report reads them.

**Why it matters.** "What did R12 cost, in which models and roles" has no complete answer for a team with members on
nodes.

**What a change needs.** The run marker makes it small. Every run's start line carries the round and the sub-round
it went into — `▶ run 7 (resume) round R12/A pr #74`, or `pr -` before a PR — and the usage lines that follow it
belong to that run. So a later synchronisation needs to send only those lines — each run's marker and the usage
lines after it, with the node's task ID, agent and model —, not the nodes' task graph: the lead sums them by the
round and the sub-round in the marker, as `run_subrounds` in `bin/agent` does for its own tasks. Points to settle:

- Where the lead keeps them: next to the task mirrors, with no limit of lines that could drop a run's usage.
- What the lead's report counts them as: the remote member's proxy task (role `member`), a node's worker or
  investigation under that member.
- The node reads the round from the job's task text (`job_round_id`); a resumed remote member's text can hold the
  human's own `Round:` line after the original one, which makes it `?`. The round should travel with the job
  instead (a field of the job), and the PR too: a node's task does not see its team's PR on the lead's bus, so its
  markers say `pr -`.
- Compatibility: a node of an older version sends no markers; its usage counts under `<round>/?`, never dropped.
