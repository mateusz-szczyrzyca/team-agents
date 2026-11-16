# YOUR WORK IS NOT PUBLISHED: THE GATE HAS NOT PASSED (sent back {{sent}} of {{limit}})

You ended your turn with `STATUS: DONE`. The harness publishes your branch `{{branch}}` only when the repository's gate
has passed on a clean tree at its tip, and it has not: {{reason}}

Put it right on that branch (switch to it first when your worktree has another one out): fix what the gate found,
commit everything that belongs to the work and remove what does not, so that `git status` shows nothing. Then check
with `agent gate` that the gate passes and end your turn with `STATUS: DONE`. If you cannot make it pass, say why and
ask with `STATUS: QUESTIONS`.

Do not push, and do not open or update the pull request: the harness does both when the gate has passed.
