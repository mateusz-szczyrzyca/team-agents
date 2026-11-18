{{member}} failed on item {{item}} (branch `{{branch}}`): its run ended with `STATUS: FAILED`. What it said:

{{message}}

The harness publishes an item only while its assignee task waits. If you take over the work, still report `STATUS: DONE`: the harness will tell the human, the human will resume {{member}}, and {{member}} must end with a status. Or ask the human (a questions file, `STATUS: QUESTIONS`).
