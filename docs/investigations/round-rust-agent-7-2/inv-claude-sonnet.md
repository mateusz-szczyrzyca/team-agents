# Item 7.2 — Questions, answers and controls (inv-claude-sonnet)

Investigator `inv-claude-sonnet` (claude/sonnet), task `20261113-075726-inv-claude-sonnet`, branch
`agents/inv-claude-sonnet/20261113-075726`, base `28d06aeb`. Independent result for the merger of Phase 2; no pull
request was opened.

## Result

Item 7.2 is implemented, test-first (RED and GREEN commits), documented and pushed. The Rust gate is green **except for
one existing test that the item itself makes wrong** — see "Needs the human" first.

## Needs the human

**One existing test fails because the item makes it wrong, and I did not change it (AGENTS.md rule 1).**

`daemon::tests::commands_the_daemon_does_not_serve_yet_are_refused_with_their_name`
([`crates/agent/src/daemon/tests.rs`](../../../crates/agent/src/daemon/tests.rs)) uses `stop` as its example of a command
the daemon "does not serve yet" and expects `refused`. Item 7.2 serves `stop`, so the daemon now answers `applied`. The
test's intent (a command that is not served yet is refused with its name) still holds for `gate`, which item 7.3 serves.

With this change, which I applied on a scratch copy and did **not** commit, `cargo test --workspace`, clippy `-D warnings`,
`cargo fmt --check` and `cargo doc` are all green (`0` failures):

```diff
--- a/crates/agent/src/daemon/tests.rs
+++ b/crates/agent/src/daemon/tests.rs
@@ -818,7 +818,5 @@ async fn commands_the_daemon_does_not_serve_yet_are_refused_with_their_name() {
         .request(
             Client::Cli,
-            Command::Stop(agent_core::protocol::ControlRequest {
-                target: Target::Task(task(1)),
-            }),
+            Command::Gate(agent_core::protocol::GateRequest { task: task(1) }),
         )
         .await
@@ -826,5 +824,5 @@ async fn commands_the_daemon_does_not_serve_yet_are_refused_with_their_name() {
 
     assert!(
-        matches!(&error, ClientError::Refused(failure) if failure.kind == FailureKind::Refused && failure.message.contains("stop")),
+        matches!(&error, ClientError::Refused(failure) if failure.kind == FailureKind::Refused && failure.message.contains("gate")),
         "{error:?}"
     );
```

The merger (or the human, `agent allow`) decides whether to apply it. The same applies to a second existing test that
the item could have broken and did not: `tests::help_goes_to_stdout_and_succeeds` — I changed the code, not the test (the
description of `agent` in its help is kept when the commands are added).

## Task (item 7.2, in full)

> 7.2 Questions, answers and controls. A run ending with questions writes the next questions file and notifies the human;
> it opens in VS Code when configured. Answers arrive by `agent answer <id>` (the editor, or `--file`), by saving the file
> with the ANSWERED marker (watched with `notify`), or from the dashboard; the task resumes with them. `agent stop`,
> `pause`, `unpause`, `resume` and `say` for a task or a team; state and activity show what was done and by whom.
> Expected result: Files `crates/agent/src/commands/{answer,stop,pause,unpause,resume,say}.rs`,
> `crates/agent/src/daemon/questions.rs`, the questions-file and notification texts under `prompts/harness/questions/`.
> Adds: the questions lifecycle and the controls. Tests: a questions file written and the task resumed after answers from
> `agent answer --file`, from a saved file and from the protocol; each control's effect on state and activity.
> Checks: a fake CLI ending with questions leaves a questions file and a notification file; `agent answer <id> --file
> answers.md` resumes the task with the answers in its next prompt. `agent pause`, `unpause` and `stop` change the status
> `agent status` shows, and the activity names who did it.

## What was built

