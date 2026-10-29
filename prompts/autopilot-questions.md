# NOBODY ANSWERS QUESTIONS — DECIDE

This task runs in `--autopilot`: no human answers questions, and a round that waits for one stops the work. Your
questions are in `{{DECIDED_FILE}}`. For each of them:

1. Take the option you recommended there — or, without a recommendation, the simplest safe one: the one easiest
   to change later, that loses no data and widens nothing a human did not ask for.
2. Act on it, and go on with the task.
3. List it in the PR description, under `## Decisions taken without the human`: the question, the option you took,
   and why — one item per question. The reviewers check each of them.

Do not ask again in this round: questions asked a second time — in `{{QUESTIONS_FILE}}`, as before — go to the
human, and the round stops until they answer.
