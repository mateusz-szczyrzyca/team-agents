Team {{team}} holds item {{item}}: the lead is done without the work of member {{member}}

The lead of team {{team}} reported `STATUS: DONE` twice, the second time after it was told what member {{member}} (task {{task}}) has that its branch lacks. {{outcome}}

Commits on the member's branch that the lead's branch lacks:
{{commits}}

Files left uncommitted in the member's worktree:
{{files}}

Tell the member what to do with `agent say {{task}} "..."` and resume it with `agent resume {{task}}`, tell the lead to take the work with `agent say <lead task> "..."`, or stop the team with `agent stop {{team}}`.
