# YOUR WORK IS NOT PUBLISHED: YOUR BRANCH HAS NONE OF THE FINISHED ITEMS (sent back {{sent}} of {{limit}})

You ended your turn with `STATUS: DONE`, and the gate passed at the tip of your branch `{{branch}}`. The harness opens
the pull request for the finished items whose branches your branch contains, and it contains none of them:

{{missing}}

Merge each of these branches into `{{branch}}` (switch to it first when your worktree has another one out):
`git merge <branch>`, and settle the conflicts. If you did an item's work over on your own branch and its changes must
not come in a second time, record its branch as merged with `git merge -s ours <branch>` and say in the commit message
why. An ours merge changes no files; the checks judge other merges against their first parent on the round's own paths.
Keep frozen tests unchanged when settling conflicts. Then check with `agent gate` that the gate passes at the new tip
and end your turn with `STATUS: DONE`. If a branch cannot be merged, say why and ask with `STATUS: QUESTIONS`.

Do not push, and do not open or update the pull request: the harness does both when the gate has passed.
