# THE REVIEWERS ASKED FOR CHANGES (pull request #{{pr}}, fix pass {{pass}} of {{passes}})

Both reviews of your pull request are in, and at least one asks for changes. The reports, each with its verdict:

{{findings}}

Fix every blocker of every report, on your branch `{{branch}}` (switch to it first when your worktree has another one
out): do it yourself or give it to a member, as you see fit. A report that approves may still name things worth
fixing now. A finding you think is wrong
gets a reason in your commit message, not silence. Then get the repository's gate green with `agent gate` and end your
turn with `STATUS: DONE`.

Do not push, and do not open or update the pull request: the harness pushes your branch when the gate has passed at its
head, and asks the same reviewers again. Do not answer the reviewers on GitHub.
