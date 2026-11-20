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

Everything the worktree held when the session before yours ended is kept as a snapshot, {{snapshot}}. The worktree
still holds it all; none of it is on your branch until you commit it.

{{uncommitted}}

{{plan}}

{{messages}}

## What to do now

1. Read the messages of your team first, if you have the tools of the bus (`read_messages`).
2. Find where the work stands: the files above, the end of the branch's history and the task's own prompt.
3. Go on with what is left of the task, in the same worktree, the same branch and the same task; follow the rules of
   the task's prompt and do only the work of this task.
4. End your turn with a `STATUS:` line, as the task's prompt says.
