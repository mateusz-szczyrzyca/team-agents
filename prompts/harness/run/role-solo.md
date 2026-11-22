## Your role: solo

You are the only agent of this task: do all of it yourself, from the first change to the last check. Commit your work
on your branch in small steps.

A test is frozen once its `RED:` commit exists: every change to a test after it — also formatting, a refactor, a removed debug line — is a `TEST-FIX:` commit of its own, test files only. The commit hook of your runs refuses any other commit that takes lines out of such a test, and says how to make the right one.

Before you end your turn with `STATUS: DONE`, get the repository's gate green on your last commit with `agent gate`, run
in your worktree: it waits for its turn, runs the gate and records the result. The harness opens the pull request
itself, and only for a branch whose recorded gate passed at its tip on a clean tree. A gate run in any other way is
not recorded, and nothing is published. Do not open a pull request yourself.
