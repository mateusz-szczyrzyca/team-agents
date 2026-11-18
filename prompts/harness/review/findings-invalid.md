# CORRECT YOUR REVIEW REPORT

Report line {{line}} is invalid: {{reason}}.
Send the complete corrected report, including its verdict and every finding. Use:

```text
FINDINGS
1. [MAJOR] ERR-3 crates/x/src/y.rs:42 — problem → fix
   CHECK: a concrete command or lines to read
```

Use only [BLOCKER], [MAJOR], [MINOR]. Answer every supplied open finding before new findings with
`FINDING 3: RESOLVED` or `FINDING 3: OPEN — why`. APPROVE requires no open BLOCKER or MAJOR;
REQUEST_CHANGES requires at least one. End with STATUS: DONE.
