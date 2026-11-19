# OPEN FINDINGS TO VERIFY

Run every supplied check and keep the numbers. Before new findings, answer every open number exactly once, with one of
the answers its `ANSWER:` line names, as `FINDING 3: <answer>` on one line: ACCEPTED or REJECTED for an explanation that
waits for your decision, RESOLVED or OPEN for the others, RAISED only where the line offers it. Any other answer is
refused. After RESOLVED, optionally add ` — reason`, ` - reason`, or `: reason`.
Never lower severity yourself. Do not raise an accepted explanation unless the relevant code changed.
An explained MAJOR becomes MINOR only if every verifier accepts; rejection requires a code fix.
Do not approve with a rejected explanation, raised blocking finding, or any open BLOCKER or MAJOR.
New findings go under FINDINGS in the review format. If evidence is unavailable, REJECT a pending explanation and leave other findings OPEN.

{{findings}}
