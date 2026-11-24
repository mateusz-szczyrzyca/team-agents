## What the member's runs left

The harness recorded why its last runs went bad as `{{reason}}`: `no_status` — they ended without a `STATUS:` line;
`hang` — they showed no activity until the watchdog stopped them; `turn_limit` — they used up their turn budget;
`daemon_down` — the daemon went down while they were going: they were cut from outside, not by the agent's fault.
`failed` — the member's last run reported `STATUS: FAILED` after it used the bus's tools: it said that it could not finish
the work, and what it said is given below.

The runs of the member's work, oldest first, each with the agent that ran it and how it ended (none when the list is
empty):

{{runs_left_out}}

{{runs}}

The work of task {{task}} is on branch {{branch}}, forked from {{base}}. Its commits since then, oldest first (none when
the list is empty):

{{commits}}

Everything the member's worktree held when its last run ended is kept as a snapshot, {{snapshot}}. The files that
differ from the last commit there (none when the list is empty) — they are on no branch until they are committed:

{{uncommitted}}

The steps of the member's `.agent/plan.md` as its runs left them — `[ ]` to do, `[>]` in hand, `[x]` done, `[-]` given
up. They are grouped: `Confirmed` — the harness ran the step's check and it passed; `Declared` — the member marked the step
done and no check ran, or it waits for a reviewer; `Mismatch` — the member marked the step done and its check failed;
`Not done` — `[ ]` to do, `[>]` in hand, `[-]` given up. Declared steps are what the member says, not evidence: check
them, and repair the mismatched ones first. An empty list: the task keeps no plan, or it does not read.

{{plan}}

The human's answers to the questions of the task, oldest first, each with its verdict: `ok` accepts the recommendation,
any other text is the human's own answer (none when the list is empty):

{{answers_left_out}}

{{answers}}

The messages to the member that no run of it read, oldest first (none when the list is empty).
{{left_out}}

{{messages}}

The member's worktree and branch are yours to use now: you may commit there. Commit the files listed as uncommitted above
first, as they are, then integrate the branch into yours.