| Part | Where | What |
|---|---|---|
| Texts | `prompts/harness/questions/{answered,notice-incomplete}.md` (with the two that existed) | what a task is told after its answers; the notification of answers marked done but not complete |
| Catalog | `agent-core` `prompt/questions.rs` (`QuestionsPrompt`), `agent-runtime` `questions_prompts.rs` | a fourth prompt catalog and its loader, checked at the daemon's start like the others (every problem at once) |
| Configuration | `agent-core` `config.rs`, `config/agents.example.toml`, `docs/configuration.md` | `[questions] open = "off" \| "vscode"`, `vscode_server_dirs`, `editor` |
| Activity | `agent-core` `cli.rs` (`ActivityEvent::Control`), `agent-runtime` `store.rs` | `{"at","kind":"control","action","by":{client}}` |
| Questions file | `daemon/questions.rs` | `read`, `apply` (answers into a file, every problem at once), `judge`, `with_instructions`, `answers_prompt`, the notification |
| Watch | `daemon/questions/watch.rs` | `notify` (inotify), one non-recursive watch per task directory with an open round; a save in place or by rename is seen |
| Answers | `daemon/owner/answers.rs`, `owner/start.rs` | the lifecycle: instructions and watch on a new round; `answer` from the protocol and the saved file; the prompt of the next run |
| Controls | `daemon/control.rs` (table), `daemon/owner/controls.rs` | `stop`, `pause`, `unpause`, `resume`, `say` |
| Opener | `agent-runtime` `editor.rs`, `daemon/opener.rs` | the VS Code window of the host, in a task of its own |
| Files | `agent-runtime` `text_file.rs` | `replace_text_file`: temporary file, mode, rename |
| Commands | `commands/{answer,stop,pause,unpause,resume,say}.rs`, `commands/control.rs`, `main.rs`, `environment.rs` | the six commands; `Environment::editor` |
| Docs | `docs/architecture.md`, `docs/state.md`, `docs/configuration.md` | the lifecycle, the table of the controls, what the daemon serves, the known limits |

### Decisions the reviewer should look at first

1. **The answers reach the next prompt through the state and the file, not through memory.** `start.rs::answered_round`:
   the last round is answered and the last run ended with `STATUS: QUESTIONS`. So it holds after a restart, after a
   `pause`/`unpause` in between, and for all three channels. A session that resumes gets the answers *instead of* the
   whole task (`harness/questions/answered.md`); a session that starts gets them after it.
