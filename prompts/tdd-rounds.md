
## Test first

Every change of behaviour starts with a test that fails for the right reason:

1. **RED** — write the test first. In Go, add the function's signature with a body that returns zero values, so
   the test compiles and fails on an assertion (a compile error proves nothing about the behaviour). Run it and
   commit the test (with the bare signature) on its own; the commit message ends with the failing run:
   `RED: <the test command>` and the FAIL lines.
2. **GREEN** — the simplest code that makes it pass. Commit it with `GREEN: <the command>` and the passing lines.
3. **REFACTOR** — better code, the tests unchanged and still passing.

Where a test first is not possible (layout, prompt text, a line of configuration), the commit message says why:
`Test-First: not possible — <reason>`. A change of behaviour with neither a test commit before it nor that line is
a major review finding. The rules in detail, with a worked example: `docs/tdd.md` of the team-agents repository.

## Rounds — every PR stays reviewable

The PR of this task {{ROUND_LIMIT_TEXT}}: added plus deleted lines against the base, tests included, generated and
vendored files not counted. Check it at any time: `{{ROUND_SIZE_CMD}}`. A task bigger than that is done in rounds:

- At the start, estimate the whole change. If it does not fit one round, plan rounds of about {{ROUND_SOFT}} lines —
  what each contains and in which order — and ask the human to approve the plan (a questions file, with your
  recommendation) before writing code. Keep the plan in your checkpoints, with the next round marked.
- After every round the base stays green: it builds, passes the linters and the tests. Unfinished functionality is
  dead code or a stub returning a typed "not implemented" error, reachable from nothing that works today.
- Tests of a later round's functionality may come earlier, skipped with a marker naming the round:
  `t.Skip("pending: <what>, round N")` (other languages: the same text as the skip reason). Round N removes them;
  the script checks it.
- End a round with its PR ready and `STATUS: ROUND_DONE N/M` (this round / all rounds); the last one may end with
  `STATUS: DONE`. The human merges the PR and resumes you: you then start from the merged base, on new branches.
- **Never sleep waiting for a merge.** If the human has to merge a PR before you can go on, that PR ends a round:
  end it with `STATUS: ROUND_DONE N/M` (raise M if this splits a planned round, and say so in the PR description).
  A team that sleeps with nothing running looks stuck, and the human gets an alarm instead of "ready to merge".
- A round over the limit, or with its own pending tests left, comes back to you to be split.
- In a team, the coordinator plans and ends the rounds and keeps the integration branch under the limit before
  merging members' work; every member follows "Test first" and keeps its part small.
