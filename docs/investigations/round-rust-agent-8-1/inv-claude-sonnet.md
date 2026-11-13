# Item 8.1 — Pull requests, reviews and the merge loop (`inv-claude-sonnet`)

Branch `agents/inv-claude-sonnet/20261113-160402`, from `0a774462`. Not a pull request: the merger integrates the branches.

## Task

Item 8.1 of the plan "Rewrite `bin/agent` in Rust" (feature 8, Autopilot; depends on 7), quoted from
[`rounds/round-8.1.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/rounds/round-8.1.md):

> **8.1 Pull requests, reviews and the merge loop.** When an item's integration has a green gate at its head: push, open the PR
> with a body from a template, mark it ready; later pushes update it. Two free investigators from outside the team (the PR waits
> until two are free) review it with the review rules of the prompt catalog and the item's full text, its expected result
> included; the verdict is parsed from the review rules' report format and posted on the PR. Both ready → merge through the API,
> then the next item; with `--no-merge` the human is notified instead. Changes requested → a fix pass for the team with the
> findings, at most the configured number; then a question to the human. All items done → the team ends and the human is notified.
>
> *Expected result.* Files: `crates/agent/src/daemon/{pr,review,merge}.rs`, the PR and review texts under `prompts/harness/pr/`
> and `prompts/harness/review/`. Adds: PR opening and updating, reviewer selection and verdict parsing, merge, fix passes,
> `--no-merge`. Tests: with a fake GitHub server and fake CLIs — two ready reviews merge and start the next item; changes
> requested start a fix pass; the last fix pass asks the human; `--no-merge` notifies instead of merging; the reviewer's prompt
> holds the item's full text and its expected result. Why: the default `--autopilot` loop, end to end.
>
> *Checks.* Against the fake GitHub server: two ready reviews merge the PR and start the next item; changes requested start a fix
> pass; after the configured passes the human gets a question. With `--no-merge` nothing is merged and the human is notified. The
> reviewer's prompt, as the fake CLI recorded it, holds the item's title, description and expected result.

## What and why

The default `--autopilot` loop, end to end, in the daemon: a finished item is published as a pull request, two investigators from
outside the team review it, their verdicts lead to the merge and the next item, to a fix pass, or to a question for the human.
How it works and its limits are in the new section of [`docs/architecture.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/docs/architecture.md)
("Pull requests, reviews and the merge (the autopilot)"). The decisions that shaped it:

- **Nothing of a round is kept in the state besides the items' phases.** Where a round is, is worked out at every look at a team
  ([`crates/agent/src/daemon/merge.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent/src/daemon/merge.rs)) from the items, the reviews they carry, the runs and questions of the tasks and two
  files of the task directories, so a daemon that restarts goes on; a test restarts it. In memory: retry times, what the human
  was told once, report bounces, when an approved pull request was last read.
- **The pull request is the lead's branch**: the coordinator's (it integrates the members' branches and gets the gate green), a
  solo agent's item branch. It is published when the lead's last gate record (`tasks/<lead>/gate.json`, from 7.3) passed on a clean
  tree at the tip of the branch and the tip contains the item's branch. Items integrated into one head share the pull request
  and move together ([`crates/agent/src/daemon/pr.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent/src/daemon/pr.rs)).
