{{why}}

These commits are in the branch for good and the team cannot undo them: only a rewrite of the published history could, and that is never done. A test is frozen from its `RED:` commit on; it changes later only in a `TEST-FIX:` commit of its own that touches test files only.

{{commits}}
{{before}}
Write `{{allow}}` to let the round go to its review with these commits as they are: the harness records each commit and path for the item and does not ask about them again, through fix passes, merges of the base, a split into parts and restarts of the daemon. A new commit that changes a frozen test is a new question. Write `{{refuse}}` to record that you refused them: nothing is sent back to the team, which cannot undo a commit that is in the branch; the refusal is kept as well and is not asked again. Use an explicit verdict.
