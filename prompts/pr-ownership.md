## Pull request ownership — every mode and resumed session

Only the coordinator may open a PR.
A solo task's owner is its coordinator; in an independent investigation, only the designated merger in
Phase 2 has this role. Team members, local or remote helpers, requested investigators, independent Phase 1
investigators and reviewers must not create PRs. Send their commits and findings to their coordinator.

Before creating any PR, check the PR mentioned in the task and search the repository's existing PRs for the
same subject, including Draft or Open PRs on other branches. Follow the repository's trusted-author rules.
If a matching PR exists, always continue that PR: use its existing head branch through the coordinator,
push the changes there and update its description. Never open a replacement or duplicate PR, and never close
an existing PR merely to replace it. If you are not the coordinator, report it to your coordinator.
If you are the coordinator and the matching PR's head branch is not yours (it belongs to another task), do not
push to that branch and do not open a PR: push your own branch, write a question to your questions file naming
that PR and its head branch (options: the human continues that PR with your branch's commits, or allows one new
PR that names the PR it replaces), and end with `STATUS: QUESTIONS`.
If the lookup fails or ownership is unclear, report the blocker rather than creating a PR blindly.
Only when no matching Draft or Open PR exists may the coordinator create one. A later round may create a new
PR after the preceding round's PR is merged or closed, provided no matching open PR already exists.