- **Reviewers are tasks of the team that end `waiting`**, not `done`: the state allows one task per investigator in a team, and a
  re-review after a fix pass or the next pull request must reach the same investigator. A report is matched with the review it
  answers by the time its run started, so a slow reviewer's report of a decided round is kept and posted, never taken for the next
  round ([`crates/agent/src/daemon/review.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent/src/daemon/review.rs)).
- **Fix passes go to the lead** (the coordinator has the integrated branch; a solo agent is its own lead), so the same branch
  updates the same pull request.
- **Reviews are posted as comment reviews** with the verdict in their body: the harness opens the pull request and posts with one
  account, which GitHub does not let approve it.
- **The question for the human is written by the lead** (woken with the reports and the file to write): the task status table does
  not allow `waiting → questions`, and the daemon cannot write a question for a task that has no run.

## Where the code is

| What | Where |
|---|---|
| decisions: which branch is published, what is written, who reviews, verdicts, rounds | [`crates/agent/src/daemon/pr.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent/src/daemon/pr.rs), [`crates/agent/src/daemon/review.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent/src/daemon/review.rs), [`crates/agent/src/daemon/merge.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent/src/daemon/merge.rs) |
| what carries them out (owner) | [`crates/agent/src/daemon/owner/rounds.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent/src/daemon/owner/rounds.rs) and its modules `publish`, `reviewers`, `verdicts`, `merges`, `human`, `prompts` |
| texts | [`prompts/harness/pr/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/prompts/harness/pr), [`prompts/harness/review/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/prompts/harness/review) (18 templates), catalog [`crates/agent-core/src/prompt/autopilot.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent-core/src/prompt/autopilot.rs), loader [`crates/agent-runtime/src/autopilot_prompts.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent-runtime/src/autopilot_prompts.rs) |
| git: `is_ancestor`, `fetch`, `detach`, `Revision::RemoteBranch` | [`crates/agent-runtime/src/git.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent-runtime/src/git.rs) |
| GitHub: `find_open_pull_request`, `RepoSlug::from_remote_url` | [`crates/agent-runtime/src/github.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent-runtime/src/github.rs) |
| files: `load_gate_record`, the reviews' reports | [`crates/agent-runtime/src/store.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent-runtime/src/store.rs) |
| state: `ItemMut::ask_reviews` | [`crates/agent-core/src/state/item.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent-core/src/state/item.rs) |
| configuration: `[github] api_base` | [`crates/agent-core/src/config/values.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent-core/src/config/values.rs), [`docs/configuration.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/docs/configuration.md) |

Files outside the expected list of the item, each needed for it to work: `owner.rs`, `owner/teams.rs`, `owner/start.rs`,
`team/prompts.rs` and `daemon/mod.rs` (wiring: the autopilot, the reviewer's end, the fix pass in the lead's prompt, the item that
starts from the remote's base, the new reasons to start a run); `agent-core` and `agent-runtime` (the additions of the table: a pull
request needs git, GitHub and files, and a new catalog needs its loader); `team-cli.sh` (four new actions); `docs/`, `README.md` and
`config/agents.example.toml`.

## Size

`4,152` changed lines outside tests, with docs and prompts (the PR-1 counter, over the whole branch; the test helpers
`tests/rig.rs`, `tests/fake_github.rs` and `tests/round_fixture.rs` are under `tests/` and not counted): `3,932` of them Rust.
That is under the limit of 5,000 and over the 4,000 at which PR-2 asks for a split planned up front. I did not split because the
stages only build on each other through the owner's glue; if the merger wants a stack:

1. `agent-core` and `agent-runtime` additions (`ask_reviews`, `api_base`, git, GitHub, files, the catalog and its loader): about `700` lines.
2. The three pure modules `pr`, `review`, `merge` and their texts: about `900` lines.
3. The owner's autopilot, its wiring and the docs: about `2,500` lines.

## Checks run by me

```bash
make gate                                # PASS (at the last commit before this report)
cargo test -p agent --locked             # 566 passed (bin), 3 + 1 + 2 + 5 + 4 + 1 (integration tests)
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings    # 0 findings
cargo test -p agent --bin agent daemon::owner::rounds --locked                   # 32 passed, about 1 s
```

The checks of the item, against the fake GitHub server of the rig:

- **two ready reviews merge the pull request and start the next item**: `two_approving_reviews_merge_the_pull_request_through_the_api_at_the_head_they_reviewed`, `the_next_item_starts_from_what_was_merged_and_is_published_in_its_turn`, `when_every_item_is_merged_the_team_ends_its_reviewers_are_closed_and_the_human_is_told`;
- **changes requested start a fix pass; after the configured passes the human gets a question**: `a_request_for_changes_sends_the_item_back_to_its_lead_for_a_fix_pass_that_records_the_reviews`, `the_lead_is_given_the_fix_pass_with_the_findings_of_the_reviewers`, `when_the_fix_passes_are_used_up_the_lead_is_asked_to_put_the_question_to_the_human`, `the_humans_answer_starts_one_more_fix_pass_with_the_findings_and_the_answer`;
- **`--no-merge`**: `with_no_merge_nothing_is_merged_and_the_human_is_told_that_the_pull_request_is_approved`;
- **the reviewer's prompt, as the fake CLI recorded it**: `the_prompt_of_a_reviewer_holds_the_pull_request_and_the_items_title_description_and_expected_result` (title, description, expected result, checks, the verdict format, the role).

## Tests (each new behaviour and what specifies it)

The rig ([`tests/rig.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent/src/daemon/owner/rounds/tests/rig.rs)) is a real daemon in a temporary home with
a scratch repository and a bare `origin`, a stateful fake GitHub on a loopback port that merges into the bare repository
([`tests/fake_github.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518/crates/agent/src/daemon/owner/rounds/tests/fake_github.rs)), the stand-in CLI `team-cli.sh`
(new actions `work`, `integrate`, `commit`, `ask`: they commit, merge and record the gate as an agent does) and, for the team tests,
the real `agentbus`. A clock that ticks a second at every reading gives the events an order.

| Behaviour | Tests |
|---|---|
| `ItemMut::ask_reviews`: fills the empty round once; refuses a second request, none, a non-pending review, a duplicate, another phase | `agent-core/tests/review_requests.rs` (7) |
| `[github] api_base`: absent, valid, `http` on loopback, not http, empty, whitespace, too long, credentials not repeated | `agent-core/tests/config_github.rs` (9) |
| git: `is_ancestor` (true, itself, false, diverged, unknown revision), `fetch` (brings the tip, no local branch moves, no origin, no such branch), `detach` (moves, discards, unknown commit) | `agent-runtime/tests/git_publish.rs` (12) |
| GitHub: `find_open_pull_request` (found, query, encoding, none, fork, retry, 404), `RepoSlug::from_remote_url` (11 URL forms, refusals, no credentials) | `agent-runtime/tests/github.rs`, module `github_discovery` (11) |
| the catalog and its loader: files and entries agree, audiences, every text renders with its variables, language override of notices, every problem reported | `agent-core/tests/harness_autopilot_prompts.rs` (7), `agent-runtime/tests/autopilot_prompts.rs` (7) |
| the gate record and the reports on disk | `agent-runtime/tests/task_files.rs` (8) |
| verdict parsing (markup, case, first line wins, unreadable, cut), the posted text | `daemon/review/tests.rs` (12) |
| reviewer pool (busy rules, reuse of a waiting task, ended tasks, the draw) | `daemon/review/tests.rs`, module `pool` (15) |
| which branches are published, one pull request per branch, riders of an update; the title and description | `daemon/pr/tests.rs` (17) |
| where a round is, what a verdict leads to, what waits to be delivered, the human's answer | `daemon/merge/tests.rs` (39) |
| the autopilot in a daemon: publish, reviews asked, prompt, reports posted and kept, merge, next item from the merged base, reuse of reviewers, team end, bounce of a report without verdict, fix pass and update and re-review, the question and the answer, `--no-merge` (+ merged by the human), merge refused, closed, the team path, events, waiting for two free investigators, stale gate, an unposted review, a branch that cannot be pushed and a restart, no token, an unsafe API address | `daemon/owner/rounds/tests.rs` (32) |

Existing tests modified or removed: **none**. Tests added to existing files: the module `github_discovery` at the end of
`agent-runtime/tests/github.rs`, three tests at the end of `daemon/merge/tests.rs` and one `use` line there (below).

### Tests changed after RED

Four `TEST-FIX:` commits and one line, each for the reviewer to accept or reject:

- [`7eb3b08`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7eb3b0883f8edd2ff6fb4a2b190e14916b719bcf) `an_investigator_whose_review_is_asked_for_in_another_team_is_busy_even_while_it_waits` asked `inv-d` for a review in team 1 without giving it a task there, so the list it expected (`inv-b` only) was wrong; `inv-d` now has a waiting task, like `inv-c`. A correction of the test's setup, the rule it names is unchanged.
- [`64c5c5c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/64c5c5c70f3bcdd666147cafd01a23cde986ef98) the rig wrote the scenario of an agent into TOML as it was, so the `\n` of an action became a line break and the script read its first line only (the reports of the stand-in reviewers lost their summary). The rig doubles the backslashes. Helper, no assertion changed.
- [`edf2c32`](https://github.com/mateusz-szczyrzyca/team-agents/commit/edf2c32e6fa984e90a3ba012c4617e532b2456ba) removes `Rig::branches` and `round_fixture::solo`, unused helpers that the gate (`dead_code`) refuses.
- [`e28cccc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e28cccc12dd83a8397ed1d9035f761b08ad951fe) moves the three test helper files under `tests/` (`git mv`, no line changes), so the PR-1 counter sees them as test code.
- One import line of `daemon/merge/tests.rs` was reflowed by `rustfmt` when the commit `RED: test what the human's answer leads to` added two names to it (`RoundEventKind`, `RoundPhase`); no test changed. I should have added a second `use` line instead.

No `GREEN:` commit touches a test file (checked with `git show --stat`). For each `RED:` commit `git diff <red> HEAD` over the test files shows added lines, the listed corrections and that import line.

## Deviations

- **RED stubs that pass.** Where the right answer of a stub is also the answer of some tests, those tests pass in the RED commit: `is_ancestor` returning `false` (2 of 12), the lookups that find nothing (3 of 11), the loaders that load nothing, `plan_publications` and `pull_text` (6 of 17), `merge` (11 of 36), `free_reviewers` (3 of 15), `load_gate_record` (2 of 8) and others; each commit says how many. The team-path, `--no-merge`-start, no-token, unposted-review, restart, stale-gate, waiting-for-reviewers and events tests pass against the earlier GREEN commits (the code does not tell a coordinator from a solo agent, and so on): their commits say so (`Test ...`, not `RED:`). I checked two by breaking the code (the gate-head comparison, the busy rule): they fail.
- **Intermediate commits do not pass clippy alone**: the pure modules are `dead_code` until the owner calls them (PR-5 would ask for `#[expect(dead_code)]`, which would be removed again in a few commits). The head passes `make gate` and clippy with `-D warnings`.
- **`count(passes) -> u32` saturates** (`u32::try_from(..).unwrap_or(u32::MAX)`, one place, commented): a number of fix passes beyond `u32` cannot be reached (each is a round of reviews). Other `unwrap_or`, `filter_map` hits of the sweep are specified fallbacks on `Option` (`strip_suffix`, `split_once` of a notification, `unwrap_or_default` of a team that is gone).
- **The owner awaits GitHub and git** in its loop (like `agentbus` and `git` before); each call is bounded (GitHub's own attempt timeout of 30 s with at most 3 attempts for reads, once for writes; a push at most 10 minutes).
- No lint suppression (`#[expect]`, `#[allow]`), no `unsafe`, **no new dependency**, no change of `Cargo.toml`, `AGENTS.md`, `CLAUDE.md` or `.github/workflows/`.

### Compatibility (rule 9)

- State: **no schema change**. `ItemMut::ask_reviews` is a new operation on an existing phase; a round that is empty is a state old code could write. An `in_progress` item of an old state is published at the next look if its lead is waiting and its gate passed.
- Configuration: `[github] api_base` is optional (absent = the public API); the example and `docs/configuration.md` have it. `github_token` was already in `secrets.toml`; without it nothing changes from before this item.
- Files: new `tasks/<reviewer task>/review-pr<N>-pass<K>.md`; `gate.json` is now read (7.3 wrote it).
- Prompts: 18 new templates and a catalog loaded at the daemon's start (a home with a prompts directory that lacks them is refused, with every problem).
- Events: `pull_request_opened` and `review_submitted`, which the protocol already had, are published.
- A reviewer's task is `waiting` after its review (it was `done`): `settle` is unchanged, the owner changes the status for a reviewer while its team goes on; the team's closing ends it.

## Not done / known limits

- **Items integrated into one head share a pull request**; a finished item waits while another is in review on the same branch. Several items of a coordinator's team are therefore published one pull request after another, unless they were integrated together.
- **A reviewer whose run fails, is stopped or never answers is not replaced**: its review stays pending and the human resumes it (the state cannot add a review to a round that has them).
- **A merge is at the head GitHub carries** (`expected_head` is the remote head): a commit someone else pushed after the reviews is merged as well. A merge refused for a reason waiting does not change is told once and not retried; the human merges and the daemon notices (every 30 s).
- **`git push` uses the machine's credentials**, not `github_token`.
- **A solo agent that takes the next item while its first is in review** has its fix pass on another branch: its prompt tells it to switch (`{branch}`), the daemon does not.
- A report is cut at 20,000 characters; a review that cannot be posted counts and is only logged.
- The owner's loop waits for GitHub and git: a call that hangs leaves the socket unanswered that long.
- Not done by design: replacing a reviewer; a draft that stays a draft; closing the stale pull request when the item is stopped.

## Suggestions for `AGENTS.md` (not changed by me)

- Say where test helper modules go: a file beside the code it tests counts in PR-1's size; under `tests/` it does not (the counter is path-based). I moved mine.
- The rule "a test is frozen once its RED commit exists" makes an import reflow by `rustfmt` or a helper that turned out unused a `TEST-FIX`. State whether an *added* line to an existing test file (a `use`, a helper) needs one.
- The rounds' text says "tests not counted" but a rig for a daemon-level test is a thousand lines; a planner could allow for it in the 4,000 split threshold.

## Out of scope (noticed, not touched)

- `daemon::tests::commands_the_daemon_does_not_serve_yet_are_refused_with_their_name` (item 7.3's note) is still out of date; I did not change it.
- The developer rules and the rest of the Rust catalog do not mention `agent gate` yet (7.3's note); only the new review and fix-pass texts do.

## Commits

- [`1a81f1a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1a81f1ad9ceaba4a96ce1134266cbcc1acfe0a65) RED: test the request for the reviews of an open pull request
- [`2c2fe26`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2c2fe26ed428b710ce176f851dc84c7b41a510a2) GREEN: ask the reviews of an open pull request once
- [`6ddacf0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6ddacf08392c75e97a1f000feb5e98b433754c8c) RED: test the address of the GitHub API in the configuration
- [`c087e0c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c087e0c215930b4647a979cba3de4a32e00e4135) GREEN: check the address of the GitHub API in the configuration
- [`2fde6f4`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2fde6f493a9c40dde548547d61847e9e103dd761) RED: test the git operations that publish a branch and start after a merge
- [`eef51af`](https://github.com/mateusz-szczyrzyca/team-agents/commit/eef51afe205ad2a39e30abdb29ada2edf73e1149) GREEN: git says whether a commit is part of a branch, fetches and detaches
- [`8487afa`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8487afa205a568dba6371e6f607d1ee04e756108) RED: test the lookup of a branch's open pull request and the repository of a remote URL
- [`67d2861`](https://github.com/mateusz-szczyrzyca/team-agents/commit/67d2861552effd1ba41f3595d26fdd0298e3c695) GREEN: find the open pull request of a branch and read the repository of a remote URL
- [`2c086c6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2c086c6cadc6f557508cfe83290917c8776c66bd) RED: test the catalog of the autopilot's texts and the loader that reads it
- [`c150cdd`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c150cddbacabacde6afc8edec6d254e05b86a8d7) GREEN: the texts of a pull request and its reviews, and the loader that reads them
- [`d509c68`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d509c6875f66e6cd9e34a40920398eacb9ace979) RED: test the verdict and the posted text of a reviewer's report
- [`687a5f9`](https://github.com/mateusz-szczyrzyca/team-agents/commit/687a5f94c9c9ad761cad77f043121b2bdf32698e) GREEN: read the verdict of a report and the text that is posted
- [`1c8eb0b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1c8eb0b49ca99a8428baff4a2e6dcc31b7013fde) RED: test which investigators are free to review a pull request
- [`7eb3b08`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7eb3b0883f8edd2ff6fb4a2b190e14916b719bcf) TEST-FIX: the review of a second reviewer asked in another team needs its task
- [`d11ba80`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d11ba801f8d6c0db5364778f894fdf566dea2349) GREEN: choose two free investigators outside the team to review a pull request
- [`3e5ed48`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3e5ed48a1f8873a4eb3b321269bf4454637b8ad5) RED: test which branches are published and what is written on the pull request
- [`f9e159b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f9e159b9a54f58f321a92823ce4379df7e689ca5) GREEN: decide which branches are published and write the pull request
- [`516b7bb`](https://github.com/mateusz-szczyrzyca/team-agents/commit/516b7bb6d736ca05a06081f1322eaf3a0176f5aa) RED: test where the round of a pull request is and what waits to be delivered
- [`1611792`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1611792ece149e002414cfa923f40b4f1c24e134) GREEN: work out where the round of a pull request is from the state alone
- [`8d5ecce`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8d5ecce64c718dea6f9bcc2bb1aec668088784c0) RED: test loading the gate record and keeping the reports of the reviews
- [`1623ed7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1623ed747eac216dc9f9ab6a9bbe3adeb66ea69d) GREEN: load the gate record of a task and keep the reports of its reviews
- [`e6a90c7`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e6a90c7469711f46a5fd73ddab31fa93376d2868) RED: test that a finished item is published as a pull request and asked for two reviews
- [`54145a6`](https://github.com/mateusz-szczyrzyca/team-agents/commit/54145a643c36e73d8c855a0e0ba06b2bd6820eab) GREEN: publish a finished item as a pull request and ask two outsiders for the reviews
- [`9a8be37`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9a8be37275c637cfd03aca6668079efe51028f94) RED: test that two approving reviews merge the pull request and start the next item
- [`c62c45e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c62c45e10f5a8bc7267da288cb52fbc5ea190d63) RED: test that a request for changes sends the item back for a fix pass
- [`64c5c5c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/64c5c5c70f3bcdd666147cafd01a23cde986ef98) TEST-FIX: keep the backslashes of a scenario out of the TOML escapes
- [`263ef66`](https://github.com/mateusz-szczyrzyca/team-agents/commit/263ef66519572523e0458854dc7741fac17b0427) GREEN: take the verdicts of the reviewers, merge the approved pull request, go on
- [`bed4be0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/bed4be0e385d90cb29a25cb0ea78e261b6bbfd7b) RED: test that a fix pass reaches the lead and a fixed branch updates the pull request
- [`3ba7f7b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3ba7f7b6badb501cc9eafc32b398bd316e87000b) GREEN: give the lead its fix pass, update the pull request, ask the reviewers again
- [`ce6c633`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ce6c633b05ebfa6377cc09e50becfa6cc8c1dae0) RED: test that used-up fix passes put a question to the human
- [`24a4617`](https://github.com/mateusz-szczyrzyca/team-agents/commit/24a461712b77a7bbb1cc3116313cd4f1a976b68f) RED: test what the human's answer leads to
- [`233e209`](https://github.com/mateusz-szczyrzyca/team-agents/commit/233e209d83d335ae04399f6de99c878432a1a10f) GREEN: the human's answer is one more fix pass
- [`4183fcb`](https://github.com/mateusz-szczyrzyca/team-agents/commit/4183fcbaf918bbe0779054cd5a0e9cc5a721b52a) GREEN: ask the human when the fix passes are used up, and give a pass on the answer
- [`ad9c893`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ad9c893243bf25718eb44f65d7c705a7d32a7c70) RED: test --no-merge and a merge that does not happen
- [`3d7b412`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3d7b41273fce16b3e42030890212a626b490d269) GREEN: with --no-merge tell the human, and tell what a merge cannot do
- [`da59e00`](https://github.com/mateusz-szczyrzyca/team-agents/commit/da59e009d23b7a23b8c6ae84ecb39a7f65b10894) Test the team path: a coordinator's branch is published, and its fix pass is its own
- [`fcb2781`](https://github.com/mateusz-szczyrzyca/team-agents/commit/fcb2781dc4b3d12e2c1ecfa3471d08c41f452ea4) Test what goes wrong: an unposted review and a branch that cannot be pushed
- [`c35cc2d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c35cc2d747a18801e215f95c37383db24fc06d40) RED: test that the subscribers see a pull request opened and the reviews come in
- [`88ce81c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/88ce81c13999eef765dce2f8c589c302f73efd5b) GREEN: publish the pull request opened and the reviews that come in
- [`689b9f4`](https://github.com/mateusz-szczyrzyca/team-agents/commit/689b9f4d74c8836c06c2bd4a8b642098a74da1bf) Test that a pull request waits for two free investigators
- [`87acd99`](https://github.com/mateusz-szczyrzyca/team-agents/commit/87acd99149e6fc39c0538158da594438dede9210) Test that a branch that moved after the gate passed is not published
- [`edf2c32`](https://github.com/mateusz-szczyrzyca/team-agents/commit/edf2c32e6fa984e90a3ba012c4617e532b2456ba) TEST-FIX: remove two helpers no test uses
- [`f50ea42`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f50ea4299e18664f8aaf4e6af5f48f45a1166509) Clear the lint findings of the autopilot code
- [`d6f3919`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d6f3919d4e8b9594fe97a67bd2127d81c7a4df15) Count the passes of a pull request in one place
- [`e28cccc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e28cccc12dd83a8397ed1d9035f761b08ad951fe) TEST-FIX: move the test helpers of the autopilot under tests/
- [`f68f44e`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f68f44e9aa9cc984f490037c54301e6216501f83) Document the autopilot: pull requests, reviews and the merge
- [`0bd0f57`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0bd0f57c73241e534743f30a261fb2bd4aad3ff2) Test the settings of the autopilot: no token, and an address that is not safe
- [`b9eef6f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b9eef6ff3bdcea561c44cdf180e30e57bd3ca518) Split the owner's autopilot module and shorten its long functions
