## Your plan

Keep the plan of your task in `.agent/plan.md` in your worktree. The harness reads it at the end of every run and keeps
each version. A run that ends with `STATUS: DONE` is sent back to you when the plan is missing, does not parse, has a
step still to do or in hand, or lost a step of its last version.

- Write the plan before your first change: what the task is for, its stages and its steps.
- Keep one step in hand (`[>]`) while you work on it, and mark it done (`[x]`) when it is done.
- Never delete a step or give its ID to another one: give a step up (`[-]`) with its reason on the line below.
- Do not commit the file: it is your working file, not part of what you deliver.

The form:

```text
What the task is for, in a sentence or two.

1. Draft
[x] 1.1 Write the outline
    Files: docs/guide.md
    Check: exists docs/guide.md
[>] 1.2 Write the first chapter
    Check: reviewer
[ ] 1.3 Check the examples
    Check: make check-examples
[-] 1.4 Translate the guide
    Nobody asked for a translation.
2. Publish
[ ] 2.1 Add the guide to the release
```

A stage is `N. Title` at the start of a line. A step is `[mark] N.M Title` at the start of a line, under its stage
`N`: `[ ]` to do, `[>]` in hand (at most one in the whole plan), `[x]` done, `[-]` given up. The indented lines under
a step are its text: at most one `Files:` line (paths from the repository's root, separated by commas), at most one
`Check:` line (a command, `exists PATH` or `reviewer`) and notes. A step need not be code: a section of a document or
an image is a step, and its check may be a file that must exist or a reviewer's judgement.
