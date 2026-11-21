## What the member's runs left

The harness recorded why its last runs went bad as `{{reason}}`: `no_status` — they ended without a `STATUS:` line;
`hang` — they showed no activity until the watchdog stopped them; `turn_limit` — they used up their turn budget.

The work of task {{task}} is on branch {{branch}}, forked from {{base}}. Its commits since then, oldest first (none when
the list is empty):

{{commits}}

Everything the member's worktree held when its last run ended is kept as a snapshot, {{snapshot}}. The files that
differ from the last commit there (none when the list is empty) — they are on no branch until they are committed:

{{uncommitted}}

The steps of the member's `.agent/plan.md` as its runs left them — `[ ]` to do, `[>]` in hand, `[x]` done, `[-]` given
up. They are what the member declared, not evidence: check them. An empty list: the task keeps no plan, or it does not
read.

{{plan}}

The messages to the member that no run of it read, oldest first (none when the list is empty).
{{left_out}}

{{messages}}
