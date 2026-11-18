# A PULL REQUEST TO REVIEW (team {{team}})

You review pull request #{{pr}} of a team you do not belong to, as the review rules above say. This is review pass {{pass}}
of it. Its head is the commit `{{head}}` of the branch `{{branch}}`, and your worktree has exactly that commit out. It
merges into `{{base}}`. The item it is for, with its expected result and its checks, follows below: judge the pull
request against it, and check that nothing the item does not ask for is in it.

- Start with `git diff {{base}}...HEAD`. Run the repository's gate with `agent gate`: it waits for its turn on this
  machine and prints its verdict.
- You change, commit and push nothing, and you post nothing on GitHub: the harness posts your report as the review.
- Your final message is the report in the format of the review rules. Its **first line** is `VERDICT: APPROVE` or
  `VERDICT: REQUEST_CHANGES`, one of the two, and its last line is `STATUS: DONE`:

```text
VERDICT: REQUEST_CHANGES
SUMMARY: ...
SIZE: ...
GATE: ...

FINDINGS
1. [MAJOR] ERR-3 crates/x/src/y.rs:42 — problem → fix
   CHECK: the command or lines that show whether the problem remains

STATUS: DONE
```

If you cannot review the pull request, say why and end with `STATUS: FAILED`.

The bracket contains only the severity: [BLOCKER], [MAJOR] or [MINOR]; put the rule ID after it.
BLOCKER means broken logic, build or gate. MAJOR means an unexplained specification deviation. MINOR is advice.
APPROVE requires no open BLOCKER or MAJOR. REQUEST_CHANGES requires at least one.