2. **A control that stops a run waits for the run to end before it answers.** The owner cancels the run's token, then
   drains `runs.join_next_with_id()` until the task is out of `in_flight`; `interrupted()` records `RunEnd::Stopped` and
   sets the status the human asked for. A client that has its answer sees the new status (the item's check). A run that
   ends by itself in the same moment leaves the task stopped/paused where the status table allows
   (`control::after_halt`, a pure function with a test).
3. **A task that waits for answers is not stopped, paused or resumed.** I first allowed `stop` (the table has
   `questions -> stopped`), then found the consequence: the round stays open under a task that can be resumed and run, a
   run that asks again finds the previous round open and `OwnerError::Question` **ends the daemon**. Pinned by a test; the
   documentation says so. (That `ask()` can end the daemon on a state the human can reach is worth a look in a later item.)
4. **A pause of the human has no wake-up time.** It is recorded as `RunEnd::Stopped` plus status `paused`; nothing in
   `recovery` makes a wake-up for it, so it lasts until `unpause` and survives a restart (tested).
5. **`say` is a bus message from `human`.** For a task: to its agent; for a team on the bus: to the team; for a solo
   team (not on the bus): to each agent. A team's controls reach its reviewers; a `say` does not.
6. **`Parsed` and `Command` of `main.rs` are not extended.** The existing tests of the command line match both
   exhaustively, so the six commands are a third enum, `Steer`, as `run` was kept out of `Command` (`Invocation`).
7. **`open` defaults to `off`**, not to the old `vscode`: a trial in a temporary home must not reach a real editor. The
   daemon reads `HOME` and the runtime directory once (`Surroundings`), and `run_in` takes them, so the tests give it
   temporary directories.

## Checks

| Check of the item | Where it runs | Result |
|---|---|---|
| A fake CLI ending with questions leaves a questions file and a notification file; `agent answer <id> --file answers.md` resumes the task with the answers in its next prompt | `crates/agent/tests/steering_process.rs::a_task_that_asks_leaves_its_file_and_a_notification_and_goes_on_with_the_answers_of_agent_answer_file` (the `agent` binary, a daemon process, the stand-in CLI) | passes |
| `agent pause`, `unpause` and `stop` change the status `agent status` shows, and the activity names who did it | `steering_process.rs::pause_unpause_and_stop_change_the_status_and_the_activity_names_the_client` | passes |
| Answers from the protocol, a saved file, the dashboard | `daemon/owner/answers/tests.rs` (`17` tests: also a file saved while the daemon was down, incomplete answers, a second round, the VS Code opener) | pass |
| Each control's effect on state and activity | `daemon/owner/controls/tests.rs` (`17` tests), `daemon/control/tests.rs` (`9`) | pass |

## Tests

Test-first: every behaviour has a RED commit (the failing output in its message) and a GREEN commit; the table is in
"Commits". `126` new tests:

| Where | New tests |
|---|---|
| `agent-core` catalog and configuration (`tests/harness_questions_prompts.rs`, `tests/config_questions.rs`) | `10` |
| `agent-runtime` loader, `replace_text_file`, activity, opener (`tests/questions_prompts.rs`, `text_file_replace.rs`, `activity_control.rs`, `editor.rs`) | `26` |
| `agent` questions file (`daemon/questions/tests.rs`) and watch (`daemon/questions/watch/tests.rs`) | `17` + `6` |
| `agent` the decisions of the controls (`daemon/control/tests.rs`) | `9` |
| `agent` the daemon: answers (`owner/answers/tests.rs`), controls and `say` (`owner/controls/tests.rs`) | `17` + `17` |
| `agent` the command line (`tests/steer_commands.rs`) | `22` |
| `agent` the binary as processes (`tests/steering_process.rs`) | `2` |

**No existing test was modified or removed.** A new action `busy` of the stand-in `fake-claude.sh` (a run that says
something every 0.2 s until it is stopped) and two committed scripts (`fake-editor.sh`, `fake-vscode-cli.sh`) are new
files; nothing existing in them changed.

Two commits say `Test-First: not possible`: the acceptance test of the two checks (written after the units it runs
through) and the documentation/lint commits.

## Gate

```sh
cargo fmt --all -- --check                                                          # ok
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings       # 0 findings
cargo test --workspace --all-features --locked --no-fail-fast                       # 1 failure: the existing test above
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked  # ok
tools/gate.sh check-generated lint-shell lint-go lint-docs                          # ok (4 of 4)
```

With the one-line change above: `0` failures. Intermediate commits are not all lint-clean (dead code until the next
commit wires it); the final commit is.

**Flaky tests that already exist** (not caused by this change): on the base commit `28d06aeb`, in `30` runs of
`cargo test -p agent`, `4` runs failed on `daemon::tests::an_exhausted_quota_pauses_the_task_until_the_reset_and_it_goes_on_by_itself`
(`3`) and `daemon::owner::teams::tests::an_item_whose_branch_could_not_be_started_is_given_out_when_the_branch_can_be_made`
(`2`, one of them a 60 s timeout). I saw the first once in `20` runs here, and
`a_team_whose_state_cannot_be_saved_is_refused_…` once (a stale socket file makes the test connect before the new daemon
listens). I did not touch them.

## Size

`3,525` changed lines outside tests by the PR-1 command (limit `5,000`, split threshold `4,000`); `408` of them are
`daemon/owner/fixture.rs`, which is test support. No split was needed.

## Compatibility

- `agents.toml` gets a section `[questions]` (additive; every key has a default; a file without it loads as before).
- The activity log gets lines of `"kind":"control"` (additive).
- The state, the protocol and the questions files are unchanged. `Message::Request` of the daemon's own channel carries the
  client now (internal).
- `docs/configuration.md` is updated (this is a decision of the kind the checkpoint after feature 2 looks at: one more
  section of the configuration).

## Files outside the item's expected list, and why

The list names the six commands, `daemon/questions.rs` and the texts. The rest is what they need:

- `commands/control.rs`: the part the four controls share. `daemon/control.rs`, `daemon/owner/{answers,controls}.rs`: the
  owner's side (the item says "the questions lifecycle and the controls"). `daemon/questions/watch.rs`, `daemon/opener.rs`.
- `agent-core` `prompt/questions.rs` + `agent-runtime` `questions_prompts.rs`: the texts under `prompts/harness/questions/`
  need a catalog (no rendered text may be a string in `.rs`), as `harness/team/` has one.
- `agent-core` `config.rs`: "opens in VS Code **when configured**" and "the editor" need a place in the configuration.
- `agent-core` `cli.rs` + `agent-runtime` `store.rs`: "the activity names who did it".
- `agent-runtime` `editor.rs`, `text_file.rs`: the opener and the atomic replace of the questions file.
- `Cargo.toml`, `Cargo.lock`, `crates/agent/Cargo.toml`: see Deviations.
- Test support: `daemon/owner/fixture.rs`, `crates/agent/src/tests/steer_commands.rs`, the committed scripts, `busy`.

## Deviations

- **Dependencies (DEP-1).** `notify` `8.2.2` (on the approved list; default features off: inotify only) for the watch the
  item names; it brings `notify-types`, `inotify`, `inotify-sys` and (for BSD, not built on Linux) `kqueue`, `kqueue-sys`
  into `Cargo.lock`. `rustix` moves from `[dev-dependencies]` to `[dependencies]` of `agent`: the user ID names the runtime
  directory of the session (`/run/user/<uid>`), where VS Code puts its sockets.
- **Lint suppressions:** none (`#[expect]`/`#[allow]` not used).
- **Rules not followed:** `daemon::run_in` stays longer than `50` lines (it was already; I moved my part into
  `start_opener`).
- **`unwrap_or`/`.ok()` in production lines**, each a specified behaviour: `split_once('\n').unwrap_or((text, ""))`
  (a notification without a body); `.ok()?` in `watch::saved_file` (a name that is not `questions-<n>.md` is not an
  event); one `unwrap_or_else` that keeps the rendering error to report it after `QuestionsFile::expand`.

## Known limits (also in `docs/architecture.md`)

- A save of a questions file is not noticed when the system's inotify limit cannot be met; `agent answer` and the dashboard
  still work, and the daemon says so in its log.
- A `say` to a solo task that is `waiting` is read when the task next runs; nothing wakes it (a solo task waits only while
  its item is in flight: item 8.1).
- A stop gives the whole task again to a task that resumes (`prompt_kind`, as before); the old harness told the agent how
  long it was paused.
- The dashboard's `answer` is the same code, but there is no dashboard yet (9.1).

## Out of scope (noticed, not touched)

- `OwnerError::Question` from `ask()` ends the daemon when a round cannot be recorded (see decision 3).
- `Notice::render` takes only `HarnessPrompts`, so a notice made from another catalog goes through `Notice::Prepared`.
- `tests.rs` of the daemon builds its own fixture; mine is separate (`owner/fixture.rs`) because the existing one is private.

## Suggestions for AGENTS.md / the plan (I may not change them)

- Say what to do when an item serves a command that an existing test uses as its example of "not served yet": the rule
  "never modify existing tests" and the plan's "every item passes the whole gate" cannot both hold for that one-line change.
  A plan line such as "the round that serves a command updates the test that names it as not served yet" would do.

## For the repository notebook

- The questions lifecycle is derived, not remembered: the prompt of a run after answers is made from the last round and
  the end of the last run (`start.rs::answered_round`).
- `Parsed`, `Command` and `Cli` of `main.rs` are matched exhaustively by existing tests: a new command goes in a new enum.
- Never leave an open round under a task that can run: `ask()` ends the daemon on it.
- Daemon tests that restart a daemon in one home see `AlreadyRunning` while another test's fork still holds the lock: retry
  the start (the new fixture does).

## Commits

RED and GREEN in pairs; the failing and passing output is in each message.

| Commit | What |
|---|---|
| [`69ce1907`](https://github.com/mateusz-szczyrzyca/team-agents/commit/69ce1907193481953d89b43a1975c2259d1e578e) | Questions: the catalog of the texts of questions and answers (RED) |
| [`7e08ea9a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7e08ea9a725f2c61964c0e0d9c7d67a3d0d77e1b) | Questions: load the texts of the questions and their answers (RED) |
| [`4411fe19`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4411fe19d8d3fa336991d3f05c3b299647bea5da) | Questions: the catalog and its loader (GREEN) |
| [`8e567222`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8e567222c9bd3ee4e18e257084a50e2f07b6b51e) | Config: the section [questions] (RED) |
| [`3fbdefa2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3fbdefa211342efc31f3f22705fce9c1dc13cc14) | Config: the section [questions] (GREEN) |
| [`a4b5df97`](https://github.com/mateusz-szczyrzyca/team-agents/commit/a4b5df9763abc0bf2cdfceece21a642f1b9ce495) | Activity: a control of the human is a line of the log (RED) |
| [`2cbeb41d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2cbeb41ddfa76b091766f2064a76c63687b14c17) | Activity: a control of the human is a line of the log (GREEN) |
| [`c18beb8b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c18beb8b98c147ead3d1dafa52fc10dadad4a8de) | Questions: answers into a file, instructions, what the task is told (RED) |
| [`424ffe5d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/424ffe5dbf2b5aa9de56ff5db241eb626896fdeb) | Questions: answers into a file, instructions, what the task is told (GREEN) |
| [`f02244f6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f02244f6e0423350685a679e14fe3daf209812d2) | Controls: what stop, pause, unpause and resume do to a task (RED) |
| [`28efb985`](https://github.com/mateusz-szczyrzyca/team-agents/commit/28efb985d330a2a3c9086e3db1dee785272b4838) | Controls: what stop, pause, unpause and resume do to a task (GREEN) |
| [`8f3f59eb`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8f3f59eb7383961ee8e8c71b6c2a4cc901135506) | Questions: answers through the daemon (RED) |
| [`d8ed7880`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d8ed7880dd1b3d64ddab3a2ca20b9a5b75c979ab) | Text files: replace a file whole (RED) |
| [`53b07874`](https://github.com/mateusz-szczyrzyca/team-agents/commit/53b078748167d933f1c957508863d8c920f47fed) | Text files: replace a file whole (GREEN) |
| [`9f9cff59`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9f9cff59c1b52856a72ce1d5770425859b3df26c) | Questions: a watch on the questions files (RED) |
| [`1de49e07`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1de49e0743bed182bf4917ede898cf93ee165741) | Questions: a watch on the questions files (GREEN) |
| [`8b84aa43`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8b84aa430a62326b8611f2e7936effa3136f344c) | Questions: what a saved file says, and the notification of incomplete answers (RED) |
| [`2ebc403d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2ebc403d140ed6d2f2f3dbef4fd689c22f383e0b) | Questions: what a saved file says, and the notification of incomplete answers (GREEN) |
| [`b03f0fcf`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b03f0fcf54fd76ddb6672082e2621e5057fbba7a) | Questions: the answers of the human resume the task (GREEN) |
| [`5eeb6fe6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5eeb6fe6cc30c915274ffb8f7d8430faeffba6ed) | Controls: stop, pause, unpause, resume and say through the daemon (RED) |
| [`0ccc8c28`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0ccc8c286feb0bdc9219a7ddd59f9aef4bd606a0) | Controls: stop, pause, unpause, resume and say through the daemon (GREEN) |
| [`7be80444`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7be80444dbbd65d3dbbe70c4092d7d4c87e776d1) | Commands: answer, stop, pause, unpause, resume and say (RED) |
| [`26a98a0b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/26a98a0bbe364f12ba53425275c5039875714309) | Commands: answer, stop, pause, unpause, resume and say (GREEN) |
| [`0819d080`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0819d080fd305e5403ccb6ccc00fadce8130cff2) | Editor: open a questions file in the VS Code window of the host (RED) |
| [`f95edb6d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f95edb6db43bc5fef62026bb457edc48997e250e) | Editor: open a questions file in the VS Code window of the host (GREEN) |
| [`6bc8a90e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6bc8a90e33061a270a8ffd860b1888b30d7fd79e) | Questions: open a new questions file in VS Code when configured (RED) |
| [`f54ef82e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f54ef82e6118582f7b469352be7a6434d98f3b6d) | Questions: open a new questions file in VS Code when configured (GREEN) |
| [`ffaee4ed`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ffaee4ed34e7700f9efc4e6a0f656709ef3ec071) | Steering: the checks of the item as processes |
| [`ded61f83`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ded61f83704af86afecc401e2344fd9360e2c969) | Docs: questions, answers and controls of the Rust agent |
| [`c32a36ef`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c32a36ef1f8fe3324d09666ebea65898d6ef98af) | Fix the findings of clippy |
| [`f4a8b8fe`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f4a8b8fe26ac57ae7d9beedd012f167a3eefdcd9) | Keep the description of agent in its help |
| [`18f6913a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/18f6913a7d9c1bab1814830d5acb6ad172bb317e) | Steering: split the long functions, no bool parameters, a test for reviewers |
| [`7d3f4d75`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7d3f4d758732d7c971f184745465edb17f88db87) | Controls: a task that waits for answers is not stopped (RED) |
| [`e3e0c54d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e3e0c54d975d45a2f8f45a5a1d6aadeb883f11bc) | Controls: a task that waits for answers is not stopped (GREEN) |
