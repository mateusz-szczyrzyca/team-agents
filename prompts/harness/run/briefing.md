# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED

You work on task {{task}} again, in a new session. The session before you went bad; the harness recorded why as
`{{reason}}`:

- `no_status`: its last runs ended without a `STATUS:` line;
- `hang`: its last runs showed no activity until the watchdog stopped them;
- `turn_limit`: its last runs used up their turn budget.
- `busy`: no run of it went bad: its agent was busy in another team for longer than `[replacement] busy_after`, and a
  free agent took the task over while it waited.
- `daemon_down`: the daemon went down while the run was going. The run was cut from outside, not by the agent's fault.
- `failed`: the last run of the session before reported `STATUS: FAILED` after it used the bus's tools: its agent said
  that it could not finish the work. What it said is at the end of this briefing; read it first.
- `human`: no run went bad: the human started this new session with `agent renew`. Read the messages at the end of
  this briefing first: they say what the human wants.

You do not have its memory; this briefing is what you need to go on without asking anybody.

{{checkpoint}}

This is where the work stands. Go on from it: do what is left, do not do again what is done.

Go on from this state. Do not start over, and do not undo the commits below: the work in the worktree and on the
branch is the work of the runs before yours and is yours to finish.

## The task you were given

{{prompt}}

## Who worked on this task

The runs of this task before yours, oldest first, each with the agent that ran it and how it ended (none when the list
is empty). `DONE` and `QUESTIONS`: the run reported its outcome; `no status`: it ended without a `STATUS:` line; `hang`:
the watchdog stopped it; `turn limit`: it used up its turn budget; `stopped`: a human stopped it; `failed`: the agent
could not do the work or the account or the provider stopped it:

{{runs_left_out}}

{{runs}}

A run told as `cut by a stop of the daemon` or `cut before its end was recorded` was cut while the daemon stopped or
crashed: that is no fault of the agent that ran it.

## The branch

The work is on branch {{branch}}, forked from {{base}}. Its commits since then, oldest first (none when the list is
empty):

{{commits}}

## The pull request and its review

{{pull_request}}

Where the review stands: {{review_state}}

The findings of the reviews of this pull request, by their permanent number (none when the list is empty). `open`:
a verifier has not accepted its fix yet, and it is yours to fix; `resolved`: every verifier accepted its fix, so do
not touch it again; `not confirmed`: an independent opinion did not confirm it, so leave it alone:

{{findings}}

## What is uncommitted

Everything the worktree held when the session before yours ended is kept as a snapshot, {{snapshot}}. The worktree
still holds it all; none of it is on your branch until you commit it. The files that differ from the last commit (none
when the list is empty):

{{uncommitted}}

## The plan of steps

The steps of `.agent/plan.md` as the runs before yours left them, in four groups. `Confirmed`: the harness ran the
step's check and it passed. `Declared`: the agent marked the step done, and no check ran, or the check waits for a
reviewer. `Mismatch`: the agent marked the step done, and its check failed. `Not done`: `[ ]` to do, `[>]` in hand,
`[-]` given up. Declared steps are what the agent says, not evidence: check them. An empty list: the task keeps no
plan, or it does not read.

{{plan}}

## The human's answers

The answers of the human to the questions of this task, oldest first, each with its verdict: `ok` accepts the
recommendation, any other text is the human's own answer, which binds you as the task's instruction (none when the
list is empty):

{{answers_left_out}}

{{answers}}

## Unread messages

The messages of your team that no run of this task read, oldest first (none when the list is empty).
{{left_out}}

{{messages}}

## What to do now

1. Read the messages of your team first, if you have the tools of the bus (`read_messages`).
2. If a pull request is listed above, it is yours: go on in it, on its branch. Do not open another pull request and do
   not start the item again. In a fix pass, fix every `open` finding, then answer for each one as the task's prompt
   says.
3. Find the real state first: the commits above, the uncommitted files listed above, and the gate of the task. Trust
   the steps listed as confirmed, check the declared ones, and repair the mismatched ones before anything else.
4. Go on with what is left of the task, in the same worktree, the same branch and the same task; follow the rules of
   the task's prompt and do only the work of this task.
5. End your turn with a `STATUS:` line, as the task's prompt says.
