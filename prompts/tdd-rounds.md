
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

## Keep tool output short

Everything a command prints stays in your context and is read again at every later step: a long output is paid
for many times. Use the quiet check target the repository offers (in team-agents: `make gate`), run single tests
while you work, send long output to a file and read only what you need (`grep -n`, `tail -n 40`). Do not print
whole logs, whole large files or full test listings.

## Rounds — every PR stays reviewable

The PR of this task {{ROUND_LIMIT_TEXT}}: added plus deleted lines against the base, tests included, generated and
vendored files not counted. Check it at any time: `{{ROUND_SIZE_CMD}}`. A task bigger than that is done in rounds:

- At the start, estimate the whole change. If it does not fit one round, plan rounds of about {{ROUND_SOFT}} lines —
  what each contains and in which order — and ask the human to approve the plan (a questions file, with your
  recommendation) before writing code. Keep the plan in your checkpoints, with the next round marked.
- After every round the base stays green: it builds, passes the linters and the tests. Unfinished functionality is
  dead code or a stub returning a typed "not implemented" error, reachable from nothing that works today.
- **No placeholder tests.** A test is written in the round that implements its behaviour (RED, then GREEN) — never
  earlier as a skipped placeholder. A stub of a later round has no test yet: nothing reaches it.
- **A skip is an exception**, for a reason outside the code (a missing tool or device, another operating system):
  the reason goes into the skip itself, and the test with its reason into a `## Skipped tests` section of the PR
  description. The script sends a round back while a skip it adds is missing there; a reason like "implemented in
  round N" is a major review finding.
- **Skipped tests already in the repository** are listed at the end of your prompt. Fill each one whose
  functionality exists now, test first. The round that implements a placeholder's functionality replaces it with a
  real test — the one exception to "existing tests are not yours to change".
- **No idea for a meaningful test?** Ask an investigator (`investigate`): what the code must do, what you would
  test. Do not skip, and do not write a test that checks nothing.
- End a round with its PR ready and `STATUS: ROUND_DONE N/M` (this round / all rounds); the last one may end with
  `STATUS: DONE`. The human merges the PR and resumes you: you then start from the merged base, on new branches.
- **Never sleep waiting for a merge.** If the human has to merge a PR before you can go on, that PR ends a round:
  end it with `STATUS: ROUND_DONE N/M` (raise M if this splits a planned round, and say so in the PR description).
  A team that sleeps with nothing running looks stuck, and the human gets an alarm instead of "ready to merge".
- A round over the limit, or adding skipped tests missing from the PR's `## Skipped tests` section, comes
  back to you.
- In a team, the coordinator plans and ends the rounds and keeps the integration branch under the limit before
  merging members' work; every member follows "Test first" and keeps its part small.
