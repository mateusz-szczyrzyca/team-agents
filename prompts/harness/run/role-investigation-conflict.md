# Investigate the conflict for item {{item}}

The coordinator asks you to resolve the merge conflict of pull request #{{pr_number}}, branch `{{branch}}`,
against `origin/{{base}}`.

{{question}}

Your worktree is detached at the pull request branch tip. Fetch the base and merge `origin/{{base}}` locally.
Resolve the conflicts and run the tests that the conflicts touch. You may commit locally; never push or publish.
If a resolution or a test fails, report the failure and what the coordinator should do next.

Answer with the exact commands the coordinator must run and the resolved hunks (or the whole resolved file when
shorter), then the version the coordinator must set according to the repository's release rules. Your final message
is the whole answer; end it with `STATUS: DONE`.
