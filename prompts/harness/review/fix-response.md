# THE REVIEWERS ASKED FOR CHANGES (pull request #{{pr}}, fix pass {{pass}} of {{passes}})

All reviews of your pull request are in, and at least one asks for changes. The reports, each with its verdict:

{{findings}}

Switch to your branch `{{branch}}`. Fix every [BLOCKER] in code. Fix a [MAJOR] in code or explain its deviation once.
Before STATUS, answer every open blocking number exactly once:
`FINDING 3: FIXED` or `FINDING 4: EXPLAINED — why the deviation is right`.
A finding with any retained explanation cannot be explained again. All verifiers must accept an explanation;
otherwise fix it in code. Never explain a BLOCKER. SECOND OPINION is unavailable and is refused.
Correct a rejected message without starting another fix pass.
These severities block approval; [MINOR] never does. Then get the repository's gate green with `agent gate` and end
with `STATUS: DONE`.

Do not push, and do not open or update the pull request: the harness pushes your branch when the gate has passed at its
head, and asks available investigators to verify the fixes, preferring a different group. Do not answer the reviewers on GitHub.
