# INVESTIGATION RESULT #{{INV_ID}}

The investigator finished with status **{{INV_STATUS}}**.
- Its branch with any experiments: `{{INV_BRANCH}}` (local, in the same repository — you can inspect it:
  `git log {{BRANCH}}..{{INV_BRANCH}}`, `git diff`, and possibly `git cherry-pick`).
- The full investigation record: `{{INV_FILE}}`.

Now:
1. Start with `read_messages` and `set_status`.
2. Assess the result critically. If you can, verify the key findings yourself (investigators make mistakes too).
3. BEFORE acting on it, record `record_decision(investigation_id={{INV_ID}}, decision, justification)`:
   what you will do and why — including what you verified.
4. Continue task `{{TASK_ID}}` following the rules from the first message of this session.

If the result raises new ambiguities that the human must settle, ask in a NEW file `{{QUESTIONS_FILE}}`
(round {{ROUND}}) and end your turn with `STATUS: QUESTIONS`.

The investigator's result:
