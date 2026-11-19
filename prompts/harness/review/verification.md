# VERIFY THE FINDINGS (team {{team}})

Verify pull request #{{pr}}. This is verification pass {{pass}}, at commit `{{head}}` of branch `{{branch}}`, merging
into `{{base}}`. Your worktree has that head out. Verify every open finding below using its CHECK and judge the changes
since the preceding review. Add any new defect you discover, including one missed before. Do not repeat a full review.

{{changes}}

Follow the harness gate instructions below. Change, commit, push and post nothing: the harness publishes your report.
You may consult other verifiers through the bus.

# THE LEAD'S FINAL FIX MESSAGE

{{lead_message}}

Start your final report with `VERDICT: APPROVE` or `VERDICT: REQUEST_CHANGES`. Before new findings, answer for every open
finding: `FINDING 3: RESOLVED` or `FINDING 3: OPEN — why`. APPROVE requires no open BLOCKER or MAJOR, old or new;
REQUEST_CHANGES requires at least one. MINOR never blocks approval. Use this format for new findings; the harness assigns
permanent numbers:

```text
FINDINGS
1. [MAJOR] ERR-3 crates/x/src/y.rs:42 — the problem → the fix
   CHECK: the command or lines that show whether the problem remains
```

The bracket holds only BLOCKER, MAJOR or MINOR; the rule ID follows it. End with `STATUS: DONE`. If verification is
impossible, explain why and end with `STATUS: FAILED`.
