# Questions — {{task}}

## 1. The plan of {{agent}} is still not right after {{count}} corrections: go on, or send the run back once more?
{{agent}} ended its turn with `STATUS: DONE`, and its plan `.agent/plan.md` says otherwise: {{reason}}.
The harness sent the run back {{count}} times, as often as `[plan] bounces` allows.

a) go on as it is — the run counts as done, and this task is not sent back for its plan again
b) send it back once more — the agent is told the reason again and gets one more correction

**Recommended:** a) — {{count}} corrections did not settle it, so one more is unlikely to; read the plan and the work
yourself before the pull request is merged (the harness)

**Verdict:**
