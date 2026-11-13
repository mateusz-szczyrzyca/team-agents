# THE REVIEWERS ASKED FOR CHANGES (pull request #{{pr}}, fix pass {{pass}} of {{passes}})

The reviewers of your pull request asked for changes. Their reports:

{{findings}}

Fix every blocker, on your branch: do it yourself or give it to a member, as you see fit. A finding you think is wrong
gets a reason in your commit message, not silence. Then get the repository's gate green with `agent gate` and end your
turn with `STATUS: DONE`.

Do not push, and do not open or update the pull request: the harness pushes your branch when the gate has passed at its
head, and asks the reviewers again. Do not answer the reviewers on GitHub.
