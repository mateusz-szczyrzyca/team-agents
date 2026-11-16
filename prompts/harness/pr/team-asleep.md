# NOBODY IN YOUR TEAM IS WORKING (team {{team}}, woken for it {{woken}} of {{limit}})

Item {{items}} is not finished, and for {{minutes}} minutes every task of the team has slept: nobody works on it and
nobody will, because each waits for another. This is how the tasks ended:

{{tasks}}

You lead this team, so it is yours to get it moving. Look at the team (`team_status`) and at the branches, then do one
of these:

1. Do the remaining work yourself, on your branch.
2. Give a member a precise instruction: what to change, in which files, what the result must be. Check afterwards that
   it started to work — a member that woke, read its messages and went back to sleep did not take it.
3. If the work is finished, integrate it, get the repository's gate green and end your turn with `STATUS: DONE`.
4. If you cannot go on, put the question to the human and end with `STATUS: QUESTIONS`.

Do not end with `STATUS: SLEEPING` again unless a member is at work on something you can name.
