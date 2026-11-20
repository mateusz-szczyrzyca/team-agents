# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED

You work on task {{task}} again, in a new session. The session before you went bad:

{{reason}}

You do not have its memory; this briefing is what you need to go on without asking anybody.

Go on from this state. Do not start over, and do not undo the commits below: the work in the worktree and on the
branch is the work of the runs before yours and is yours to finish.

## The task you were given

{{prompt}}

## The branch

The work is on branch {{branch}}, forked from {{base}}.

{{commits}}

## What is uncommitted

Everything the worktree had when the last run before yours began is kept as a snapshot, {{snapshot}}. The worktree
keeps it all, but none of it is in your branch until you commit it.

{{uncommitted}}

{{plan}}

{{messages}}

## What to do now

1. Find where the work stands: read the files above, the end of the branch's history and the task's own prompt.
2. Go on with what is left of the task, in the same worktree, the same branch and the same task.
3. Follow your rules: test first, small commits, and only the work of this task.
4. End your turn with a `STATUS:` line, as your first message says.
