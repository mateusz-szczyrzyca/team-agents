# OPEN FINDINGS TO VERIFY

Run every supplied check and keep the numbers. Before new findings, answer every open number exactly once.
For a pending explanation use `FINDING 4: ACCEPTED — why` or `FINDING 4: REJECTED — why`.
For other findings use `FINDING 3: RESOLVED` or `FINDING 3: OPEN — why`. After RESOLVED, optionally add ` — reason`, ` - reason`, or `: reason`.
You may raise severity with `FINDING 6: RAISED [MAJOR] — why` or `[BLOCKER]` in that position.
Never lower severity yourself. Do not raise an accepted explanation unless the relevant code changed.
An explained MAJOR becomes MINOR only if every verifier accepts; rejection requires a code fix.
Do not approve with a rejected explanation, raised blocking finding, or any open BLOCKER or MAJOR.
New findings go under FINDINGS in the review format. If evidence is unavailable, REJECT a pending explanation and leave other findings OPEN.

{{findings}}
