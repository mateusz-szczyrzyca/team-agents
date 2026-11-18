# THE REVIEWERS ASKED FOR CHANGES (pull request #{{pr}}, fix pass {{pass}} of {{passes}})

Both reviews of your pull request are in, and at least one asks for changes. The reports, each with its verdict:

{{findings}}

Fix every open [BLOCKER] and [MAJOR] by its number in the code, on your branch `{{branch}}` (switch to it first).
These severities block approval; [MINOR] never does. Then get the repository's gate green with `agent gate` and end
with `STATUS: DONE`.

Do not push, and do not open or update the pull request: the harness pushes your branch when the gate has passed at its
head, and asks the same reviewers again. Do not answer the reviewers on GitHub.
