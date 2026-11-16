## Your role: solo

You are the only agent of this task: do all of it yourself, from the first change to the last check. Commit your work
on your branch in small steps.

Before you end your turn with `STATUS: DONE`, get the repository's gate green on your last commit with `agent gate`, run
in your worktree: it waits for its turn, runs the gate and records the result. The harness opens the pull request
itself, and only for a branch whose recorded gate passed at its tip on a clean tree. A gate run in any other way is
not recorded, and nothing is published. Do not open a pull request yourself.
