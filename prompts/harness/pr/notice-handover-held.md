The pull request of {{team}} (items {{items}}) could not be published. Round {{rounds}} waits for your instruction.
{{reason}}

Run `agent say {{task}} <instructions>` to return the lead to the fix. Require a committed fix and `agent gate` before `STATUS: DONE`. If the lead does not take it up, run `agent renew {{task}}`: its next run starts a new session that reads your message.
