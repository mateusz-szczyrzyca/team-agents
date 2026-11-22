This commit was not made. A test is frozen once its `RED:` commit exists (TDD-14), and the commit you asked for changes frozen tests:

{{findings}}

Nothing is lost: your changes are still staged. Make the commit the rule allows:

1. A change to a frozen test is a `TEST-FIX:` commit of its own — also formatting, a refactor, a removed debug line. It holds test files only, its subject starts with `TEST-FIX:` and its message says what was wrong. Unstage every file that is not a test (`git restore --staged <file>`), then run `git commit -m "TEST-FIX: <what was wrong>"`.
2. Stage the rest again and commit it as a commit of its own, with its own message.
3. A change you do not need is put back: `git restore --staged <file>`, then `git checkout -- <file>`.

Do not use `--no-verify`: the checks before the review find the same commit afterwards, when it can no longer be undone. Never rewrite a commit that is already pushed; this refusal is about the commit you are making now.
