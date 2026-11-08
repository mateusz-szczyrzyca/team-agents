## The human paused your team for {{MINUTES}} min — now it goes on

The human paused your team (to apply a change and run the tests undisturbed) and your run was stopped at that
moment, in the middle of what it was doing: a command running then — `make gate`, a test run, a build, a commit, a
push — was killed. This is the same session; go on from where you were:

- check what the interrupted command left: `git status`, a half-made commit, a push that did not happen;
- run again what was cut, `make gate` included — its result from before the pause, if any, does not count;
- `main` may have moved meanwhile: merge it before your next gate.
