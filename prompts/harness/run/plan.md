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
    For: R1
    Check: exists docs/guide.md
[>] 1.2 Write the first chapter
    For: R1, K1
    Check: reviewer
[ ] 1.3 Check the examples
    For: K1
    Check: make check-examples
[-] 1.4 Translate the guide
    Nobody asked for a translation.
2. Publish
[ ] 2.1 Add the guide to the release
    For: R2
```

A stage is `N. Title` at the start of a line. A step is `[mark] N.M Title` at the start of a line, under its stage
`N`: `[ ]` to do, `[>]` in hand (at most one in the whole plan), `[x]` done, `[-]` given up. The indented lines under
a step are its text: at most one `Files:` line (paths from the repository's root, separated by commas), at most one
`For:` line (the numbers of the requirements of your item that the step serves, separated by commas), at most one
`Check:` line (a command, `exists PATH` or `reviewer`) and notes. A step need not be code: a section of a document or
an image is a step, and its check may be a file that must exist or a reviewer's judgement.

The requirements of your item are numbered: `R1`, `R2`, ... for its expected result and `K1`, `K2`, ... for its checks;
an item without an expected result has one requirement, `R1`, its title. Every step that is not given up names in its
`For:` line the requirements it serves, and the harness sends your plan back when a step names none or names one that
your item does not have. When your plan has `For:` lines, every commit message ends with a line `Step: N.M` that names
the step of the plan the commit belongs to; a commit without it is refused.
