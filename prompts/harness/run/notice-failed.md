{{agent}} failed ({{task}})
The run of {{agent}} ended with STATUS: FAILED in task {{task}}.
What it said:

{{message}}

The harness does not start this task again by itself: the ladder has no rung left, or `[replacement] mode` is `off`.
Look at what it said and the work in the worktree, then run `agent resume {{task}}` to start it again.
