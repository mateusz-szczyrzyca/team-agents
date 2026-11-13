# A PULL REQUEST TO REVIEW (team {{team}})

You review pull request #{{pr}} of a team you do not belong to, as the review rules above say. This is review {{round}}
of it. Its head is the commit `{{head}}` of the branch `{{branch}}`, and your worktree has exactly that commit out. It
merges into `{{base}}`. The item it is for, with its expected result and its checks, follows below: judge the pull
request against it, and check that nothing the item does not ask for is in it.

- Start with `git diff {{base}}...HEAD`. Run the repository's gate with `agent gate`: it waits for its turn on this
  machine and prints its verdict.
- You change, commit and push nothing, and you post nothing on GitHub: the harness posts your report as the review.
- Your final message is the report in the format of the review rules. Its **first line** is `VERDICT: APPROVE` or
  `VERDICT: REQUEST_CHANGES`, one of the two, and its last line is `STATUS: DONE`:

```
VERDICT: REQUEST_CHANGES
SUMMARY: ...
SIZE: ...
GATE: ...

BLOCKERS
1. [ID] file:line — problem → fix

STATUS: DONE
```

If you cannot review the pull request, say why and end with `STATUS: FAILED`.
