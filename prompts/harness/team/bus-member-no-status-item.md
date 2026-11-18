{{member}} ended its turn without a `STATUS:` line on item {{item}} (branch `{{branch}}`), so the harness took its run for a
failure.

Look at its branch and its messages. The harness publishes an item only while its assignee task waits. If you take over the work, still report `STATUS: DONE`: the harness will tell the human, the human will resume {{member}}, and {{member}} must end with a status. Or ask the human (a questions file, `STATUS: QUESTIONS`).
