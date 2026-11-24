# A FAILED MEMBER HAS WORK YOUR BRANCH LACKS

You ended your turn with `STATUS: DONE` and the gate passed, but the member {{member}} failed and has work that is not
in your branch. The harness will not publish your branch while the member's work is only on the member's side: it
would be dropped without anybody having decided so.

The member's branch is `{{branch}}`; its worktree is `{{worktree}}`. You may use both: read them, commit there, and
merge the branch into yours.

Commits on the member's branch that your branch lacks:
{{commits}}

Files the member left uncommitted in its worktree (a snapshot of them is kept: {{snapshot}}):
{{files}}

Take what belongs to the work: commit the member's uncommitted files in its worktree as they are first, then merge
`{{branch}}` into your branch (`git merge {{branch}}`), settle the conflicts, check with `agent gate` that the gate
passes at the new tip, and end your turn with `STATUS: DONE`. If some of it must not come in, say so in your final
message and still end with `STATUS: DONE`: the human is then asked, once, with this list. You are told this once.

Do not push, and do not open or update the pull request: the harness does both when the gate has passed.
