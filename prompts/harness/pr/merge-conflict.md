## Merge conflict on pull request #{{pr}}

Pull request #{{pr}} was approved by both reviewers, but GitHub refuses to merge it: it conflicts with `{{base}}`. This
is conflict pass {{pass}} of {{passes}} for this round. Do this on your branch `{{branch}}`, in your worktree:

1. Fetch the base: `git fetch origin {{base}}`.
2. Merge it into your branch: `git merge --no-ff origin/{{base}}`. Do not rebase and do not force push.
3. Resolve every conflict. Keep what your item asks for; take the base's side where the base is the newer one.
4. Raise the version of the workspace by `0.1.0` from the base's version, with `Cargo.lock` in the same commit, and add
   the entry for it at the top of `WhatsNew.md`, as the repository's rules say.
5. Run `agent gate`. It must pass at the tip of your branch, on a clean tree. If it fails, fix the cause and run it again.
6. Commit the merge and the version with a message that says the base was merged and why.

End with `STATUS: DONE`. If you cannot resolve a conflict, do not guess: end with `STATUS: QUESTIONS` and name the
files that conflict.
