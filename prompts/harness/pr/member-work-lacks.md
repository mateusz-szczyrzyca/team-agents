# A FAILED MEMBER HAS WORK YOUR BRANCH LACKS

You ended your turn with `STATUS: DONE` and the gate passed, but the member {{member}}, whose task failed or was
stopped, has work that is not in your branch. The harness does not let the work of a member be dropped without anybody
having decided so: it tells you, once.

The member's branch is `{{branch}}`; its worktree is `{{worktree}}`. They are yours to use for this: read them, commit
there, and merge the branch into yours, as the rules of your team say for this message.

Commits on the member's branch that your branch lacks:
{{commits}}

Files the member left uncommitted in its worktree (a snapshot of them is kept: {{snapshot}}):
{{files}}

Take what belongs to the work: commit the member's uncommitted files in its worktree as they are first, then merge
`{{branch}}` into your branch (`git merge {{branch}}`), settle the conflicts, check with `agent gate` that the gate
passes at the new tip, and end your turn with `STATUS: DONE`. If some of it must not come in, say so in your final
message and still end with `STATUS: DONE`: the human is then asked, once, with this list. You are told this once.

Do not push, and do not open or update the pull request: the harness does both when the gate has passed.
