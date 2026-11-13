# Item 8.1 — Pull requests, reviews and the merge loop (`inv-claude-sonnet`)

Work in progress: this file is rewritten into the final report when the item is done.

## Design decisions (taken while reading the code)

- **No new persisted state beyond one core operation.** Everything the autopilot needs to know is derived from the state
  and the files of the home at every look at a team, so that it survives a restart: the round of an item from its phase
  (`ItemPhase`) and fix passes, "reviews asked" from the pending `Review`s, "a fix pass was delivered" from the lead's
  runs against the time of the review that asked for it, "the human was asked" from the lead's questions.
- **The pull request is the coordinator's branch** (the solo agent's item branch for a solo task): the coordinator merges
  the members' branches into its own, gets the gate green (`tasks/<coordinator>/gate.json`, 7.3), and the daemon publishes
  when the record passes on a clean tree at the commit the worktree has out and that commit contains the item's branch.
- **Items integrated into one head share the pull request** (parallel features): they move through the phases together.
- **Reviewers**: a reviewer is a task of the team (`Team::reviewers`), one per investigator. It ends its review `waiting`
  (not `done`), so that the same investigator can review the next pull request and the re-review after a fix pass.
- **Fix passes go to the lead** (the coordinator; a solo agent itself): it has the integrated branch, so the pull request
  is updated by the same branch.
- Reviews are posted as comment reviews (the harness opens the pull request and posts the reviews with one account).
