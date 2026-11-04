<!-- part: heading -->
## Pull request ownership

<!-- part: coordinator -->
**Only the coordinator may open a PR.** That is a solo task's agent, or a team's coordinator. A task has one PR
for all its work.

<!-- part: coordinator known -->
**This task's PR is {{TASK_PR}}.** Continue it: push this task's commits to its branch and keep its title and
description up to date (`gh pr edit {{TASK_PR_NUMBER}}`). Never open another PR for this task, and never close this
one to replace it: the agents' `gh` refuses `gh pr create` while it is open. Only once the human has merged or closed
it may the next round open a new one, as follows.

<!-- part: coordinator none -->
No PR of this task is known yet.

<!-- part: coordinator -->
Before you open a PR, look for this subject's: your branch's (`gh pr list --head {{BRANCH}}`), the PR the task
mentions, and the repository's Draft or Open PRs on the same subject, on other branches too (`gh pr list`). If there
is one, continue it and open none. If its head branch is not yours (another task's), do not push to that branch and
do not open a PR: push your own branch, write a question to your questions file naming that PR and its head branch
(options: the human continues that PR with your branch's commits, or allows one new PR that names the PR it
replaces), and end with `STATUS: QUESTIONS`. If the lookup fails, ask the same way instead of opening a PR blindly.
Only when there is none, open **one draft PR** after your first meaningful commit:
`gh pr create --draft --base {{BASE}} --head {{BRANCH}}`.

<!-- part: member -->
**Only the coordinator may open a PR.** You open none, and the team's task has one, the coordinator's: push your own
branch and tell the coordinator what is on it (a board item or a message); it merges your work into that PR.

<!-- part: member known -->
**This task's PR is {{TASK_PR}}.**

<!-- part: member none -->
The coordinator opens it, as a draft, once the work is split.
