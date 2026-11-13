# THE ROUND IS NOT READY FOR ITS REVIEW (pull request #{{pr}}, check {{pass}})

The harness checked the round's own diff and its commits before the review ({{passes}} such returns are allowed before the human is asked) and found:

{{reasons}}

{{refused}}
Settle every finding on your branch, as the repository's rules ask: tests first, a commit for each step. Then get the repository's gate green with `agent gate` and end your turn with `STATUS: DONE`. The harness checks the round again, and when nothing is left it asks the reviewers.

Do not push, and do not open or reopen the pull request: the harness pushes your branch when the gate has passed at its head. Where a finding asks for a change of the description of the pull request, make it there; the description is read again with the round.
