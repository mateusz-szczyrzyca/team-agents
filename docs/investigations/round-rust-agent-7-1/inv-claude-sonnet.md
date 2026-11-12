# Item 7.1 — Starting work and the team's life (inv-claude-sonnet)

Working notes and the final report of the investigator `inv-claude-sonnet`. Updated as the work goes on; the last
section is the report.

## Starting point

- The base of this task (`e8210e8`) predates item 6.1, which the plan names as a dependency and which was merged
  afterwards ([#93](https://github.com/mateusz-szczyrzyca/team-agents/pull/93)). `origin/main` was merged into the branch
  before any work (merge commit, no conflicts) so that the daemon, its socket and the run engine exist.

## Design (decisions taken before coding)

1. **Decide, then carry out.** The team's life is pure functions over the state (`daemon/team/*.rs`: formation,
   assignment, the bus mapping, closing, reactions to a run's end), which the owner's glue (`daemon/owner/teams.rs`)
   carries out: git, the bus, the state, events, notifications.
2. **The bus is reconciled, not told.** The Go bus wakes a sleeping member by setting its state to `waking` and spawning a
   wake command; the Rust wrapper configures none, so the daemon reads the bus (`team-get`) on its tick and after every
   change, derives each member's bus state from its task's status, sets what differs and queues the task of every member
   the bus says is `waking`. A bus that fails is retried at the next tick (a specified fallback, with a test), never
   ignored. Notices for the coordinator (`member_finished`, ...) wait in an outbox until they are sent.
3. **Held members.** A member or worker that has never run and has no item stays `queued` but is not started: it runs
   when it is given an item, woken by the bus, or given a delegated request. This is derived from the state (no runs, no
   item, role member or worker), so it survives a restart; no new status is needed.
4. **Delegation** is served by polling: for the first free worker (`first_free_worker`) the daemon calls `helper-next`;
   the request goes to that worker's task, `helper-set` records `running`, and the worker's final message goes back as
   `helper-set --status done --summary ...` and a `helper_result` notice to the requester.
5. **Closing**: a team is finished when its items are all done (a list without items: when its lead ended with `DONE`) and
   no task is running, asking, paused or about to start. Closing ends the waiting tasks, marks the bus team done and
   notifies the human, once.
6. **Solo tasks have no bus team.** Only `Crew::Team` teams create one.
