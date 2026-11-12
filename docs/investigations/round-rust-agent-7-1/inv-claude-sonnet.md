# Item 7.1 — Starting work and the team's life (inv-claude-sonnet)

Report of the investigator `inv-claude-sonnet` for the round `rust-agent/7.1`. Branch
`agents/inv-claude-sonnet/20261112-213443`, no pull request (the merger integrates the independent results).

## Starting point

- The base of this task (`e8210e8`) predates item 6.1 (the daemon, its socket and the run engine), which the plan names as
  the dependency of feature 7 and which was merged afterwards
  ([#93](https://github.com/mateusz-szczyrzyca/team-agents/pull/93), `8f9f3c7`). The branch had no commits, so
  `origin/main` was merged by a fast-forward before any work; every commit below is on top of `8f9f3c7`.

## What was done

`agent run` and the team's life, as the item asks, in the files it names and a few more (listed under *Files*).

- **`agent run <agents...> "tasks" | -f file [--coordinator NAME] [--no-merge]`** (`crates/agent/src/commands/run.rs`).
  The repository is named by the `origin` of the checkout the command runs in; the words, the file, the parse of the task
  list, the repository and its origin are checked before the daemon is looked for; then `run` and `show` are asked and
  the team is printed: a line per task with its role, agent, worktree and item.
- **Starting a team** (`crates/agent/src/daemon/team/form.rs`, `crates/agent/src/daemon/owner/teams.rs`): `plan_team`
  (coordinator rules and refusals), IDs, the first items (`next_assignments`), one git worktree and branch per task
  (`agents/<agent>/<task>-<item>` or `agents/<agent>/<task>`), the team on the agentbus (a solo task has none). A failing
  worktree or bus takes back what was made.
- **The team's life** (`crates/agent/src/daemon/team.rs` and `team/`): the owner looks at a team after an event and
  every tick; a look sends what waits for the bus, reconciles the bus with the tasks (waking members the bus marked
  `waking`), gives pending items to free members (new branch in their worktree), serves requests for workers with the first
  free worker and returns the result to the requester, and closes a finished team once (waiting tasks end, bus team done,
  the human is notified). A member that ends tells the coordinator on the bus (`member_finished`, `member_failed`,
  `member_no_status`), which wakes it to integrate.
- **Texts**: `prompts/harness/team/` (11 files) in a catalog of their own, `TeamPrompt`, with the loader `TeamPrompts`.
- **Documentation**: `docs/architecture.md` (section "Teams"), `docs/state.md` (what the daemon serves).

## Item checks

| Check | Where it is shown |
|---|---|
| `agent run a "text"` starts a solo task | `team_process::agent_run_with_one_agent_and_a_text_starts_a_solo_task_which_runs_to_done` (the real binary) |
| `agent run a b -f list.md` starts a team whose coordinator follows the rules | `team_process::agent_run_with_two_agents_and_a_file_starts_a_team_led_by_the_investigator`; `team::form` tests (explicit coordinator, `Coordinator:` line, investigator, random normal) |
| a worker, or too many investigators, is refused before anything starts | `team_process::a_worker_as_coordinator_and_too_many_investigators_are_refused_before_anything_starts`; `daemon::owner::teams` refusal tests (each cause, and that no team, worktree or branch is left) |
| a member's DONE leads to the coordinator's integration and the next assignment, shown by `agent status` | `team_process::a_member_that_is_done_wakes_the_coordinator_which_agent_status_shows`; `daemon::owner::teams::a_member_that_is_done_tells_the_coordinator_…` and `a_free_member_is_given_the_next_item_when_the_daemon_looks_at_the_team_again` |

## Checks run

```sh
make gate                                    # gate: PASS (recorded at the last commit)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings      # 0 findings
cargo test --workspace --all-features --locked                                      # all green
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked  # 0 warnings
for i in $(seq 1 60); do cargo test -p agent --bin agent daemon::owner::teams; done  # 60/60 (21 tests then)
```

## Size

`3,326` changed lines outside tests by the PR-1 command (limit `5,000` for this plan); `4,319` lines of tests
(`16` files); `57` files in all.

## Commits

RED then GREEN for each part, tests first (the RED commits fail only on their new tests).

- [`1d99f00`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1d99f00a789a1f80be987d08d2dabe52ebbafb5d) RED — team decisions
- [`dce215c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/dce215c7bcb05d6c6cbc867f9812edaedf40a0b4) GREEN — forming, assigning, closing, bus sync, notices
- [`9d77653`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9d776530b25a4f564f54ff36f88bc9837db41ac2) RED — git: toplevel, origin URL, new branch
- [`3365327`](https://github.com/mateusz-szczyrzyca/team-agents/commit/33653271f3d30068730ad6d47e3a5b9118581c2e) GREEN — the same
- [`770f9bc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/770f9bc0fed760a06517007969bacb2160cc76db), [`f1caf7d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f1caf7debd635cb1f607479c9e817bd68957ea3b) RED — team texts and their loader
- [`a4b31b9`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a4b31b9b1126202a50a9dbbe130ecb0cfdabeb7a) GREEN — the loader
- [`d45bdb6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d45bdb600c79c951497cfc743e828695635fb0bc) RED, [`be90735`](https://github.com/mateusz-szczyrzyca/team-agents/commit/be907356013761d3c79498ad61f913c8b1ee011d) GREEN — the prompt of a team's run, the notices on the bus
- [`3397091`](https://github.com/mateusz-szczyrzyca/team-agents/commit/33970918930deac13eab0b7acf658c9483c3a391) RED, [`eb7bd41`](https://github.com/mateusz-szczyrzyca/team-agents/commit/eb7bd412b59f56231d47e988f445f35aadbf9340) GREEN — the daemon: `run` and the team's life
- [`82345c3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/82345c3fbd8ef46685a54e1e1f7784e84f479bfd), [`20675de`](https://github.com/mateusz-szczyrzyca/team-agents/commit/20675dee7b162f9ed91c89bb07fce549b83e825c) RED, [`afae7c6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/afae7c6b05ddad6f461902924bedc7e48c2103b6) GREEN — `agent run`
- [`9bb0d97`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9bb0d977e6d6f364e77ede2532f1f2f261e4feb6) process tests; [`e196eb0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e196eb02835fd625e10979740d85a369e8ab4ca6) documentation and error-path tests; [`484e870`](https://github.com/mateusz-szczyrzyca/team-agents/commit/484e8708f07988d564f3d393f7e1203a4f122f94) refactoring

## Files

Outside the item's list: `crates/agent/src/daemon/team/*` and `owner/teams.rs` (the item names `daemon/team.rs`; its
submodules are what it needed), `crates/agent-core/src/prompt/team.rs`, `crates/agent-runtime/src/team_prompts.rs`
(the loader), `crates/agent-runtime/src/git.rs` (`toplevel`, `origin_url`, `switch_new_branch`),
`crates/agent-runtime/src/run/compose.rs` (`item_sections` made public), `crates/agent/src/main.rs`,
`environment.rs`, `daemon/mod.rs`, `daemon/owner.rs`, `daemon/owner/start.rs`, `daemon/notice.rs`, `daemon/settle.rs` (the
wiring), tests and two fixture scripts under `crates/agent/tests/fixtures/`, the two docs.

## Deviations (decisions a reviewer should look at first)

1. **`run` is not a variant of `Command`.** `crates/agent/src/tests/commands.rs` (6.1) matches `Command` exhaustively
   (`Some(Command::Daemon(_)) | None => panic!`), so a new variant does not compile without editing that test, which rule 1
   forbids. `run` is added to the clap command when the command line is parsed (`parse`, `Parsed::Run`); `--help` lists it.
   **Suggested change for the human:** allow the one-line change of that test (add `| Some(Command::Run(_))`), then
   `Run(RunArgs)` can become a normal variant and `parse`/`Parsed`/`Work` go away.
2. **The team texts are a catalog of their own (`TeamPrompt`)**, not entries of `HarnessPrompt`: the existing test
   `harness_prompt_catalog::every_file_of_the_run_directory_is_in_the_catalog` compares the whole `HarnessPrompt` catalog
   with the files of `prompts/harness/run/`, so an entry under `harness/team/` fails it. Cost: ~150 lines for a second loader.
3. **Every task of a team starts at once** (the coordinator first): there is no "held" member. I first built one (a member
   with nothing to do stays queued until it is given something), but the Go bus posts a `team_created` message that every
   member has unread, and `team-set --state sleeping` wakes a member that has unread messages, so a member that never ran
   was woken at once. The old harness started everybody too. The context text tells every member to read its messages first.
4. **Bus failures are specified fallbacks, not ignored errors**: what cannot be sent stays in an outbox and is tried
   again at the next look (tested with a bus that is down for a while); a transient failure is `warn`, a refusal for good is
   logged at `error` and dropped so that it does not block what is behind it (`is_transient`, tested by class).
5. A few strings of reasons ("the worker ended with STATUS: FAILED", ...) are in `.rs` files: they are values of a
   template's variable (`bus-helper-failed.md`), short and factual, in the way 6.1's notices pass `exit`. If the rule must
   be absolute, they become templates.
6. No lint suppression (`#[expect]`/`#[allow]`), no new dependency, no skipped test, no change of a persisted format or of
   the protocol (the `run` request and the `started` reply existed since 2.2).
7. Three tests added after their code, as error paths found while testing the flows (`is_transient`, the item whose branch
   cannot be started, the unit test of `item_in_flight`); the branch-failure test was checked to fail with the code mutated.

## Not done / known limits

- **Wake loops.** An agent that never reads its messages is woken again and again (the bus keeps its messages unread).
  The Bash wake-loop guard is not ported; the settings doc says it comes back "if the new design needs it". Worth a decision
  before the cutover.
- A request for a worker that is being served when the daemon stops is not returned (the daemon keeps the map in memory;
  the bus has no listing of requests to rebuild it from). Notices still in the outbox at shutdown are lost.
- `helper-next` gives the oldest request of every team to whichever team's worker asks: a request of another team goes
  back to the queue, and what is behind it waits until its own team has a free worker.
- A member's item stays `in_progress` until its pull request is open (8.1); so after a member's DONE the next item of its
  feature does not start (the plan's rule: after the merge). The "next assignment" is exercised with a state in which the
  item is `in_review` (what 8.1 produces).
- The gate (7.3), pull requests, reviews and the merge (8.1), answers and controls (7.2), the checkpoint of a resumed agent
  are not in this item.

## Suggestions for `AGENTS.md` (not changed)

- Rule 1 and an exhaustive `match` on a CLI enum in a test: say that a new command may add its variant to such a test's
  pattern, or ask tests to use a wildcard arm — as written, every new command needs the human.
- The catalog test of `harness/run/` has the same shape of problem for every new group of texts.

## For the notebook

(`notebook_append` is not available in this phase.)

- The Go bus wakes a sleeping member only by marking it `waking` and spawning `team.WakeCmd` (empty here): the daemon must
  poll `team-get`.
- A member that never ran has the bus's `team_created` message unread: `team-set --state sleeping` returns `waking` for it.
- `agentbus helper-next` ignores teams; `helper-set --summary ""` leaves the summary as it is.
- The tests that start the daemon twice in one process must retry on `AlreadyRunning` for a few milliseconds (a forked
  child holds the released lock until its exec).
