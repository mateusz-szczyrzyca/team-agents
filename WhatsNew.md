# What is new in `agent`

Newest first. One entry per version, written by the pull request that raises it — the rule and the form are in
[`AGENTS.md`](AGENTS.md), "Commits and pull requests". Every line names in brackets the crates or modules the
change is in: `[agent]`, `[agent-core]`, `[agent-runtime]`, `[agent-web]`, `[agentbus]`. The entries start with
0.16.0; what came before is in the merged pull requests.

From PR #271 every crate has a version of its own: a pull request raises by `0.1.0` the version of each crate whose
code it changes, and only those, and the heading names them (`## agent 0.109.0 · agent-core 0.109.0 — ...`). Until
0.108.0 the four crates shared one version, and the heading gave that one.

## agent 0.156.0 · agent-core 0.156.0 — 28.11.2026 — PR #377

### New features
- [agent] [agent-core] An account can be held before its limit is reached: with `hold_at = 0.9` in `[accounts.NAME]`, no new run of the account starts once a window is 90% used, until that window resets. Running runs finish, queued tasks wait and nothing fails. You get one notification, `Account comp-claude is held until 15:30`, and a restart of the daemon keeps the hold.
- [agent] The log says `account_held` when a hold begins and `account_released` when it ends.

### Configuration
- [agent-core] new: `[accounts.NAME] hold_at` (no default: no hold) — a number from 0 to 1, the share of a limit window from which no new run of the account starts until the window resets; a value outside 0 to 1 is refused, naming the account.

## agent 0.155.0 · agent-core 0.155.0 · agent-runtime 0.138.0 — 28.11.2026 — PR #375

### New features
- [agent] `agent status` shows how much of each rate-limit window an account has used: `comp-claude: 5h 62% (resets 15:30, read 3 min ago), 7d 41% (...)`. An agent without an account has a line of its own. A line goes when its window has reset.
- [agent] [agent-core] [agent-runtime] The daemon keeps the newest reading of each window that a run of Claude Code showed, with the account of the agent that ran, and a restart of the daemon keeps it.

## agent 0.154.0 · agent-core 0.154.0 — 28.11.2026 — PR #371

### New features
- [agent] `agent swap <task> <agent>` gives a task to another configured agent: an ongoing run is ended first, the task is handed over with reason `swap`, and its next run starts in a new session in the same worktree and branch.

### Bug fixes
- [agent] [agent-core] Human swaps preserve the automatic replacement allowance while retaining every past holder.
- [agent] A run ended by a swap records the human who stopped it in its steps and history.
- [agent-core] An inconsistent state whose human swap count exceeds its holder history is refused.

## agent 0.153.0 · agent-core 0.153.0 — 28.11.2026 — PR #372

### New features
- [agent] [agent-core] An account can have a limit of runs: with `max_runs = 1` in `[accounts.comp-claude]`, the agents of that account never have more than one run going at once. A task beyond the limit stays queued, is not failed, and starts when one of the runs ends.
- [agent] `agent status` says why such a task waits: `queued, waiting for account comp-claude (1/1 runs)`.
- [agent] A request for a worker waits in the queue, not failed, while every pool worker that could take it has an account with its `max_runs` going; before, such a worker started a run beside the others.

### Configuration
- [agent-core] new: `[accounts.NAME] max_runs` (no default: without the key the account has no limit; `0` is refused) — the most runs the agents of the account have going at once, over all teams.

## agent 0.152.0 · agent-core 0.152.0 · agent-runtime 0.137.0 — 28.11.2026 — PR #368

### New features
- [agent] [agent-core] `agent status` writes the account in brackets after an agent that has one: `comp-claude-1 [comp-claude]`. The same goes for the tasks of `agent status <round>`. The line of an agent without an account is as before.
- [agent-core] [agent-runtime] `agent debug` has an `## Accounts` section in its summary: each account with its owner, its agents and investigators, and the names — never the values — of its `env` and secret variables. A configuration that is missing or does not load is reported there as unreadable.

## agent 0.151.0 · agent-core 0.151.0 — 28.11.2026 — PR #364

### New features
- [agent] [agent-core] While an account is paused, none of its agents is picked: not an investigator for a review or an investigation, not a replacement for a waiting task, not a worker for a request. Another account of the same owner and agents without an account are picked as before; when only agents of a paused account could be picked, the work waits in the queue, not failed, until the reset.

### Bug fixes
- [agent] A request kept by a restart of the daemon was given again to a worker of a paused account, and a worker's recount of runs could lift its exclusion within one pass; both now keep it out. A team's workers of a paused account take no request, on the first or the second try, a task refused by its provider is not handed to an agent or investigator of a paused account, and the lead of a paused account is not given a member's work: when the lead is the only one left to take it, the member's task is paused until the reset and carries on then, not failed — also when the daemon restarts in between, and a task held for its own account is woken at the end of its own account's pause, not its lead's.
- [agent-core] `state.json` keeps, for a task that waits for its lead's paused account, when it is woken (`account_waits`, written only when there are any); a state without it loads as before.

## agent 0.150.0 · agent-core 0.150.0 · agent-runtime 0.136.0 — 28.11.2026 — PR #362

### New features
- [agent] [agent-core] [agent-runtime] A session belongs to its account: when the account of an agent changes while its task is open, the next run starts a new session from the briefing of `agent renew`, with the reason `account_changed`, instead of resuming the old one. No new session is counted against the task. An investigator, which works for no round, is told in its prompt that the account changed, and given the plan its session before saved in its worktree.
- [agent-core] `state.json` keeps the account each task's session began under (`session_accounts`, written only when there are any); a state without it loads as before.

## agent 0.149.0 · agent-core 0.149.0 — 28.11.2026 — PR #363

### New features
- [agent] [agent-core] A human's swap skips only the tier rule, allowing tasks to be handed over to any configured tier except workers, with new socket command `Swap` on protocol 10, trouble reason `swap`, and typed refusal causes. An older daemon cannot read a state holding `swap`.

## agent 0.148.0 · agent-core 0.148.0 — 27.11.2026 — PR #361

### New features
- [agent] [agent-core] When a provider refuses a run of an agent that has an account (quota or rate limit), every task of every agent of that account is paused at once until the same reset, without a run of its own, and you get one notification (`Account comp-claude is paused until ...`, naming its agents) instead of one per task. At the reset all of them are queued again. Agents of other accounts, also of another account of the same owner, run on; agents without an account are judged one by one, as before.
- [agent] `agent status` shows a line per paused account: `account comp-claude paused until 15:30 (quota)`.
- [agent-core] `state.json` keeps which tasks the pause of an account holds (`account_held_tasks`, written only when there are any), so that a restart of the daemon keeps the pause and its reset and never wakes a task you paused yourself.

## agent-core 0.147.0 — 27.11.2026 — PR #360

### Enhancements
- [agent-core] Agents are told when to leave a finding about files and when to resolve one.

## agent 0.147.0 · agent-core 0.146.0 · agent-runtime 0.135.0 — 27.11.2026 — PR #357

### New features
- [agent] [agent-runtime] An agent that works on files another agent left a finding about (a flaky test, a trap in the code, a tool's behaviour) is shown it once, at the start of its run.

## agent 0.146.0 · agent-runtime 0.134.0 — 27.11.2026 — PR #358

### New features
- [agent] [agent-runtime] A run of an agent that names an account gets the account's variables in its environment: the daemon's, `[agent_env]`, the account's `env`, the account's `[account_env.NAME]` — the later wins. Two agents of one CLI with two accounts in one team each see only their own account's login. An agent without an account runs as before. The quota of an account is not shared between its agents yet.

## agent-core 0.145.0 — 27.11.2026 — PR #359

### Enhancements
- [agent-core] The state can hold the pause of an account (`accounts` in `state.json`, written only when an account is paused), and a new rule says when a refused run pauses its account: the first refused run pauses it until the reset (or the configured pause), a later one finds it paused, an old pause is expired. Nothing records a pause yet, so no run changes.

## agent-core 0.144.0 · agent-runtime 0.133.0 — 27.11.2026 — PR #355

### Enhancements
- [agent-core] [agent-runtime] A Claude Code run's `rate_limit_event` records are read into readings of the five-hour and the seven-day window (how much is used, when it resets); the newest record of each window wins, and a record of another shape gives no reading and no error. Nothing stores or shows the readings in this version.

## agent-runtime 0.132.0 — 27.11.2026 — PR #353

### Bug fixes
- [agent-runtime] opencode agents work in their own worktree again. Before, every opencode run started in the
  directory the daemon was started from (`AGENTS_HOME`), because the daemon passed its own `PWD` to the agent; one
  worker ran `git init` and `git config --global` there.

## agent-core 0.143.0 · agent-runtime 0.131.0 — 27.11.2026 — PR #350

### New features
- [agent-core] `agents.toml` can define accounts — `[accounts.comp-claude]` with an `owner` and an optional `env` — and an agent or an investigator can name one with `account = "comp-claude"`; a variable of the account's secrets goes to `[account_env.comp-claude]` of `secrets.toml`. This version only reads and checks them; no run uses them yet, and a home without accounts behaves as before.
- [agent-core] An agent or an investigator with an account must be named `<owner>-…` (`comp-inv-opus` for owner `comp`); an agent that names a missing account, an account nobody names, a bad owner and a name without its owner are refused when the configuration loads, each error naming the account.
- [agent-core] [agent-runtime] A home whose `secrets.toml` has an `[account_env.NAME]` of an unknown account, or a variable that the account's `env` sets too, is refused, naming the account and the variable and never a value.

### Enhancements
- [agent-runtime] The values of `[account_env.NAME]` are cut from `agent debug` bundles and from the history, like every secret.

### Configuration
- [agent-core] new: `[accounts.NAME] owner` (required, a lowercase word of letters and digits, such as `comp`) — whose subscription or key the account is.
- [agent-core] new: `[accounts.NAME.env]` (default none) — variables of the environment of a run with the account that are not secret.
- [agent-core] new: `[agents.NAME] account` and `[investigators.NAME] account` (default none) — the account the agent runs with.
- [agent-core] new: `[account_env.NAME]` of `secrets.toml` (default none) — the secret variables of the account `NAME`.

## agent 0.145.0 — 27.11.2026 — PR #349

### Bug fixes
- [agent] A gate that finds the machine's test lock held by another test run now waits in line for it. Before, it was refused at once ("another tests are running now"), was recorded as a failed gate, and its round waited until the agent asked again.
- [agent] A process that a test left behind no longer keeps the machine's test lock after its test run or gate is over. Before, it held every later gate, for example after a daemon restart stopped a gate halfway.

## agent-core 0.142.0 — 27.11.2026 — PR #342

### Enhancements
- [agent-core] Repository findings can be matched to a run's paths, including long paths and names with spaces, and prepared as a bounded prompt section with one line per finding; the daemon does not send them to runs in this version.

### Configuration
- [agent-core] new: `[findings] keep_days` (default `30`) — how many days a finding is shown.
- [agent-core] new: `[findings] max_shown` (default `8`) — findings in one run's message.
- [agent-core] new: `[findings] max_chars` (default `3000`) — their texts' length in one message.

## agent 0.144.0 · agent-core 0.141.0 · agent-runtime 0.130.0 — 27.11.2026 — PR #346

### Bug fixes
- [agent] [agent-core] [agent-runtime] A step given back after one worker's wrong result now says "the worker's result does not serve step 5.3"; before, it said "neither result" and "do not send it a third time". A step whose two results were both wrong keeps the old words.
- [agent-core] A worker commits the files of its request also when the request says not to commit, and says so in its notes; before, it obeyed, and its work was thrown away. Members, coordinators and solo agents are told never to ask a worker for uncommitted edits.

## agent 0.143.0 · agent-core 0.140.0 · agent-runtime 0.129.0 — 27.11.2026 — PR #338

### New features
- [agent] [agent-core] [agent-runtime] A worker's request, a review and an investigation now keep checkpoints like a round's own work: what is meant, what is done in relation to it and what is left, taken after each step, each local commit, the end of a run and the start of the daemon.
- [agent] [agent-core] [agent-runtime] A worker's request that goes back to the queue, a reviewer that is replaced and an investigation tried a second time start from the last checkpoint of the one before: the commits are in the worktree, the findings so far are the new reviewer's leads, and the pass keeps its number.
- [agent] [agent-core] A job that passes to another holder leaves a `holder_changed` checkpoint of the new holder, and a run is briefed from the checkpoint of its own task only: two reviewers of one pass that run side by side are no longer told one replaces the other.
- [agent] [agent-core] After a restart of the daemon the next checkpoint of a worker's request still tells how its check last ran.
- [agent] [agent-runtime] A reviewer is told to write each finding to a findings file of its own for the pull request and the pass (`review-pr<n>-pass<p>-findings.md` in its task directory); a new checkpoint `findings_written` follows when it changes.
- [agent] `agent checkpoints <task>` works for the task of a worker, a reviewer and an investigator, and the line of such a checkpoint ends with its job (`job review`).

### Enhancements
- [agent-core] A checkpoint written by an earlier version, with no job, is read as the checkpoint of an item.

## agentbus — 27.11.2026 — PR #341

### New features
- [agentbus] Agents can record findings about repository files with `finding_add`, close them with `finding_resolve`, and list open notes with `agentbus findings --repo <repo> --json`.

## agent 0.142.0 · agent-core 0.139.0 · agent-runtime 0.128.0 — 27.11.2026 — PR #339

### Enhancements
- [agent] [agent-core] [agent-runtime] After the lead's conflict passes, the daemon asks an investigator for a resolution and gives the lead one extra pass before asking the human.

### Configuration
- [agent-core] new: `[merge] investigator_pass` (default `true`) — ask an investigator once per item and pull request head after the ordinary conflict passes; `false` keeps the direct human fallback.

## agent-runtime 0.127.0 — 27.11.2026 — PR #337

### Enhancements
- [agent-runtime] [agentbus] The daemon can file a conflict investigation for an item with `agentbus inv-add --kind conflict --item 2.3`.

## agent-runtime 0.126.0 — 27.11.2026 — PR #336

### Enhancements
- [agent-runtime] Every agent is told to read code, diffs and logs in small parts (line ranges, git diff --stat first, one dump per command), so that less output fills its context.

## agent 0.141.0 · agent-core 0.138.0 · agent-runtime 0.125.0 — 27.11.2026 — PR #334

### New features
- [agent] [agent-core] [agent-runtime] A new session of a task now opens with the last checkpoint of its round: what is to be done, what is done per requirement and what is left, then "Go on from it: do what is left, do not do again what is done." This holds for `agent renew`, an agent replaced because it was busy or failed, a round the round limit cut, a restart of the daemon or of the machine, and a session that ended without a status.
- [agent] [agent-core] A lead that is given the work of a member that went bad is told the member's last checkpoint as well.

### Enhancements
- [agent] [agent-runtime] The briefing of a new session keeps the snapshot of the last checkpoint when the worktree has not changed since, and takes no new snapshot reference.

### Bug fixes
- [agent-runtime] A checkpoint whose texts the history cut (`[history] max_text_bytes`) can be read again, so that a new session starts.

## agent 0.140.0 · agent-core 0.137.0 · agent-runtime 0.124.0 — 27.11.2026 — PR #335

### Enhancements
- [agent] [agent-core] [agent-runtime] A run that resumes its session after a report (the checks, a fix pass, a continuation) is sent only what happened since, not its whole task again; the first prompt of each session is kept in the task's directory as session-prompt.md.

## agent 0.139.0 · agent-core 0.136.0 · agent-runtime 0.123.0 — 27.11.2026 — PR #332

### New features
- [agent] [agent-core] [agent-runtime] Coordinators can request a conflict investigation with `kind: "conflict"` and an item. The investigator works locally at the current pull request tip and returns resolution commands, resolved hunks and the version to set without pushing.

## agent 0.138.0 · agent-core 0.135.0 · agent-runtime 0.122.0 — 27.11.2026 — PR #331

### New features
- [agent] `agent checkpoints <round|task>` lists the checkpoints of a round or a task, oldest first, one line each; `--show [N]` prints one checkpoint in full (the last without N) and `--json` prints them as JSON lines.
- [agent] [agent-core] [agent-runtime] Every checkpoint now says what is meant (the task, the item and its numbered requirements), what is done per requirement (its state, its steps and the commits with their `Step:` trailer, the findings resolved, the review passes passed) and what is left (the requirements not met, the open findings and what to do next).

### Enhancements
- [agent-core] [agent-runtime] A checkpoint of a task with a plan that has no `For:` lines lists its steps and its commits under "not tied to a requirement".

## agent 0.137.0 · agent-core 0.134.0 · agent-runtime 0.121.0 — 27.11.2026 — PR #328

### New features
- [agent] [agent-core] [agent-runtime] Coordinators can ask an investigator for a complete split proposal with `kind: "split"` and `item`, then submit the returned JSON through the normal split checks.
- [agentbus] Investigation requests accept an optional kind and item; split and conflict requests require the team's coordinator and an item.

## agent 0.136.0 · agent-core 0.133.0 · agent-runtime 0.120.0 — 27.11.2026 — PR #324

### New features
- [agent-core] [agent-runtime] The requirements of an item are numbered when the item is given to an agent: the bullets of its `Expected result:` are `R1`, `R2`, ... and its checks `K1`, `K2`, ...; an item with neither has one requirement, `R1`, its title. The prompt shows each with its number.
- [agent-core] [agent-runtime] A step of the plan can say which requirements it serves with a line `For: R1, K2`.
- [agent] [agent-core] A run on an item whose plan has a step that names no requirement, or one the item does not have, is sent back for its plan. A plan of an older task with no `For:` line is read as before.
- [agent] [agent-core] [agent-runtime] In the worktree of a task whose plan has `For:` lines, a commit with no `Step: N.M` line naming a step of the plan is refused with what to add (merge, `RED:` and `TEST-FIX:` commits too).

### Enhancements
- [agent-core] `state.json` can hold a plan reason `no_requirement` and a `requirements_tied` marker of a plan; older states load unchanged, and an older `agent` cannot read a state that has them.

## agent-core 0.132.0 — 27.11.2026 — PR #327

### Enhancements
- [agent-core] Coordinators and members ask an investigator before they ask you a technical question.

## agent 0.135.0 · agent-core 0.131.0 · agent-runtime 0.119.0 — 27.11.2026 — PR #321

### New features
- [agent] Team investigation requests now start a free investigator in a separate worktree at the requester's branch tip. Completed answers are saved and sent directly to the requester, waking its task; investigation work stays local.
- [agent-core] [agent-runtime] Investigation starts, answers and unknown request kinds are recorded in the daemon log and history. Existing history is preserved on upgrade.

### Bug fixes
- [agent] Failed investigations retry with another investigator once, keep unfinished text private, and notify the requester after a second failure, timeout or human stop. Requests from ended tasks are closed without a message; active assignments recover after restart.
- [agent] A failed investigation with no eligible replacement now notifies its requester. Worktree setup no longer consumes the answer deadline, and failure cleanup and notice delivery can resume after restart.
- [agent-core] Investigators at a provider's limit wait for their cooldown before receiving another request.

### Configuration
- [agent-core] new: `[investigations] answer_minutes` (default `30`, range `10..=1440`) — maximum time for an investigator's current attempt, excluding time in the queue.

## agent 0.134.0 · agent-core 0.130.0 · agent-runtime 0.118.0 — 26.11.2026 — PR #319

### New features
- [agent] [agent-core] [agent-runtime] After every step of a round the daemon records a `checkpoint` in the history and keeps a snapshot of the holder's worktree under `refs/agents/snapshots/<round>/<task>/c<n>`: each local commit (also one that is never pushed), the pull request opened, pushed, ready, checked, approved, merged or closed, every review, fix pass and answer to a finding, a question and its answer, a merge refused for a conflict, the end of a run, a round cut by `[limits] round_minutes` (before the task is taken over), a hand-over, a renewal or a new session, and the start of the daemon. An agent cut before it pushed can be replaced from its last local commit. A snapshot of an unchanged worktree is reused; the references of a round are removed when it is merged or closed.

### Enhancements
- [agent-core] [agent-runtime] `history.db` is upgraded to schema 34 at the first start (old records load unchanged): it admits the `checkpoint` record and the `checkpoint_failed` log event.
- [agent] [agent-runtime] The daemon waits at most 2 seconds for the history to have a checkpoint's record: a history that is stuck (a database another process holds) is logged once and no longer holds a round up.
- [agent] A checkpoint that cannot be taken (the worktree is gone, git or the disk fails) is logged as `checkpoint_failed` and recorded without a snapshot; the round goes on.

## agent 0.133.0 — 26.11.2026 — PR #322

### Enhancements
- [agent] [agentbus] Investigation tools are available only to team coordinators and members, including those using investigator-tier agents; other tasks and pool workers cannot call them.
- [agentbus] `agentbus serve --investigations=off|on` controls investigation access; omitting it preserves compatibility with older daemons.

### Bug fixes
- [agentbus] Investigation requests now ask agents to wait with `STATUS: SLEEPING`, so an answer message can wake them.

## agent 0.132.0 · agent-core 0.129.0 · agent-runtime 0.117.0 — 26.11.2026 — PR #316

### Enhancements
- [agent] [agent-core] Team state and activity history can retain investigator tasks separately from reviews; older state still loads.
- [agent-core] The texts of an investigation request and of its answer are in the prompt catalog; nothing sends them in this version.
- [agent-runtime] History accepts investigator runs while preserving existing records.
- [agent-runtime] [agentbus] `agentbus inv-next --task TASK` can select requests from specific tasks without claiming another team's requests; repeat the option for more tasks.

## agent 0.131.0 · agent-core 0.128.0 — 26.11.2026 — PR #315

### Bug fixes
- [agent] [agent-core] A round is no longer stopped and handed over for the time it waited for your answer: `round_minutes` counts only the time the agent could work.

### Configuration
- [agent] [agent-core] changed: `[limits] round_minutes` (default `0`, off) — time waiting for your answer is excluded from the round's time limit.

## agent 0.130.0 · agent-core 0.127.0 · agent-runtime 0.116.0 — 26.11.2026 — PR #313

### Bug fixes
- [agent] [agent-core] [agent-runtime] A replacement takes the old agent's place on the team bus before its first run, so team messages and Serena reminders reach it. If the bus is unavailable, the daemon saves the change and retries it before starting the replacement, including after a restart. Busy hand-overs use the same process; solo hand-overs make no bus change. Hand-overs saved by older daemons are recovered before launch, and missing or closed teams keep the replacement pending.

## agent 0.129.0 · agent-core 0.126.0 · agent-runtime 0.115.0 — 26.11.2026 — PR #314

### Enhancements
- [agent] [agent-core] [agent-runtime] The briefing of a new session (after `agent renew`, a takeover or a busy hand-over) now has a section "The pull request and its review": the pull request of the task's item, where its review stands (in review, fix pass `n` of the limit, waiting for the human, approved) and the findings by number with their status (`open`, `resolved`, `not confirmed`). The new session is told to go on in that pull request and fix the open findings, not to start the item again.

## agent 0.128.0 · agent-core 0.125.0 — 26.11.2026 — PR #312

### New features
- [agent] [agent-core] `agent renew <task>` starts the next run of a task as a new session, briefed from its checkpoint, its commits and its unread messages: the way out when a lead repeats itself or cannot be reached, without removing the team. Say what you want with `agent say <task> "..."` first; the new session reads it before anything else. It prints `Renewed <task>: its next run starts a new session`, works for a lead or a member (solo or in a team), and is not counted against `[replacement] new_sessions`. A task that is running, done, never run, a worker or a reviewer is refused with the reason.
- [agent] The notice of a round that waits for your instruction after "not published" names `agent renew <task>`.

## agent-core 0.124.0 — 26.11.2026 — PR #311

### Bug fixes
- [agent-core] Replacement and busy hand-over now select only an agent of the same tier: a normal agent for a normal agent, an investigator for an investigator.

### Configuration
- [agent-core] changed: `[replacement] agents` (default `[]`) — only an agent of the same tier as the one it replaces is taken.

## agent 0.127.0 — 26.11.2026 — PR #310

### Bug fixes
- [agent] Answers saved in the questions file while the run that asked was still going (with **ANSWERED** on the last line) are now taken up when that run ends with its questions; before, the task waited for ever for a save that had already happened. An incomplete file tells you what is missing, and `agent answer` for a task that is still running says to save the answers in the file.

## agent-core 0.123.0 — 26.11.2026 — PR #308

### Bug fixes
- [agent-core] The story of a review pass tells the runs of both reviewers together, in the order they were requested, before their reports; a report recorded before the other reviewer's run had begun no longer splits the two runs.

## agent 0.126.0 · agent-core 0.122.0 — 26.11.2026 — PR #304

### Bug fixes
- [agent] [agent-core] A file you refused at the checks before the review no longer stays refused for good after the team changes it again: a commit to the file made after the refusal lifts the refusal, and you are asked about the file again (`allow` starts the review, `refuse` sends the round back once more). A refused file with no later commit is sent back and not asked, as before.

## agentbus — 26.11.2026 — PR #302

### New features
- [agentbus] `agentbus team-replace --team ID --from OLD --to NEW` moves a member, the board work they claimed and their unread team messages to the replacement, and informs the team.

## agent 0.125.0 — 26.11.2026 — PR #300

### Bug fixes
- [agent] A gate that the busy test lock refused (exit 75, "another tests are running now") no longer counts as a failure of the commit: it did not run the checks, so two refusals no longer make the gate answer "not run again" for a commit that never failed.

## agent 0.124.0 · agent-core 0.121.0 — 26.11.2026 — PR #297

### Enhancements
- [agent] [agent-core] The times `agent status`, `agent log`, the notifications, the prompts and the pull request descriptions show no longer end with the offset: `2026-11-21 17:29:53` instead of `2026-11-21 17:29:53 +01:00`. A time that cannot be shown in the local zone is still written in UTC with `UTC` after it, and `agent log --utc` still prints RFC 3339 with `Z`.

## agent 0.123.0 · agent-core 0.120.0 — 26.11.2026 — PR #294

### Bug fixes
- [agent] [agent-core] A request to a worker whose check goes on as a sentence (`grep -n x README.md shows the line`) is refused at once, with the word that starts the sentence, instead of going to two workers whose results then fail that check.

## agent 0.122.0 · agent-core 0.119.0 — 26.11.2026 — PR #292

### Bug fixes
- [agent] [agent-core] `[limits] round_minutes` now stops an agent that never commits: before, each cut for an order to commit started the clock of the round again, so the limit never came.

## agent 0.121.0 · agent-runtime 0.114.0 — 26.11.2026 — PR #289

### Bug fixes
- [agent] A pool worker's run is now recorded in the history: `agent log --task <worker task>` shows its start, end and steps instead of nothing, and the steps are no longer lost.
- [agent-runtime] `agent debug` now covers a team's pool workers: `--task` accepts a worker task and writes its team's bundle, and every team bundle holds the workers' `context.json`, the output of their last three runs and a `workers:` list in the summary. Before, a worker task failed with `unknown diagnostic target` and the bundle left the workers out.

## agent 0.120.0 · agent-core 0.118.0 · agent-runtime 0.113.0 — 26.11.2026 — PR #287

### New features
- [agent-runtime] The bus library adds typed wrappers for `inv-next`, `inv-set`, `inv-get` and `inv-list`.

### Enhancements
- [agent] [agent-core] The daemon picks randomly among investigators tied on oldest activity.

## agent 0.119.0 — 26.11.2026 — PR #288

### Enhancements
- [agent] `agent status` and `agent status --running` keep listing a team's reviewers after their review, with when it finished, as they list its workers.

## agent 0.118.0 · agent-core 0.117.0 · agent-runtime 0.112.0 — 26.11.2026 — PR #284

### Bug fixes
- [agent] [agent-core] [agent-runtime] An item's pull request is now judged and pushed from the tip of the item's own branch, whatever branch the lead's worktree has out: the checks, the gate match and the push no longer see another item's commits. When no gate passed at the item's tip and the worktree has another branch out, nothing is pushed and the lead is sent back to run `git switch <branch>` and `agent gate` (`publish_off_branch` in `agent log`).

## agent 0.117.0 · agent-core 0.116.0 · agent-runtime 0.111.0 — 25.11.2026 — PR #283

### Enhancements
- [agent] Solo tasks, members and leads whose provider still refuses their retry move to a free agent of another provider, keeping their worktree, branch and briefing. With nobody free, they wait for capacity or their provider’s return.
- [agent] [agent-core] [agent-runtime] You are notified once when a task waits for a replacement or changes agent; its history names the provider, full return time and replacement.

### Configuration
- [agent] changed: `[replacement] agents` (default `[]`) — now drives refusal takeovers, trying the list then the tier `normal`; a lead goes to an investigator.

## agent 0.116.0 — 25.11.2026 — pushed to main

### Bug fixes
- [agent] A worker's result is accepted again: workers were told to commit only when the request said so, so they left their work uncommitted and every result was refused ("the worker committed nothing"). A worker is now told to commit the files a request names in its own worktree, and to do the job there even when the requester's worktree seems to have it already.

## agent 0.115.0 · agent-core 0.115.0 · agent-runtime 0.110.0 — 25.11.2026 — PR #281

### Bug fixes
- [agent] [agent-core] [agent-runtime] A run for an item of a task that works several items now starts with the worktree on the item's branch: a clean tree is switched (`worktree_switched` in `agent log`); a tree with uncommitted changes is left alone (`worktree_switch_refused`) and the lead is told to commit them and run `git switch <branch>` itself.

## agent 0.114.0 · agent-core 0.114.0 — 25.11.2026 — pushed to main

### New features
- [agent] [agent-core] An agent that works on an item longer than `[limits] round_minutes` (e.g. `120`) without its pull request in review is stopped, and its task is handed over at once to the first fitting agent of `[replacement] agents`, which is briefed in a new session on the same branch, or to the team's lead. With nobody to take it over, the task fails and you are notified with the round, the item and the minutes. No `agent swap` is needed. The clock starts again after a hand-over and when you stop and resume the task.

### Enhancements
- [agent] Every `task_handed_over` log record names why the task was handed over in `reason`, e.g. `reason = "round_overdue"` or `reason = "hang"`.

### Configuration
- [agent-core] **Action:** new: `[limits] round_minutes` (default `0`, off) — minutes an agent may work on an item without its pull request in review before its task is handed over; add `round_minutes = 120` to `[limits]` to turn it on.

## agent 0.113.0 · agent-core 0.113.0 — 25.11.2026 — PR #279

### Bug fixes
- [agent] [agent-core] A round whose task (a reviewer, a member, a lead or a solo task) is paused because its provider refused its run no longer says in `agent status` that it waits for an answer of the human when no question is open. It says whose provider and until when: `waits for the provider of inv-gemini-argon until 10:07` (local time, no offset; with the date when the return is more than a day away). An open question, or a pause you asked for with `agent pause`, still says the human.
- [agent] [agent-core] A round whose only wait is a task you stopped with `agent stop` no longer says it waits for an answer of the human: `waits for you to resume task-20261125-161204-67e1 (stopped at 17:12)`.
- [agent] The line of a provider's wait is written from one look at the clock, so a refusal that ends while `agent status` is being answered no longer leaves `{{agent}}` in it.

## agent 0.112.0 · agent-core 0.112.0 · agent-runtime 0.109.0 — 25.11.2026 — PR #278

### Enhancements
- [agent] [agent-core] The notice of a provider's refusal says what the provider said: the kind (an overload, a rate limit or a quota), the status, the message and the provider's code, for example `an overload: 429 · Model busy, retry later · engine_overloaded`, instead of the raw JSON line cut off. The raw line stays in the run's files.
- [agent] The resume time of a paused task in its notice uses the same local minute format and offset as its header, instead of UTC.
- [agent] [agent-core] [agent-runtime] A Claude run that its account's rate-limit window refused resumes at the window's own reset time (`resetsAt`), not at the time its text names, also when the text says a quota was hit. The notice gives that reset in the host's zone, for example `… · resets 2026-11-12T12:00:00+01:00`.

## agent 0.111.0 · agent-core 0.111.0 — 25.11.2026 — PR #273

### Enhancements
- [agent] [agent-core] A reviewer whose agent the refusal rule changes (its provider refused it again after its retry, or the first time when `[replacement] limit_retries` is `0` or a quota's reset is further away than `[replacement] limit_wait_minutes`) is replaced at once by a free investigator of another provider, not after `replace_after_minutes`. On 23.11 a review waited 35 minutes behind such a reviewer while other investigators were free. A reviewer refused once is kept and tried again after its pause, in the same session.
- [agent] With no investigator of another provider free, the review waits for the first that is, or for the refused reviewer's return, and you get one notification naming the provider and the time (also after a restart of the daemon). You are also told once when a refused reviewer is replaced.

### Configuration
- [agent] [agent-core] changed: `[review] replace_after_minutes` (default `15`, unchanged) — a reviewer refused by its provider after `[replacement] limit_retries` no longer waits for it; it still bounds a reviewer that is paused for any other reason.

## agent 0.110.0 · agent-core 0.110.0 — 25.11.2026 — PR #277

### Bug fixes
- [agent] [agent-core] A worker whose last request brought no usable result is no longer given the next request while another worker is free. Before, the first worker of a provider in `[tiers] workers` got every request of that provider, failed or not: on 25.11 `openrouter-minimax` failed three steps in a row of one task while four other workers stood idle.

### Enhancements
- [agent] [agent-core] The workers of one provider now take turns: with `deepinfra-qwen` and `deepinfra-glm` in `[tiers] workers`, DeepInfra's requests go to one and then the other, instead of always to `deepinfra-qwen` while it is free.

## agent-core 0.109.0 — 25.11.2026 — PR #275

### New features
- [agent-core] A pure function chooses the agent that takes over a task whose provider refused it (rate limit, overload or quota): from `[replacement] agents` — of another provider, not in the task's team, not busy and not refused — then from the tier `normal` in its order. A coordinator's task goes to an investigator; a member's task never goes to a worker. When no agent fits, the earliest time a refused agent returns is given so the daemon knows when to try again. The daemon is wired in a later release; this one provides the choice.

### Configuration
- [agent-core] changed: `[replacement] agents` (default `[]`) — the list now also serves a refused-provider takeover: each candidate is tried in order, skipping a name in the task's team, a busy one, a refused one or one of the refused provider; then the tier `normal` is tried in its order (for a coordinator, the investigators); a member never goes to a worker. The daemon is wired in a later release.

## agent 0.109.0 — 25.11.2026 — PR #271

### Enhancements
- [agent] Each crate now has its own version: `agent status` shows `agent 0.109.0 · agent-core 0.108.0 · ...`, and a version goes up only when that crate's code changed; a lead that resolves a merge conflict is told to raise only the versions of the crates its branch changes.
- [agent] A gate asked again on a clean commit where it already passed, with the same command, is answered with that pass at once instead of running again; one that failed twice in a row on the same commit is answered with its last failure, and its log ends with `gate: not run again: ...` and what to do (fix and commit, or ask in the questions file). A new commit or a changed tree runs the gate. On 25.11 such repeats took 2 of the day's 6 hours of gates.
- [agent] Each gate answered that way is logged as `checks_decided` with `outcome` `gate_reused`. After a restart of the daemon the first gate of every task runs again.

## 0.108.0 — 25.11.2026 — PR #270

### Bug fixes
- [agent] A daemon that is stopping or restarting no longer starts a queued task on the agent a stopped run has just freed. Before, such a run was cut a moment later, counted as a run that ended without a status and cost the task one of its retries (or failed it at the last one).

## 0.107.0 — 25.11.2026 — PR #266

### Enhancements
- [agent] In `agent status --running` the line of a worker under its team is laid out as the lines of the tasks: the worker's task first, then `worker`, the worker with its step, what became of the result and when it ran — `  task-20261125-095910-qlb9  worker       openrouter-minimax  step 4.2  failed, 2026-11-25 10:59–11:00 +01:00`. The request's bus number (`H5`) is no longer shown; look the worker up by its task.

## 0.106.0 — 25.11.2026 — PR #268

### Enhancements
- [agent-runtime] A Claude Code run no longer reads your own auto memory (`~/.claude/projects/<repo>/memory/`) or your claude.ai connectors: its settings carry `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` and `ENABLE_CLAUDEAI_MCP_SERVERS=false`, so the run starts with less context.

## 0.105.0 — 25.11.2026 — PR #267

### Enhancements
- [agent-runtime] Every Claude Code run is started with only agentbus and Serena: none of your own MCP servers or claude.ai connectors, no skills and none of 18 built-in tools it cannot use (`Workflow`, `CronCreate`, `RemoteTrigger`, ...) — the run starts with far less context.

## 0.104.0 — 25.11.2026 — PR #264

### New features
- [agent] [agent-core] The description of a pull request lists the steps whose request came back to the requester, with the first line of the worker's summary: `- step 1.3 of claude-1's plan — returned to claude-1 (H5): Result: the file is not written.`

## 0.103.0 — 25.11.2026 — PR #262

### Enhancements
- [agent] [agent-core] A provider that refused a run (rate limit, overload or quota) keeps its agent out of new tasks, solo tasks queued for it that have not run yet, reviews, takeovers and team dispatch until the time it returns; `agent status` shows the agent as refused until then. The agent comes back by itself, with no change of `agents.toml`.
- [agent] [agent-core] A task paused by a provider's refusal before this version keeps its agent out after a restart of the daemon, as a task paused now does.
- [agent] [agent-core] A refusal is counted per task: a good run between refusals clears the count. The rule's answer for each refusal is written to the daemon's log at debug level as a `decision_taken` event with `decision` `limit`; the change of agent after the retries is not acted on yet, so a refused task still waits out its pause.

**Action:** none is needed. Both keys are optional and have defaults; add them to `agents.toml` only to change them. Until the next version acts on the rule, their values change only the decision that is logged.

### Configuration
- [agent-core] new: `[replacement] limit_retries` (default `1`, `0` to `10`) — how many times a task is tried again in the same session after a pause for a refusal; when the retries are used up the rule answers "change agent". Until the replacement of an agent on a refusal is in place, the task keeps its pause.
- [agent-core] new: `[replacement] limit_wait_minutes` (default `60`; `0` never waits) — a quota whose reset is named further away than this answers "change agent" at once, because a retry before the reset is certain to be refused.

## 0.102.0 — 25.11.2026 — PR #263

### New features
- [agent] [agent-core] [agent-runtime] Every end of a request to a worker is one record of the history, `worker_request_ended`, under the requester's team and round: who asked, which worker served, the try, how it ended (`brought`, `again`, `returned`, `conflict`, `report`, or an interrupted request requeued or returned), when it started, how many seconds the worker's run took and what it cost. Example fields: `request H3, step 2.1, worker glm-1, try 1, outcome brought, seconds 90, cost_usd 0.0123`.

### Enhancements
- [agent-runtime] A history of version 28 is read and upgraded to version 29, which admits the new log event.

## 0.101.0 — 25.11.2026 — PR #258

### New features
- [agent] [agent-core] [agent-runtime] A request that no worker of the pool takes for `[workers] wait_seconds` (default 10 minutes) is answered `failed` and returned to its requester with the words `Nobody took your request H3 (step 2) for 10 minutes: no worker of the pool was free. Do the step yourself now.` The log event is `worker_wait_returned`.
- [agent] `agent status` shows since when a queued request waits: `request H3  step 2  no worker yet  queued  waiting since 21:10 (12 min)`.

### Enhancements
- [agent-runtime] A history of version 27 is read and upgraded to version 28, which admits the new log event.

### Configuration
- [agent-core] new: `[workers] wait_seconds` (default `600`) — how long a request waits in the queue for a free worker of the pool, in seconds; `0` waits for ever. A file without the key gets the default.

## 0.100.0 — 25.11.2026 — PR #260

### Enhancements
- [agent] [agent-core] [agent-runtime] `agent status --running` lists under each team every worker that took a request of the team, with the worker's task, its step, when it started and ended and whether its work was accepted: `worker H4  glm-1  task-20261125-075745-w8vj  step 6.1  2026-11-25 08:57–08:59 +01:00  accepted`.
- [agent] `agent status --running` no longer lists free workers: its `pool:` section shows only the busy workers of teams not listed above.

## 0.99.0 — 25.11.2026 — PR #255

### Bug fixes
- [agent] A restart after a worker result was recorded but before its pending result was saved now finishes the request and tells the requester once who served it.
- [agent] [agent-core] [agent-runtime] Recovery uses one retained-workspace context for startup and dispatch, preserving the failed provider preference across queued restarts.

### Enhancements
- [agent-core] [agent-runtime] Worker request returns and requeues are typed log events stored in history; a history of version 26 is upgraded to version 27 automatically while preserving its records.

## 0.98.0 — 25.11.2026 — PR #254

### New features
- [agent] [agent-core] [agent-runtime] `agent log --reminders [--days N]` shows, agent by agent and model by model, how the reminders work over the last `N` days (default `7`): how many reminders to commit and context reminders were given, how many commit reminders were followed and the median minutes until the commit, and how many runs were cut. With nothing to show it says `no reminders in the last 7 days`. It reads the history without the daemon.

## 0.97.0 — 25.11.2026 — PR #253

### New features
- [agent] [agent-core] [agent-runtime] A run that was reminded to commit and has not committed 20 minutes later is stopped, and its session is resumed at once with the order to commit what is finished first. It is never cut while a gate (`agent gate`) or a test run of its worktree holds the test lock. The cut counts as no retry, turn, rejection or replacement of the task, and what the run left in its worktree is snapshotted first. Each cut is the log event `commit_cut`.

### Enhancements
- [agent-runtime] A history of version 25 is read and upgraded to version 26, which admits the new log event.

### Configuration
- [agent-core] new: `[commit_reminder] cut_after` (default `20`) — the minutes after the reminder to commit at which a run that has not committed is cut and resumed with the order to commit; `0` turns the cut off.

## 0.96.0 — 25.11.2026 — PR #252

### Bug fixes
- [agent] [agent-runtime] Requests interrupted by a missing worker result or a daemon restart resume in the same worker workspace, preserving partial work and snapshots without spending a wrong-result try. Provider failures prefer a free worker of another provider; requests stay queued while no compatible worker is free.
- [agent] [agent-core] [agent-runtime] After three returns to the queue, another interruption returns the step to its requester with all recorded causes. Replayed and late results cannot deliver a second final answer.
- [agentbus] New requests with required capabilities use the same interruption recovery as other requests; requests saved before recovery was enabled keep their earlier rules.
- [agent] Final failure notices arrive after retry metadata is removed; the durable settlement remains available until delivery completes.

## 0.95.0 — 25.11.2026 — PR #251

### New features
- [agent] [agent-core] [agent-runtime] A session whose context reaches 80% of its bound is told once, on the bus, to commit what is finished and to write in its report where it is and what is next, before the next run may start a new session. A resumed session is told once, a new one anew; a session with no bound is never told, and neither is a Codex session, which tells no context. Each reminder is the log event `context_reminded`.

### Enhancements
- [agent-runtime] While a run goes on, its context is kept in `tasks/<task>/context.json`, written whole and only when it changes.
- [agent-runtime] A history of version 24 is read and upgraded to version 25, which admits the new log event.

### Configuration
- [agent-core] new: `[commit_reminder] context_percent` (default `80`) — the percent of its bound (`[session] max_context`, or the agent's own) at which a session is reminded once to commit and to write down where it is; `0` turns the reminder off, and a value above `100` is refused when the file is loaded.

## 0.94.0 — 25.11.2026 — PR #250

### New features
- [agent] [agent-core] [agent-runtime] A solo agent, a coordinator or a member that holds 300 or more uncommitted lines and has made no commit for 30 minutes is told once, on the bus, to commit what is finished. A commit re-arms the reminder; a reviewer or a worker is never reminded, and the daemon never touches the files of the agent. Each reminder is the log event `commit_reminded`, and `commit_followed` says when the run committed after it.

### Enhancements
- [agent-runtime] A history of version 23 is read and upgraded to version 24, which admits the two new log events.

### Configuration
- [agent-core] new: `[commit_reminder] lines` (default `300`) — the uncommitted lines (added plus removed) at which a run is reminded to commit; `0` turns the reminder off. A file without `[commit_reminder]` keeps the defaults.
- [agent-core] new: `[commit_reminder] minutes` (default `30`) — the minutes without a commit (from the later of the last commit and the start of the run) at which a run is reminded; `0` turns the reminder off.

## 0.93.0 — 25.11.2026 — PR #246

### New features
- [agent] [agent-core] [agent-runtime] A run that has Serena and searches code without it past a bound is told once to find code with Serena, while it goes on: the reminder reaches the agent on the bus and the run is not stopped. The shell's `grep`, `rg`, `sed -n`, `cat`, `head`, `tail` and `find` over a file or a directory of code count, and so do `Grep`, `Glob` and `Read` of the CLI; The line of the shell is read command by command, so a search's pattern is never taken for a directory, `rg x src | sort` counts and `git log | grep x` does not; a search filtered to Markdown or another text (`rg --glob '*.md'`) and `sed -i` never count. Markdown, TOML, logs and diffs never count, and neither does a run that called Serena.
- [agent] [agent-core] [agent-runtime] Each reminder is the log event `serena_reminded` and a record of the history with the task, the run, the count and the CLI. `agent debug` tells next to a run's tool count whether it was reminded and how many calls of Serena it made after.

### Enhancements
- [agent-runtime] A session that goes on after a restart of the daemon keeps its count and its reminder, and is never told twice, also when the bound was lowered since its earlier runs. A history of version 22 is read and upgraded to version 23, which admits the new record and log event.
- [agent-core] [agent-runtime] A CLI's `Grep` and `Glob` are told by what they were given: with no path a `Grep` searches the whole repository (also for a pattern such as `docs (path)`), with `docs` or `config` it searches text, and with a glob, `include` or `type` filter it searches the files the filter names — `Grep` of `src` limited to `*.md` is no search of code, a `Glob` of `*.rs` under `src` is. The history keeps the files as `src/*.rs (path)`. A search filtered to Markdown and Rust together counts, and so does `rg --files src`. A `Glob` with a brace group (`*.{ts,tsx}`) counts by each alternative; a search over the absolute path of the repository root is not counted — search `.` or give no path.
- [agent] A reminder of Serena that the engine recorded just before the daemon went down is put on the bus once when the daemon starts; the file `serena-reminder-told` in the run's directory, made before the bus is asked, keeps a reminder from going out twice: it goes out at most once per session, and one whose mark cannot be made is not sent, and a mark that cannot be looked for is logged and left to that claim.
- [agent] A reminder of Serena that the bus refuses, or that cannot be rendered or addressed, is tried again at the next turns of the daemon and at the end of its run, not lost.

### Configuration
- [agent-core] new: `[serena] remind_after` (default `40`; `0` turns the reminder off) — the count of searches of code without Serena at which a run with Serena is reminded once; a file without `[serena]` keeps the default.

## 0.92.0 — 24.11.2026 — PR #247

### Bug fixes
- [agent] [agent-runtime] [agentbus] A late or repeated worker result can no longer settle a newer attempt or send a second final notice, including after a daemon restart. The final notice names the worker that served the request.
- [agent] [agent-runtime] Workers that declare FAILED or QUESTIONS get another try, preferring another provider. Rejected work is saved before handoff, and a pool with one eligible worker can use its second try.

- [agent] Failed worker preparation leaves requests ready to dispatch again and preserves running workspaces.
- [agentbus] Replaying worker notices reads only newly appended messages after the first scan and preserves concurrent bus updates.

### Configuration
- [agent] [agent-runtime] changed: `[workers] tries` (default `2`) — new local requests count declared FAILED and QUESTIONS endings as wrong-result tries, even without file changes. Existing open requests keep their previous handling.

## 0.91.0 — 24.11.2026 — PR #244

### Enhancements
- [agent-core] [agent-runtime] Prepare a recovery policy and an opt-in durable journal for worker requests, retaining workspace and snapshot identities, run ownership and separate retry counters. Automatic recovery is not enabled in this version; existing requests keep their previous handling.

## 0.90.0 — 24.11.2026 — PR #243

### New features
- [agent] [agent-core] [agent-runtime] [agentbus] Teams and solo tasks can delegate to the installation's configured workers without adding them to their crew. Requests alternate providers, skipping busy providers; second tries prefer another eligible provider and can reuse the only eligible worker. Workers use isolated worktrees at the requester's commit.
- [agent] `agent status` shows each pool worker's requester and step, or whether it is free. Workers awaiting workspace release and workers in active legacy crews stay unavailable.

### Bug fixes
- [agent] [agentbus] Legacy crew requests keep their own workers without consuming unrelated pool capacity or changing the configured provider order.
- [agent] Pool runs share worker limits, the global run ceiling and disk checks with ordinary runs, including after their requester is removed. Failed preparation leaves the provider's turn available.
- [agent] `agent status --running` keeps workers assigned to active legacy crews even when those crews have no open rounds.

### Configuration
- [agent-core] new: `[agents.NAME] capabilities` and `[investigators.NAME] capabilities` (default `[]`) — capabilities matched against worker request `requires`, written as `<namespace>:<value>` with namespaces `os`, `arch`, `build`, `test`, `hw`, `net`.

## 0.89.0 — 24.11.2026 — PR #242

### Bug fixes
- [agent] [agent-core] A reviewer that its provider refuses run after run (a rate limit or a used-up quota) is now given to another investigator once `[review] replace_after_minutes` have passed since the review was asked. Before, every refused run started the wait anew, so short rate limits a few minutes apart kept the review, and the team, waiting until the human stopped the reviewer. A reviewer that did work since the request still waits the full time from its last real run.

### Configuration
- [agent-core] changed: `[review] replace_after_minutes` (default `15`) — counted from the reviewer's last run that its provider did not refuse, never before the review was asked; before, from its last run of any kind.

## 0.88.0 — 24.11.2026 — PR #241

### New features
- [agent-core] [agent-runtime] A run that has Serena is told in its first prompt how to find code with it. Find a symbol with `find_symbol`, its callers with `find_referencing_symbols` and a file's declarations with `get_symbols_overview`, and read only the body you need; `grep` and `rg` stay for text that is not a symbol. Every role gets it, a worker with the others, and so does a new session; a resumed session does not repeat it.
- [agent] [agent-core] [agent-runtime] `agent debug` counts each run's tool calls, Serena's and the shell's among them, as `tools: 214 (serena 12, shell 160, other 42)` — or `tools: none recorded`. Each earlier run of a task has its own count on a line of its own. With `[binaries] serena` set, you can see whether the agents reach it or grep the code.

### Enhancements
- [agent-runtime] An `agents.toml` without `serena` runs as before, and its prompts are byte for byte as before.

## 0.87.0 — 24.11.2026 — PR #240

### Enhancements
- [agent-runtime] Prepares a worker's task, branch and worktree at the requester's committed base, keeping the requester's uncommitted files untouched. Dispatch does not use it in this version.
- [agent] Settles recorded worker requests outside the team crew: checks the result in its own worktree, brings over a valid commit, retries a wrong result, and releases the workspace. Failed cleanup is retried; malformed results are retained for inspection.

## 0.86.0 — 24.11.2026 — PR #239

### Bug fixes
- [agent] [agent-core] A team no longer waits behind a member that failed after it called a bus tool when its lead already holds the member's work: once the lead ends `STATUS: DONE` with a passed gate and the member's branch is in the lead's branch (and no uncommitted file of the member's worktree is new to the lead), the member is closed as taken over and the pull request is opened, with no `agent stop` by hand. A member that failed without a single tool call still holds the round and the human is still told.
- [agent] [agent-core] [agent-runtime] Work of a failed or stopped member that the lead's branch lacks is never dropped: the lead is told once, in one message for all members, which commits and files; a file whose contents changed after the lead was told of it is told again; if the lead ends `STATUS: DONE` again without the commits, the human is asked once, with the list, also after a restart of the daemon.
- [agent] [agent-core] A member the human stops after it was once started in a new session is looked at like any other stopped member: its uncommitted files are told to the lead before the round is published. Only a member the ladder gave to its lead is skipped, and only until it runs again: a hand-over member that is resumed and stopped again is looked at; the state records that as `renewals.given_to_lead` (optional, absent in older states).

## 0.85.0 — 24.11.2026 — PR #238

### New features
- [agent] [agent-core] A team member that reports `STATUS: FAILED` after it called a bus tool no longer ends its task at once. The task is started once more in a new session, briefed with the reason `failed` and the words its agent last wrote; if that fails too, another agent of `[replacement] agents` or the team's lead takes it over (the lead may use the member's worktree and branch), and the human is asked only when no rung is left. `[replacement] mode = "off"` keeps the failure final, as before.
- [agent] **Action:** an installation with no `[replacement]` section now gets this too (a solo task or a coordinator that declares itself failed still ends failed) (defaults: `mode = "auto"`, `new_sessions = 1`, `agents = []`, so a new session and then the lead); add `mode = "off"` under `[replacement]` to keep a failed member failed at once.

### Configuration
- [agent-core] changed: `[replacement] mode` (default `"auto"`, unchanged) — it now also decides what follows a member's `STATUS: FAILED` after it used a bus tool: `auto` and `lead` climb the ladder, `off` fails the task at once, as before.
- [agent-core] changed: `[replacement] new_sessions` (default `1`, unchanged) — the new session after a declared failure is counted here too; `0` skips it.
- [agent-core] changed: `[replacement] replacements` (default `1`, unchanged) and `[replacement] agents` (default `[]`, unchanged) — a member that declared itself failed is handed to the first fitting agent of `agents`, as many times as `replacements` allows, and then to the lead.

## 0.84.0 — 24.11.2026 — PR #236

### New features
- [agent] [agent-core] [agent-runtime] An agent or investigator of `agents.toml` may carry `context`, its model's context window in tokens. For an opencode agent whose model is registered under `deepinfra/` or `openrouter/`, the generated configuration then gives the model a `limit` with that `context` and the agent's `output` (opencode's schema requires both; without `output` it is `0`, which makes opencode use its fallback of 32000 output tokens instead of the model catalogue's), and a codex agent gets `-c model_context_window=N`; the CLI then compacts the session itself before the provider refuses it. Without the key nothing changes; claude and agy are left as they were.

### Configuration
- [agent-core] new: `[agents.NAME] context` and `[investigators.NAME] context` (default: none) — the model's context window in tokens, given to the opencode configuration or the codex command line of that agent only. `0` is refused at load; a value below `max_context` is accepted.
- [agent-core] new: `[agents.NAME] output` and `[investigators.NAME] output` (default: none) — the most tokens the model writes in one answer, written next to `context` in the opencode configuration (without it opencode is told `0`, its fallback). `0` and a value above `context` are refused at load; other CLIs ignore it.

## 0.83.0 — 24.11.2026 — PR #235

### Enhancements
- [agent-core] Adds the decision that shares the agents of `[tiers] workers` as one installation-wide pool; nothing on a serving path uses it in this version.

## 0.82.0 — 24.11.2026 — PR #231

### New features
- [agent] [agent-core] A session whose context grew above its agent's bound is no longer resumed: the task's next run starts a new session, whose prompt opens with the size that ended the old one and then the briefing of the checkpoint (what was to be done, what is done, what is left). A size the CLI did not tell resumes the session as before.

### Enhancements
- [agent] [agent-core] The briefing of a new session ends with the agent's own last final message, whole.
- [agent-runtime] Each run leaves `outcome.json` in its own directory: the context of its last model call and its final message, whole. The bound of a session and the briefing of the next session read it first.
- [agent-core] The role texts tell every agent, in one sentence, to keep what the next session will need in `.agent/plan.md` and in the notebook.
- [agent] [agent-core] **Action:** a session above its agent's bound starts anew after this version; a session with a context the CLI tells and below the bound is resumed as before. The new session is not counted against `[replacement] new_sessions`.

### Configuration
- [agent-core] changed: `[session] max_context` (default `150000`) — the bound in tokens of a session's context for an agent without its own `max_context`; a session whose last run reached more tokens is no longer resumed; `0` switches the bound off, as before.
- [agent-core] changed: `agents.NAME.max_context` (default: the `[session]` bound) — this agent's own bound in tokens; above it the agent's next run starts a new session; `0` switches it off.
- [agent-core] changed: `investigators.NAME.max_context` (default: the `[session]` bound) — the bound of this investigator's session, as `agents.NAME.max_context` does for an agent; `0` switches it off.

## 0.81.0 — 23.11.2026 — PR #232

### Bug fixes
- [agent] [agent-runtime] The harness tests wait for complete records and saved state, so a loaded build no longer makes them read unfinished fake CLI files or miss a round transition.
- [agent-runtime] Restarting a daemon with a pending command check no longer fails when an unrelated process exits during recovery; malformed process records still report their cause.

## 0.80.0 — 23.11.2026 — PR #227

### New features
- [agent] [agent-runtime] `agent status <task>` gives the context of the task's last run: the size of the context at its first and its last model call, as the CLI told them. A CLI that tells no size shows `context: not told`, never a guess.
- [agent] [agent-core] [agent-runtime] `agent log` tells each run's usage at the run's own end, naming the run: its context, the output tokens it wrote and its cost in US dollars, each part "not told" where the CLI did not tell it. `agent log --task` and `--run` tell the same line after the `run_ended` row of each run, and `--show usage` gives a row of numbers per run.
- [agent] [agent-runtime] `agent debug` puts a team's series of run contexts into its summary and names each session whose context is above its bound.

### Enhancements
- [agent-runtime] **Action:** history schema 22 adds the table `run_usage` for the usage of each run; an existing history is upgraded in place and its runs read as "not told". Keep a history backup if you need to downgrade.
- [agent-runtime] The usage of a run that its CLI reports (Claude Code, Codex, Antigravity, opencode) is read from the CLI's output and written to the history after the run ends; a refused write is logged as `history_record_incomplete` and the run ends as before.

### Configuration
- [agent-core] new: `[session] max_context` (default `150000`) — the bound in tokens of a session's context for an agent without its own bound; `0` switches it off. `agent debug` names a session above it. Nothing resumes or stops a session by it yet.
- [agent-core] new: `agents.NAME.max_context` (optional, default: the `[session]` bound) — this agent's own bound in tokens, and that of `[investigators.NAME]`; `0` switches it off.

## 0.79.0 — 23.11.2026 — PR #224

### New features
- [agent] [agent-core] A task that fails twice at startup without calling a tool shows the reason in `agent status`; the human notice names the agent, CLI and model and advises checking its configuration before giving it more work.

### Bug fixes
- [agent] [agent-core] [agent-runtime] A first run that reports failure without calling a tool is retried once in a new session, for every role, including after a CLI change. The retry remains bounded after a daemon restart.
- [agent] Every failed team member tells and wakes its coordinator, with its actual run ending, item and last words when available, so the coordinator can wait for the human or take over when its role permits it.

### Enhancements
- [agent-core] **Action:** state schema 15 preserves earlier tasks and runs while recording tool use and startup failure reasons. Keep a backup before upgrading if you need to downgrade.
- [agent] [agent-core] [agent-runtime] **Action:** history schema 21 preserves existing events, round links, search and cursors while adding `run_no_tool_retry` at info and `run_no_tool_failed` at warn, with the task, agent, CLI and model. Keep a history backup if you need to downgrade.

### Configuration
- [agent-core] changed: `[replacement] new_sessions` (default `1`) — the startup retry always runs, even at `0`, and consumes one session of this shared recovery budget; at the default, a later hang or missing status gets no further recovery session.

## 0.78.0 — 23.11.2026 — PR #225

### Enhancements
- [agent] The release changes nothing the daemon does; it adds tests of a restart that cuts a run where no session is left to resume: a run whose CLI never reported a session still goes on in a new, briefed session; a run of a round that is already closed is resumed in its own session as before; a member of a state written by an older version begins a `daemon_down` session after a crash, and its reviewer does not; an older state without the renewal of a stop resumes the cut run as that version did.

## 0.77.0 — 23.11.2026 — PR #223

### New features
- [agent] [agent-core] **Action:** after a restart of the daemon, a run that was cut (the daemon stopped or crashed while it worked) goes on in a **new session** of the same agent, in the same worktree, whose first prompt is the briefing with the reason `daemon_down`: it lists the files the run left uncommitted, who worked on the task, the plan's steps as the harness judged them and your answers. The files stay in the worktree as they were, the round keeps its pull request and its review passes, and the cut counts as no retry. Set `[restart] session = "same"` in `agents.toml` to resume the old session as before.

### Enhancements
- [agent] [agent-core] A task started in a new session is briefed with more of what its earlier runs left: who worked on it and how each run ended (`DONE`, `QUESTIONS`, `no status`, `hang`, `turn limit`, `stopped`, `failed`, or cut before its end was recorded), the newest ten runs.
- [agent] [agent-core] The steps of the plan in that briefing are grouped as confirmed (the check passed), declared (no check ran, or a reviewer decides), mismatch (the check failed) and not done, so the new session checks the declared steps and repairs the mismatched ones first.
- [agent] [agent-core] The briefing lists the human's answers to the questions of the task with their verdicts, the newest ten, and says when older runs or answers are left out.
- [agent] [agent-core] The briefing tells the new session to find the real state first (the commits, the uncommitted files, the gate) before it goes on.

### Configuration
- [agent-core] new: `[restart] session` (default `"new"`) — `"new"`: a run cut by a restart of the daemon goes on in a new session with a briefing; `"same"`: it resumes the session it was cut in, as before 0.77.0.

## 0.76.0 — 23.11.2026 — PR #222

### Enhancements
- [agent-core] Add the internal foundation for restarting removed teams: saved replacement identities, bounded restart accounting and preserved merged items and split parts. Repeating a saved replacement leaves its current work intact. The daemon still removes idle teams without starting replacements; automatic restart will be activated separately.

## 0.75.0 — 23.11.2026 — PR #221

### New features
- [agent] [agent-core] **Action:** The daemon removes teams with no work for two hours, checking at startup and once a minute. This release removes idle teams without starting replacements. Set `[limits] idle_team_minutes = 0` to keep the previous behavior. Running or queued work, rounds waiting for a free investigator, human pause or stop, provider reset waits and another team's pull-request merge protect a team; a sleeping team waiting for review verdicts or an open human question alone does not.
- [agent] [agent-runtime] Before automatic removal, history records the last activity, items, split parts, round states and branches. Removal keeps the command's bundle safeguards and notification. A refusal keeps the team and names its branches in one notification; the daemon retries once a minute while the team remains idle, without repeating the refusal notice.

### Enhancements
- [agent] [agent-core] Idle observations survive a daemon restart. Removal counts from the later of recorded activity and current daemon startup, so downtime never counts and an old team gets a full configured idle interval after restart. Keep a state backup before upgrading if you need to return to an older daemon, which does not recognize the new observation field.

### Configuration
- [agent-core] new: `[limits] idle_team_minutes` (default `120`) — minutes without work while the current daemon is running before automatic removal; downtime does not count and `0` disables it.
- [agent-core] new: `[limits] idle_team_restarts` (default `1`) — a limit of replacements of a removed team; it has no effect in this release.

## 0.74.0 — 23.11.2026 — PR #220

### New features
- [agent] [agent-core] [agent-runtime] `agent remove <team> [--force]` removes a team's local work and takes it out of status. Before deletion, unmerged branches and detached commits are saved in `AGENTS_HOME/removed/<team>.bundle`, and open pull requests are closed with a reason; history, notifications and GitHub branches stay. `--force` first ends active work. The output names tracked uncommitted changes, the bundle, freed bytes and anything left behind, and a notification records the removal.
- [agent] Bus wake-ups are neither read nor cleaned, and reservations remain until released or expired; removal reports the available wake-budget records and known reservations. A team without a surviving worktree is refused with recovery instructions.

### Enhancements
- [agent] Removal also cleans up document and artwork directories when the team has no Git worktrees, including their sizes in the freed-byte total.
- [agent-runtime] **Action:** On the first start, the daemon migrates `history.db` to schema 20 to record team removals, preserving existing history. Older daemons refuse this newer schema; keep a backup before upgrading if you need to return to an older daemon.

## 0.73.0 — 23.11.2026 — PR #217 (continues #190)

### New features
- [agent] [agent-core] A queued task that waits longer than `[replacement] busy_after` (default `300` seconds; `0` never hands it over) for an agent running in another team is handed over to a free agent from `[replacement] agents` of the same or higher tier outside its team, choosing the one whose CLI has the fewest runs going at that moment; an empty `[replacement] agents` list performs no hand-over for a busy agent and the task keeps waiting for its own agent. The new holder starts from a briefing in the same worktree and on the same branch, the team is told on the bus, and the log records `task_handed_over` with `reason = "busy"`. Only a review on one of the waiting task's own recorded rounds prevents a busy hand-over; another member's review does not.
- [agent] `agent status` shows how many runs each configured CLI has going when at least one run is active, for example `runs: claude 1, codex 2, gemini 0`. It reads the configuration asynchronously and reports a file it cannot read or parse instead of dropping the error.
- [agent] `agent run` that starts a task for an agent already in another open team says that its task here waits while its run there goes on, or goes to another agent when `[replacement] agents` is non-empty and `busy_after` is above `0`.

### Bug fixes
- [agent-core] Pausing or stopping a queued task ends its busy wait; after resuming, it waits a fresh `busy_after` interval before a hand-over.
- [agent] A passing gate completed while the daemon checks a resolved merge conflict is recognized immediately, instead of unnecessarily asking the human to intervene.

### Enhancements
- [agent-core] The text story shows adjacent reviewer runs of the same team, pull request and pass in review-request order, keeping parallel reviews consistent; missing requests preserve the original order, and JSON stays chronological.
- [agent] [agent-core] The time a queued task has waited for its busy agent is kept in `state.json` (`waiting_since`) and survives a restart of the daemon; when the task is handed over before its first run, the first holder is not woken for it when its other run finishes.
- [agent] [agent-core] A busy hand-over's obligation to tell the whole team who holds each item is persisted as `busy_notice_owed`, rebuilt after restart and retried after transient bus failures. Permanent bus refusals and rendering failures are logged and drop the affected notice; the obligation clears once all notices are accepted or dropped. The coordinator and peers receive the item, branch, task and both holders instead of only the replaced member being addressed.

### Configuration
- [agent-core] **Action:** new: `[replacement] busy_after` (default `300`) — seconds a queued task waits for its busy agent before a free agent of `[replacement] agents` of the same or higher tier outside the team takes it over; `0` never hands a task over for this reason, and an empty `[replacement] agents` list performs no hand-over for a busy agent either.

## 0.72.0 — 22.11.2026 — PR #189

### New features
- [agent] [agent-core] [agent-runtime] A wrong result of a worker is tried once elsewhere: the request goes to a free worker of another CLI (of the same CLI when no other is free), whose prompt says who served it before and why that result did not serve, and the requester is told it went again. A second wrong result returns the step to the requester with the reasons of both results, and no third worker starts. `[workers] tries = 1` never gives a request to a second worker.
- [agent] [agent-runtime] A daemon that stops while a request waits for its second worker goes on from the file of the first result; the try is kept in `tasks/<worker task>/worker-try-<n>.json`.

### Bug fixes
- [agent] A file of a request that holds another request than its name says now answers the request it holds too, once.
- [agent] A request running for a worker whose task the bus has not recorded yet is the worker's by its name; a result already checked is not brought over for a request the bus has forgotten.
- [agent] A worker is no longer held busy for ever by the file of a request that was answered and cannot be removed.

### Enhancements
- [agent] [agent-core] [agent-runtime] The damage of a worker's request file is logged as `worker_request_damaged` (history migration 19, a warning) with the file's name; the log calls of the result of a worker that had no event now carry one (`bus_down`, `request_delegated`, `worker_request_damaged`).

### Configuration
- [agent-core] changed: `[workers] tries` (default `2`) is applied now to the result of a run that ended `STATUS: DONE`: a wrong first result goes to a second worker. `1` never gives a request to a second worker; a value above `2` behaves as `2` (a request gets one second try at most). A run that ends otherwise than DONE is still answered as failed at once.

## 0.71.0 — 22.11.2026 — PR #188

### Bug fixes
- [agent] [agent-core] A split no longer stops silently when its previous member is stopped, failed or handed over: the next part goes to a qualified configured replacement or the lead, or asks the human once when nobody can take it. Splits already waiting for a holder resume after an upgrade or daemon restart.
- [agent] [agent-core] A lead can report a split part delivered with `DELIVERED: #N` when its pull request was merged outside the harness. The next part opens, and the last part closes the item and team; unrelated, open or unmerged pull requests are refused.
- [agent] [agent-core] An outside replacement starts its own CLI session and failure history when taking over a task, instead of resuming the previous holder's session; the earlier runs remain in history.
- [agent] [agent-core] A later split part accepts a merged pull request from its parent task branch as well as its current part branch, including after the lead takes over.
- [agent] [agent-core] A replacement starts only after its new split round opens; a branch collision keeps the old holder stopped until the round can open. A replacement can also continue a stopped lead's task.

### Configuration
- [agent] [agent-core] changed: `[replacement] mode` (default `"auto"`) also controls who takes the next split part when its previous holder cannot continue; `auto` tries configured replacements then the lead, `lead` tries the lead, and `off` asks the human.
- [agent] [agent-core] changed: `[replacement] agents` (default `[]`) also orders the qualified replacements considered for the next split part; an empty list leaves the lead as the automatic fallback.

## 0.70.0 — 22.11.2026 — PR #187

### New features
- [agent] [agent-runtime] A worker's result is checked before it reaches your branch: when a worker's run ends `STATUS: DONE`, the request's check runs in the worker's worktree, and only when it passes is the worker's commit brought to the requester's branch, after a snapshot of the requester's worktree; the requester's uncommitted and untracked files stay as they were.
- [agent] The requester is told what became of the result: the step, the commit, the files and the end of the check's output when the commit is on its branch; the files in conflict, or in the way, when it is not applied; the reasons when the result is wrong (the check failed or ran past its limit, a file outside the request's `files` was changed, nothing was committed, the run lasted past `[workers] seconds`). A request answered that way ends `failed`, and `agent status` shows `request H1  step 1.1  worker glm-1  failed`.
- [agent] When a commit is brought over, the step of the requester's plan is recorded `confirmed` in the history, with the worker's name and the commit.
- [agent] [agent-runtime] A daemon that stops while a worker's result is being settled goes on from the stage it reached: the check runs once and a commit is brought over once. The stage is kept in `tasks/<worker task>/worker-request-<n>.json`.

### Bug fixes
- [agent] [agent-runtime] Only what the worker committed since its request is brought over, not the commits its branch already had from earlier requests.
- [agent] A result the daemon cannot judge (the starting commit or the worker's files cannot be read, a requested file was left uncommitted) is returned to the requester and never brought over.
- [agent] [agent-runtime] A worker whose result is being checked is no longer given the next request in the same worktree; it is busy until its result is settled.
- [agent] A request is answered once: a file of a request the bus has answered already, one that holds another request than its name says, or one that cannot be written back is no longer settled again at every look, and the answer of the bus is not overwritten.
- [agent] A long check no longer stops the daemon from serving other tasks: it runs in the background.
- [agent] [agent-runtime] A worker's request file that is cut short, has an unknown field, holds another request than its name says, cannot be read or cannot be written no longer stops the daemon: the file is kept aside as `worker-request-<n>.json.damaged`, logged as `restart_corrected` with its name, the requester is told that the result could not be judged, and the other requests go on; a restart does not meet the file again.
- [agent] A request the bus no longer knows has its file removed; an answer the bus cannot take now is tried again at the next look.
- [agent] A result of a worker for a request that names files and a check is no longer given to the requester unchecked as `done`; a worker that commits nothing for such a request has its step returned.

### Configuration
- [agent-core] changed: `[workers] seconds` (default `0`, no limit) is applied now: a worker's run that lasts longer is a wrong result and the step is returned to the requester. `tries` is applied with the second try of a request, in the next version.

## 0.69.0 — 22.11.2026 — PR #185

### New features
- [agent] [agent-core] An agent has one run at a time over all its teams and solo tasks. A task queued for an agent that runs another task waits, queued, and starts when that run ends; it is never failed. `agent status` shows it as `queued, waits for claude-1, running in team-…`.
- [agent] [agent-core] The round's wait line says which run a queued task waits for, after the run of its agent in another team.
- [agent] `agent run` that starts a task for an agent already in another open team says that its task here waits while its run there goes on.
- [agent-core] `[limits] runs_per_agent` and `runs_per_worker` set how many runs an agent (normal agent or investigator, and worker) may have going at once: default `1`, `0` is no limit.
- [agent] [agent-core] **Action:** the default is one run at a time per agent; set `runs_per_agent = 0` (and `runs_per_worker = 0` for workers) under `[limits]` in `agents.toml` to keep the old behaviour where an agent runs two tasks at once.

### Bug fixes
- [agent] Two tasks of one agent no longer run side by side in two teams; the second one waits for the first.

### Enhancements
- [agent] [agent-core] [agent-runtime] A queued task that waits for its agent is recorded once in the log as `task_waits_for_agent`.

## 0.68.0 — 22.11.2026 — PR #186

### Enhancements
- [agent-runtime] The harness can now name the files a worker's commit changes, tell which of them the request's `files` do not name, and bring the commit to the requester's branch after a snapshot of the requester's worktree: the requester's uncommitted and untracked files stay as they were, a commit that conflicts, one that cannot be written because the requester changed the same files, or one made while the requester committed meanwhile applies nothing and is returned or refused with the files named. The daemon does not use it yet.

## 0.67.0 — 22.11.2026 — PR #184

### New features
- [agentbus] The bus keeps, next to the step of a request for a worker, the paths the worker may write, the request's check and what to do (`files`, `check`, `doing` in the bus state); a request in the old form has none, and a bus state written before loads as it was. The daemon does not read them yet.
- [agent-core] `[workers]` in `agents.toml` says how many tries a request for a worker gets and how long its run may last — `tries = 2` and `seconds = 0` (no limit) unless you set them; `tries = 0` is refused. A file without `[workers]` keeps these defaults. The daemon does not apply the keys yet.

### Enhancements
- [agent-core] The harness decides, in one pure decision, whether a worker's result serves the request: accept it, give the request once more to a free worker of another CLI (of the same CLI when no other is free), or return the step to the requester with every reason. The requester is told each of those in its own text; the daemon uses them from the version that wires them.

## 0.66.0 — 22.11.2026 — PR #182

### New features
- [agent] [agent-core] [agent-runtime] A run of a role in `[plan] roles` that ends `STATUS: DONE` while a step it marked done fails its check (`mismatch`) now goes back to the same agent with the step, its check, the reason and up to 40 trailing lines of the check's output (counting against `[plan] bounces`, and combined in one send-back when an unfinished or lost step is present at the same time); after `[plan] bounces` send-backs you are asked once, in the task's questions file, whether to go on as it is or send it back once more.
- [agent] [agent-core] A step whose check matches `[plan] check_refuse` (`agent gate`, `make gate`) stays `declared`, and the task's next run is told once in its prompt that the gate is never a step's check.
- [agent] [agent-core] **Action:** the first run after this update may send a `STATUS: DONE` run back for a failed step check (up to `[plan] bounces`, default `3`); set `[plan] check_seconds = 0` in `agents.toml` to disable command checks, or `[plan] bounces = 0` to record plans and judgements without sending runs back.

## 0.65.0 — 22.11.2026 — PR #183

### New features
- [agent] [agent-core] [agentbus] The member, the coordinator and the solo agent, and an investigator through the bus, are told when to give a step to a worker and when not to: a step with one result, named files and a check a machine runs, never a decision, a design or a step a reviewer judges; a refused or failed request they do themselves.
- [agent-core] A plan can mark a step for a worker with the line `By: worker` in its text; plans without it read as before.
- [agent] [agent-runtime] [agentbus] `agent status <task>` lists the task's requests for workers under its plan, for example `request H3  step 1.2  worker glm-1  done`.
- [agent] The description of a pull request gets a section of the harness, "Steps done by a worker", for example `- step 1.2 of claude-1's plan — done by glm-1 (H3)`.

## 0.64.0 — 22.11.2026 — PR #181

### Bug fixes
- [agent] [agent-core] A failed publication of a merge conflict resolution no longer consumes a conflict pass or refunds an earlier published pass. Retries and daemon restarts count each published result once.
- [agent] [agent-core] Exhausted split rounds with valid gated work that never reached the pull request branch now resume publication and merging; legitimately spent passes and open human questions are preserved.
- [agent] Errors while reading changes after publication now follow the retry limit, lead notice and human question flow instead of retrying indefinitely.

## 0.63.0 — 22.11.2026 — PR #180

### New features
- [agent] [agent-core] [agentbus] An agent that keeps a plan names its work when it asks a worker: `delegate` takes `for` (a step of its plan still to do or in hand), `files`, `check` (a command or `exists PATH`, never `reviewer` or the gate) and `do`. A request that lacks one, names a step the plan does not have or that is done, a path outside the repository or a refused check is refused with words that say what to add, and no worker starts.
- [agent] [agentbus] The worker's prompt quotes the step of the requester's plan in full, then the request's files, check and what to do, and tells the worker to return a request that needs a decision with its question.
- [agentbus] In a team, a request for a step is refused at once when no worker of the team waits for work, so that the requester does the step itself.
- [agentbus] An investigator may now ask a worker too; a worker still may not.

### Enhancements
- [agentbus] Agents without a plan, such as a worker, a reviewer, an investigator answering an investigation request or a run whose worktree has no plan yet, still send the old form of a request, for look-ups. A bus state from before still loads.

## 0.62.0 — 22.11.2026 — PR #177

### New features
- [agent] [agent-core] [agent-runtime] When a run ends with a step of its plan newly marked done (`[x]`), changed while done or previously mismatched, the daemon now runs that step's command check in the task's worktree in the background, one command at a time for a task and without blocking other tasks or `agent status`. A command that exits `0` within `[plan] check_seconds` (default `120`) marks the step `confirmed`, and a command that exits non-zero, is ended by a signal or runs past `check_seconds` marks it `mismatch` with its reason and saves its output under the task's `checks/<run>/<step>.log`. Commands that match `[plan] check_refuse` (`agent gate`, `make gate`) are not run and stay `declared`.
- [agent] [agent-core] **Action:** command checks in `.agent/plan.md` now run automatically at the end of a run; set `[plan] check_seconds = 0` in `agents.toml` if you want command checks disabled while keeping `exists` checks active.
- [agent] [agent-core] [agent-runtime] Stopping or pausing a task, or stopping or restarting the daemon while a step's command check runs, ends the check's process group (validated by its kernel start time) and loses no finished run or completed judgement: a restarted daemon re-runs only the command checks not yet persisted for that run and settles the run once without starting a second agent CLI run.

## 0.61.0 — 22.11.2026 — PR #179

### New features
- [agent] [agent-core] [agent-runtime] The daemon counts how often a team's lead sends an item's work back to the member. The lead says so with a line `RETURNED: 1.1` beside `STATUS:` in its final message (its role text prescribes it); the daemon counts the line only when the member's last run on the item ended `DONE` and the member is started again for it, and keeps what the lead told the member after the hand-over, whole, as the reason. A part of a split item is returned as `RETURNED: 1.1a` and has its own count; a return is saved with the end of the lead's run. An answer to a question, a bounce of the plan, a refused gate and a lead that writes no such line are never counted.
- [agent] [agent-core] `agent status` shows the count on the member's line, `claude-1  waiting, returned 3 of 5`, and on the round's line when one more return reaches the limit. The count survives a restart; a state written before it loads with none.
- [agent] [agent-core] **Action:** `[replacement] rejections = 5` is the limit the count is shown against; `0` shows the count with no limit. This version only counts and shows: it does not replace the agent at the limit.

## 0.60.0 — 22.11.2026 — PR #178

### Enhancements
- [agent] `agent status` and `agent status --running` again list who is in each team and what each one does: a line for the coordinator, the members, the workers and the reviewers whose review is not over, between the team's rounds and its count of tasks — for example `task-…  member  claude-1  running, last activity 2026-11-22 10:15:42 +01:00`. A reviewer that gave its report or was stopped is only counted in the `tasks:` line. The plans of the tasks and the parts of a split stay behind `--task`.

## 0.59.0 — 22.11.2026 — PR #176

### New features
- [agent] [agent-core] A `SPLIT: <item>` line the daemon cannot take (the item is split already, or the team has no such item) is no longer dropped without a word: it is recorded in the log and the history with its reason, and the coordinator is told on the bus what was refused, why and what to do instead (deliver the part of its round, describe the rest as a task of its own). The message wakes a coordinator that sleeps; the status the run ended with (`DONE`, `QUESTIONS`) is acted on as before. A coordinator that repeats the line is told as many times as a proposal is corrected (`[split] corrections`, two), and then only its own status counts. A line that is not `SPLIT:` and an item number (`SPLIT: none`) asks nothing.
- [agent] [agent-core] The first prompt of a fix pass writes its findings to `fix-pass-<n>-findings.md` in the lead's task directory, and the line that says the pass is still open and the request to correct a response name that file when it is there, so a lead that lost the findings reads them again instead of asking you.

### Enhancements
- [agent] [agent-core] Every wait line of a round (`agent status`, `agent status --json`) now says who owes it and since when: `an answer of the human, owed by the human since 2026-11-22T03:59:28Z` (the `waiting` field is the same, with longer text; `agent status` shows the time in your zone). The start is the question's for your answer and the end of the round's last run otherwise.
- [agent-core] A lead's response is judged only when it was given in a run that started at or after the review's verdict, never a message from before the review.
## 0.58.0 — 22.11.2026 — PR #175

### New features
- [agent] `agent status` and `agent status --running` print each team with its rounds and a count of its tasks by what they do: `tasks: 1 running, 1 waiting, 1 with questions; questions number 1 wait for answers (claude-1, task-…)`. A task that waits for the human's answer is named with the number of its questions file.
- [agent] [agent-core] [agent-runtime] [agent-web] Every list of `agent status` starts with the versions of the four crates, `agent 0.58.0 · agent-core 0.58.0 · agent-runtime 0.58.0 · agent-web 0.58.0`. When the daemon runs another build, a second line shows its versions and says that they differ; a daemon that tells none is shown as `daemon: version unknown`.
- [agent] `agent status --task` (and `agent status --running --task`) prints the task lines, their plans and their parts, as `agent status` printed them before.
- [agent] **Action:** a script that reads the text of `agent status` or `agent status --running` for the task lines must add `--task`; `agent status --json` is unchanged.
- [agent] `agent status --task` is refused with a team, a task or a round, which already show their tasks, and with `--investigators`; with `--json` it is accepted and changes nothing.

## 0.57.0 — 22.11.2026 — PR #174

### New features
- [agent] [agent-core] [agent-runtime] A commit that takes lines out of a test its `RED:` commit froze is now refused while it can still be undone: the `git` of every run an agent starts has a `commit-msg` hook that tells the agent the rule, which file and which `RED:` commit, and to make a `TEST-FIX:` commit of the test files instead. It costs no question to you, works for every CLI that commits through `git`, reads the branch and changes nothing (a commit that is already made is never touched), and keeps the repository's own hooks running. The hook reaches the run through its environment: nothing is written into the repository, and a commit the agent makes in another repository during a run runs that repository's own hooks. `commit_guard = false` under `[repos.NAME.checks]` turns the refusal off for a repository (the checks before a review stay); a repository without the table is not judged.
- [agent-core] The roles of a member, a coordinator and a solo agent say in one line that every change to a test after its `RED:` commit — also formatting, a refactor, a removed debug line — is a `TEST-FIX:` commit of its own, test files only.
## 0.56.0 — 22.11.2026 — PR #173

### Bug fixes
- [agent] [agent-core] Rejected publication no longer retries forever without an explanation: after `[publish] tries` failures (default 3), the lead gets one correction pass, then the human gets a question to retry or stop. Conflict publication now observes the same retry backoff.
- [agent] [agent-core] Publication questions notify the human and enter the questions status directly, without recording a run that never happened. Another failed publication waits while the lead already has a question open.
- [agent] [agent-core] [agent-runtime] A restart no longer loses publication retry counts or question identity. Interrupted question creation preserves existing questions and answers, adopts an answer already saved, and clears retry backoff only for the publication that was answered.

## 0.55.0 — 22.11.2026 — PR #172

### New features
- [agent] [agent-core] `agent status` writes every time in the zone of this machine, with the date and the offset valid at that moment, in the same form as `agent log` (`2026-11-21 17:28:53 +01:00 command: git status` instead of `2026-11-21T16:28:53Z command: git status`): `started 2026-11-21 13:29 +01:00` instead of `started 21-11-2026 13:29`, `last activity 2026-11-21 17:28:53 +01:00`, and in `agent status <round>` the round, run and snapshot times (`opened 2026-11-21 13:29:09 +01:00`). `agent status --json` and the files of the harness stay in UTC (RFC 3339).
- [agent] [agent-core] Every notification's `Time:` line and a waiting step's `next try` time are written in the daemon's local time zone with the offset of that moment: `Time: 2026-11-21 17:30 +01:00` instead of `Time: 2026-11-21 16:30`.
- [agent] [agent-core] [agent-runtime] `agent debug` writes every timestamp in `SUMMARY.md` and in the task bundle texts (`end.md`, `questions.md`, `gate.md`) in the zone of this machine (`2026-11-21 17:28:53 +01:00`) instead of UTC with the local time in brackets.
- [agent] **Action:** a script that reads the text of `agent status`, `agent debug` or the `Time:` line of a notification must read the new form; use `agent status --json` for machine-readable UTC status.

## 0.54.0 — 21.11.2026 — PR #171

### New features
- [agent] [agent-core] [agent-runtime] When a commit took lines out of a test that its `RED:` commit froze (and is not a `TEST-FIX:` commit of test files only), the checks before a review ask you about it at once, all such commits of the round in one question: each commit with its subject, its author and the lines it added and took out. What you answer is kept for the item until it is closed, by the commit and the path, also through fix passes, merges of `main`, the parts of a split (a new commit in a later part is asked alone) and a restart of the daemon. A new commit is a new question, which says what was allowed before. `refuse` is kept as well, also across the parts of a split, and sends nothing back to the team, which cannot undo a commit. A file you allowed or refused with an earlier version covers the commits that were in the round then, and no later one.

### Bug fixes
- [agent] [agent-core] The checks sent a round back to its team for a commit that was already in the branch, which the team could not repair, and asked you about the same commits again after every fix pass. Now the round is not sent back for it and no pass is counted; a commit you allowed or refused is never asked again.

## 0.53.0 — 21.11.2026 — PR #170

### New features
- [agent] [agent-core] `agent log` writes its times as a person reads them: in the zone of this machine, with its offset at that moment, in one form. For example `2026-11-21 17:28:53 +01:00 command: git status` instead of `2026-11-21T16:28:53Z command: git status`. The rows, steps, story and follow all show the same moment the same way; `--json` and the files of the harness stay in UTC (RFC 3339).
- [agent] `agent log --utc` prints the times as the files hold them, RFC 3339 with `Z`, as before this version.
- [agent] [agent-core] A `--since` or `--until` without a zone is read in the zone of this machine: `agent log --round <round> --since "2026-11-21 17:29"`. With `Z` or an offset it is read as written.
- [agent] **Action:** a script that reads the text of `agent log` must read the new form, or use `agent log --utc` to keep the old text.

## 0.52.0 — 21.11.2026 — PR #169

### Bug fixes
- [agent] [agent-core] [agent-runtime] A split pull request kept a stale gate commit after a conflict pass, and a checked-out part branch or detached worktree caused publication to fail or switch branches: publication now pushes the passing gated commit directly to the pull request branch without forcing or changing worktree checkouts, and refuses diverged remote branches.
- [agent] Harness prompts and documentation instruct agents and leads not to switch branches in their worktrees during round execution.
- [agent] The harness waits for its running gate to finish before reading the result; previously a stale record could send the lead back before that gate finished.

## 0.51.0 — 21.11.2026 — PR #168

### New features
- [agent] [agent-core] A task file can say what it is in one line: `Short desc: Fix the typo in the README`, among the `Allow:` lines before the brief (at most 70 characters, once; a longer, an empty or a second one refuses the file with its line number and what to do). `agent run --short-desc "Fix the typo" claude-1 "..."` gives it to a task given as text. A task without it has a description derived from its brief: its first sentence cut to 70 characters with a `…`.
- [agent] [agent-core] Every notification about a task or a team says in words what the task is: `Task:` with the description and `Task ID:` on a line of its own, the item's number and title after the fields when the notification concerns an item (the part's title for a split part), and at the end a block `TASK (<task>):` with the first sentences of the task's brief. For example:

  ```text
  Repo: team-agents
  Time: 2026-11-21 16:30
  Task: Notifications name the task in words; `Short desc:` in task files
  Agent: inv-claude-sonnet (claude, sonnet)
  Task ID: task-20261121-132909-y6mz
  Team: team-20261121-132909-4qem
  Status: ❓ waiting for answers
  Note: inv-claude-sonnet has questions (task-20261121-132909-y6mz)

  1.1 The daemon runs the command check of a step that became done, one at a time for a task, without blocking

  Questions number 4 of task task-20261121-132909-y6mz wait for your answers in /home/…/questions-4.md.

  TASK (task-20261121-132909-y6mz):
  The first sentences of the task's description.
  ```
- [agent] [agent-core] `agent status` ends the line of each team with its description in quotes, `agent status --json` prints the teams as a JSON array with each team's `short_desc`, and the summary of a team that the daemon sends carries it as the optional field `short_desc`.

### Enhancements
- [agent] [agent-core] **Action:** the text of the notifications changed (new order of the fields, new lines): a script that reads them must follow. The socket protocol only gained the optional field `short_desc` in version 6, which older negotiated protocols do not carry; replace the CLI together with the daemon.

### Bug fixes
- [agent-core] A team started before 0.51.0 from a task file that already had a `Short desc:` line was named with the label in notifications and in `agent status` (`Task: Short desc: Checks ask…`): now only the description is shown (`Task: Checks ask…`), and the excerpt under `TASK` no longer repeats the line.

## 0.50.0 — 21.11.2026 — PR #167

### New features
- [agent] [agent-core] [agent-runtime] `agent log --follow` with `--task`, `--agent`, `--run`, `--round` or `--team` now shows the work, not only the daemon's records: the agent's messages, commands and tool calls as they are recorded, across runs and restarts of the daemon, after the newest lines of the scope (`-n`, default 100; `-n 0` shows only what comes). When a run ends it says how, and what the round waits for next. `--no-steps` leaves the steps out. For example:

  ```text
  2026-11-21T12:50:00Z run run-…-2fqp started by inv-claude-sonnet on task task-…-8cgk of team team-…-4qem (solo) in round round-…-q1w2
  2026-11-21T12:50:03Z command: cargo test -p agent-core
  2026-11-21T12:50:41Z The tests pass; I run the gate.…
  2026-11-21T12:52:10Z info run_ended task-…-8cgk (inv-claude-sonnet) team team-…-4qem rounds round-…-q1w2 run run-…-2fqp: ended reported DONE
  round round-…-q1w2 waits for the delivering pull request to merge
  ```
- [agent] `agent log --agent <agent>`, `--task <task>` and `--run <run>` without `--follow` print the steps of the runs they cover between the daemon's records, the newest `-n` lines of both together; `--agent` covers every run of that agent, in every team.

### Enhancements
- [agent] [agent-core] **Action:** the text of `agent log` lines changed (`--json` did not): every line of a record now says what happened and to whom — the task and the agent, the team and rounds, the run, and in words for the common events (`queued ← waiting`, `run started by claude-1`, `ended reported DONE`, `item 1.1 in review ← in progress after a review verdict`) or as `key=value` for the others, with long values cut. A program that reads the text should read `--json`.

## 0.49.0 — 21.11.2026 — PR #166

### Bug fixes
- [agent] `agent status --running` showed the parts of a split that were already merged as `to do` and the parts after them as `waiting for`, while `agent status` showed them `done`. Every view now prints the same state for a part, and the running view leaves out the parts of an item whose parts are all merged.
- [agent] [agent-core] Opening the round of the next part right after a merge no longer writes `daemon_task_failed` for `round_wait` into the daemon's log: a round whose run is queued waits for that run, and only a round that waits for nothing is still reported.

## 0.48.0 — 21.11.2026 — PR #165

### New features
- [agent-core] [agent-runtime] `[plan] check_seconds` (default 120; `0` disables commands) and `[plan] check_refuse` (default `["agent gate", "make gate"]`) prepare plan step command limits and gate refusals. In the daemon, commands remain declared while existence checks continue.

### Enhancements
- [agent-runtime] Command-check output starts fresh on each attempt; preparation failures no longer name missing output files, and cleanup or output-read errors remain visible in mismatch diagnostics.

## 0.47.0 — 21.11.2026 — PR #164

### New features
- [agent] [agent-core] The pull request of each part of a split now says which split it belongs to: the original item and the part's letter, the other parts with their pull requests so far, who approved the split (the harness or you), and the tier the part asked for with the agent it was given. `agent status` shows the same in one line per part; what an old split did not record is shown as `not recorded`.
- [agent] [agent-core] When the team has no agent of the tier an item or a part asks for, you are asked once, in a questions file of the lead's task. A restart of the daemon, an answer or free text never asks again for the same wait; the question comes back only if the wait ends and a new one begins. An answer never lowers the tier.

### Enhancements
- [agent] [agent-core] The assigned agent of a part is kept across reviews and restarts, and the status reply carries it, with the waits for a missing tier and the asked marker, under a new socket protocol 6 that the CLI and the daemon negotiate (6, then 5, 4 and 3 as before): an older `agent` CLI keeps working against this daemon and only misses the new lines.
- [agent] [agent-core] The status replies of protocols 4 and 5 no longer carry the waits for a missing tier of 0.46.0, which an older CLI could not read.

## 0.46.0 — 21.11.2026 — PR #162

### New features
- [agent] [agent-core] An item and every part of a split are now given only to an agent of at least the tier they ask for (investigators above normal agents above workers; no `Tier:` and no tier in the split means normal). The first part of a split is handed to a fitting agent as well, not only to whoever was already running the item. A part for an investigator in a team with one investigator and one normal agent goes to the investigator, and a part for a normal agent never starts a worker.
- [agent] [agent-core] When the team has no agent of the tier an item or a part asks for, it waits and nothing is given lower. The wait is kept across a restart of the daemon, is logged once as a `team_changed` event with `kind = tier_wait`, and `agent status` shows it. The one-time question to you about it follows in the next version.
- [agent-core] The assignment text now says which tier was asked for and why this agent was chosen.

### Enhancements
- [agent] [agent-core] A fix pass after a review that asks for changes goes to an agent of at least the tier of its item or part, too: the coordinator keeps it when it qualifies, otherwise a qualified member does it on the same pull request and round, and with nobody qualified the item waits. `agent status` shows the wait of a ready later part before its round opens.
- [agent] A coordinator that is not of the tier of a part is no longer told to take that part over, and an unassigned coordinator is not given the delivery scope of a part.

## 0.45.0 — 21.11.2026 — PR #163

### New features
- [agent] [agent-core] [agent-runtime] `agent status <task>` and `agent status <round>` show plan steps with their marks and a done step's settlement: `confirmed`, `declared` or `mismatch`. The daemon checks `exists PATH` inside the worktree and retains judgements across restarts; steps without checks, reviewer checks and command checks remain declared.
- [agent-runtime] **Action:** history upgrades automatically to schema 17 while retaining older records. Keep a backup before upgrading if you may need to return to an older binary.

### Enhancements
- [agent] [agent-core] Plan status uses socket protocol 5. Older `agent` CLIs using protocols 3 and 4 still talk to the new daemon, without plan summaries; the new CLI falls back when talking to older daemons.


## 0.44.0 — 21.11.2026 — PR #161

### New features
- [agent] [agent-core] When the agent that took a task over goes bad too, or a lead given a member's work does, you are asked once in the task's questions file instead of the task just failing: answer `a` to start it once more in a new session, briefed; `b`, `ok` or anything else stops it — a member's work then goes to its lead. `[replacement] mode = "off"` asks nothing.
- [agent] [agent-core] [agent-runtime] Every hand-over of a task is logged as `task_handed_over`, with the agent it was taken from and the one that holds it now, for example `agent log --event task_handed_over`.

### Enhancements
- [agent] A restart of the daemon after a task was taken over goes on with the agent that took it, in its own session.
- [agent] [agent-core] A lead given a stopped member's work is told of it even when the daemon restarts, or the bus fails, before the lead was told.

## 0.43.0 — 21.11.2026 — PR #158

### New features
- [agent-core] A task-list item can name the kind of agent it needs with a line `Tier:` (`investigator`, `normal` or `worker`; an item without one asks for `normal`). An unknown tier is refused at its line. A list without `Tier:` lines reads as before. Nothing gives items out by tier yet.
- [agent-core] The choice of an agent by tier exists: it never picks an agent of a lower tier than asked (investigators above normal agents above workers), prefers a fitting member, takes the coordinator only when no member fits, and says when nobody fits. The daemon does not use it yet.

### Enhancements
- [agent-core] A recorded split plan can keep who approved it, the harness or you. **Action:** a plan written with this field cannot be read by an older `agent`, so keep a backup of `state.json` from before the upgrade; plans without the field load as before.

## 0.42.0 — 21.11.2026 — PR #160

### New features
- [agent] [agent-core] **Action:** a task whose new session goes bad too can now be taken over by another agent: list them in `[replacement] agents = ["claude-2", "inv-opus"]`. The first one that is free, not in the task's team and of the same or a higher place carries on in the same worktree and on the same branch, briefed on where the work stands, under the same name on the team's bus. `[replacement] replacements = 1` says how many times a task may be taken over.
- [agent] [agent-core] **Action:** a team member whose new session goes bad too, with no listed agent that fits — also when you list none — now stops, and the team's lead gets its work in a message on the bus, with its branch, its worktree and a briefing of what its runs left; the item is published when the lead ends `DONE`. Before, the member failed and you were asked. `[replacement] mode = "lead"` gives the work to the lead at once; `mode = "off"` keeps the old behaviour.
- [agent] [agent-core] `agent status` says from whom a task was taken over, for example `claude-2  queued, for claude-1`.


## 0.41.0 — 21.11.2026 — PR #159

### New features
- [agent-core] A pure table decides who takes over a task whose own agent cannot go on: the first idle agent of the same or a higher place (`[replacement] agents`, in the order you name them), otherwise the team's lead, otherwise you. The place of an agent is its tier: a worker below a normal agent below an investigator.
- [agent-core] The state keeps who held a task before the agent that holds it now, with the time, so that a task given to another agent still shows whose work it is. This part only decides and records; nothing acts on it yet. No migration: an older `state.json` loads as before.


## 0.40.0 — 20.11.2026 — PR #156

### New features
- [agent] [agent-core] [agent-runtime] A task whose session went bad goes on in a new session of the same agent: after the two retries of a run that ended without a `STATUS:` line, after the watchdog's last strike for a hung run and after the last turn-limit resume, the task is started once more in a fresh session instead of failing. Its first prompt is a briefing: why it was started anew, the task's whole prompt, the commits of its branch, the files it left uncommitted (kept in a snapshot first), its plan of steps and the messages of its team it had not read. When the new session goes bad too, the task fails as before.
- [agent-core] The new key `[replacement] new_sessions` says how many such new sessions a task gets: `1` by default, `0` keeps the old behaviour. The count is kept in `state.json`, so a restart of the daemon does not reset it.
- [agent] [agent-core] `agent status` says it on the task's line: `claude-1   queued, in a new session`.

### Bug fixes
- [agent] An item no longer stands behind a member the human stopped: when the lead ends `STATUS: DONE` with a passed gate, the item is published with the lead's branch.


## 0.39.0 — 20.11.2026 — PR #154

### New features
- [agent] [agent-core] [agent-runtime] When GitHub refuses to merge an approved pull request because it conflicts with its base, the lead of the round is sent to merge the base into its branch, not told to you at once. The result is gated and pushed, and the merge is tried again; a change to files the task does not allow is verified by the reviewers first, and `agent status` shows `waits for the lead's correction` while the pass is idle. The new `[merge] conflict_passes` setting (default `3`) limits these passes per round; when they are used up, or the lead gives no result, the lead asks you once in its questions file. `conflict_passes = 0` keeps the old behaviour.

### Enhancements
- [agent-runtime] A refused merge is told apart as a conflict or another refusal from GitHub's answer; a rate limit or a failing GitHub is still retried, not told.

## 0.38.0 — 20.11.2026 — PR #157

### New features
- [agent] [agent-core] [agent-runtime] A coordinator now proposes a split itself: it writes `split-proposal-<item>.json` in its task directory (the parts with their checks, dependencies, an estimate and a tier, and whether the scope changes) and ends its run with the line `SPLIT: 1.1`. A proposal that reads, has checks on every part, has no dependency cycle, stays within the size limit and keeps the scope is approved at once with no questions file, and its parts are recorded whole; the round goes on as the first part.
- [agent] [agent-core] A proposal with a fault goes back to the coordinator with its own text naming the part (does not read, a part without checks, a cycle, a part above the limit), at most `[split] corrections` times (default 2).
- [agent] [agent-core] You are asked once, in the task's questions file, when a proposal changes the scope and when the corrections are used up; the question says which condition failed and holds the proposal as the example of the verdict. A verdict `split` with the plan, as before, still works.
- [agent] A member that finds its item too big tells its coordinator, not you.
- [agent-core] New key `[split] unit` (default `"lines"`): the unit the size limit `part_limit` is applied to. Use `pages` or `images` for work that is not code. **Action:** none, the default is today's behaviour.

### Enhancements
- [agent-core] [agent-runtime] The history keeps a record of every decision on a split proposal. **Action:** the history database migrates to version 15 on the next daemon start; an older `agent` cannot read it, so keep a backup from before the upgrade.
- [agent] A restart of the daemon between a proposal and its decision, or between a correction and the next proposal, loses neither.

## 0.37.0 — 20.11.2026 — PR #155

### New features
- [agent-core] A split can now be written as a proposal: for each part its title, what it delivers, its checks, its dependencies, an estimate (a number and its unit, for example `900 lines`) and the tier of agent it asks for (`investigator`, `normal` or `worker`; none means `normal`), and whether the split changes the scope of the item. The approving verdict of today, without the new fields, is still read as before.
- [agent-core] The decision on a proposal exists: approve it with no human, send it back with the fault named by part (it does not read, a part without checks, a dependency cycle, a part above the limit), or ask the human when it changes the scope or the corrections are used up. The daemon does not use it yet.
- [agent-core] New keys `[split] part_limit` (the largest estimate a part may have, default 1500, `0` checks no limit) and `[split] corrections` (how many times a proposal goes back, default 2) in `agents.toml`; a file without `[split]` gets the defaults.

### Enhancements
- [agent-core] A proposal and its corrections are kept in `state.json` with the item, so a restart loses neither. The schema number stays 14 and the new fields are optional, so a stored split plan written by 0.36.0 loads, with its parts asking for `normal`. **Action:** a plan written by this version has fields an older `agent` rejects, so keep a backup of `state.json` from before the upgrade.

## 0.36.0 — 20.11.2026 — PR #153

### New features
- [agent] [agent-core] The coordinator's prompt now tells a lead when and how to report an item as delivered by another pull request of its team: the line `DELIVERED: #N` before the final `STATUS: DONE`, what the harness checks, and what a refusal means. A member is told what it means for its item.
- [agent] [agent-core] [agent-runtime] When the pull request named in an accepted report is closed without a merge, the item no longer waits for ever: the lead is told, the item goes back to its member in a new round and is published by its own pull request. `agent status` shows the old round as closed and the new one as open. A pull request that is closed and opened again before the daemon saw it closed is still waited for; one the daemon saw closed stays closed. A restart of the daemon at any point of this goes on from where it was. The returned item is published on a branch of its own, so it does not wait for the closed pull request.
- [agent] A failure to send the lead's notice no longer stops the daemon: it is logged as `bus_down` and sent at the next poll.

### Enhancements
- [agent-core] [agent-runtime] A closed delivering pull request is logged with its own `delivery_closed` warning event. **Action:** the history database migrates to version 14 on the next daemon start; an older `agent` cannot read it, so keep a backup from before the upgrade.

## 0.35.0 — 20.11.2026 — PR #152

### New features
- [agent] [agent-core] [agent-runtime] The daemon reads the plan a task keeps in `.agent/plan.md` at the end of every run and keeps each new version in the history; a missing plan and one that does not parse are recorded too.
- [agent] [agent-core] A solo agent, a member or a coordinator that ends `STATUS: DONE` while its plan is missing, does not parse, has a step still to do or in hand, or lost a step goes back to the same agent with the reason, at most `[plan] bounces` times per task (default 3, `0` only records).
- [agent] After that limit you are asked once, in the task's questions file, whether to go on as it is (`a` or `ok`) or send the run back once more.
- [agent] The first prompt of a solo agent, a member and a coordinator says how to keep the plan and its form, for example one step in hand and a step given up with its reason instead of deleted.

### Enhancements
- [agent] [agent-runtime] The plan stays out of git and out of the pull request: new worktrees ignore it, and a plan an agent commits by mistake is taken out of its branch by a commit "Keep the plan out of the delivery".
- [agent] Tasks that were open before this version are never sent back for their plan; their plan is recorded all the same.
- [agent-runtime] The history database migrates automatically to version 13 on the next daemon start.

## 0.34.0 — 20.11.2026 — PR #151

### New features
- [agent] [agent-core] [agent-runtime] The harness can check a coordinator's report that an item is delivered by
  another pull request of its team (`DELIVERED: #N`). It accepts it only when that pull request belongs to the team and
  the item's branch holds nothing outside it or the base branch; otherwise the lead is told why. An accepted item opens
  no pull request, `agent status` shows `delivered by PR #N, waits for its merge`, and when #N is merged the item is
  done as `done, delivered by PR #N`. A stop and a resume of the team in between change nothing: the item still
  waits, opens no second pull request and closes at the merge, also when it is a part of a split item. Coordinators are not told about the report yet:
  that and the return to work when #N is closed without a merge come in the next version.

### Enhancements
- [agent-core] The state file is now schema 14 and still reads every state written by earlier versions; an older `agent`
  cannot read a state written by this one, so keep a backup from before the upgrade.

## 0.33.0 — 20.11.2026 — PR #150

### New features
- [agent] [agent-runtime] When the daemon starts, every task whose run was cut by a crash or a stop of the daemon gets a snapshot of its worktree if its round is still open: all uncommitted, staged, deleted and untracked files (binary ones too; ignored files are left out) are kept as a commit under `refs/agents/snapshots/<round>/<task>/<n>`, without changing the worktree, its index or its files.
- [agent] `agent status <round>` shows the last snapshot of each task at the end of the round view, for example `snapshot task-… number 1 at 14:05:10, 4096 bytes`.
- [agent-core] [agent-runtime] The new optional `[snapshot] warn_mb` setting (default `200`) sets the size in megabytes above which a snapshot is logged with a `snapshot_large` warning, and the history database migrates automatically to version 12 on the next daemon start.

### Enhancements
- [agent-runtime] Snapshots are recorded in the history database (`snapshots`) and can be read back by round.

## 0.32.0 — 20.11.2026 — PR #149

### Enhancements
- [agent-core] The core can now read, write back and compare the plan of steps a task will keep in its worktree;
  nothing the daemon or the CLI does changes yet.

## 0.31.0 — 20.11.2026 — PR #148

### New features
- [agent] [agent-core] An explicit split approval records every part together. Each part gets its own round and pull
  request when its dependencies have merged; the item stays open until its last part merges, also after a restart.
- [agent] [agent-core] `agent answer` reports which split parts were recorded or why approval was refused, and
  `agent status` shows each part as done, in progress, waiting for a dependency, or to do.

### Enhancements
- [agent] [agent-core] The CLI negotiates socket protocol 4 for split details. Older clients keep protocol 3, and
  the new CLI falls back to protocol 3 when talking to an older daemon.

## 0.30.0 — 20.11.2026 — PR #147

### Bug fixes
- [agent] [agent-runtime] [agentbus] An agent that works in two open teams gets a message from a team, and wakes for
  it, only in that team: its session in the other team no longer reads it and acts on it, and reading in one team
  no longer marks the other team's messages as read. A running or configured agent is no exception.
- [agent] [agent-runtime] [agentbus] `agent say <task>` to a solo task reaches that task's session only, also when the
  agent is in an open team.
- [agentbus] A reservation belongs to the task that took it: the same agent's run in another team can no longer
  release it or free it when that run ends.

### Enhancements
- [agent] [agent-core] `agent run` says which of its agents are also in another open team, for example
  `codex-1 is also in the open team team-… (task task-…): its messages there do not reach this team`.
- [agent] [agentbus] A message from a team to a name that is not in that team is refused with the list of the team's
  members. The history of `read_messages` and `read_message` show a session nothing of the agent's other teams.
- [agent] Team members are told that only their own worktree and branch are theirs to change, and that a message
  naming another task's branch is not for them.

## 0.29.0 — 20.11.2026 — PR #146

### Enhancements
- [agent-core] Team state can retain complete split plans with lettered parts, deliverables, checks and dependencies.
  In this version nothing runs those parts and answers do not activate splits.
- [agent-core] **Action:** State files now use schema 13 and older states migrate automatically without changing
  their history. Keep a backup before upgrading: older binaries cannot read the upgraded state.

## 0.28.0 — 20.11.2026 — PR #145

### Bug fixes
- [agent] [agent-core] When a round has findings only you may allow and findings the team can fix, you are asked
  about your part before the team starts its next pass. Your question uses no team pass and does not give the team
  more passes than it had.
- [agent] [agent-core] What you allowed for a file is asked about again when the team changes that file afterwards.
  Allowing a protected file no longer allows a lint suppression or another finding of the team's added to it.

## 0.27.0 — 20.11.2026 — PR #143

### Bug fixes
- [agent] [agent-core] [agent-runtime] A pause of the network or of GitHub no longer stops the daemon: the round waits
  and the step is tried again by itself, 30 seconds after the failed call, then after a pause that doubles up to 10
  minutes, while the other teams go on.
- [agent] A failure of git while the daemon reads a worktree's commit (before a merge, after a fix, or the commit a
  reviewer verified) or prepares a second opinion no longer stops the daemon: the step waits.
- [agent-runtime] **Action:** The history learns the daemon's newest log events (it refused them before, and
  `build_directory_removed` on histories made before 0.19.0); `history.db` is migrated to version 11 at the start of
  the daemon.

### New features
- [agent] [agent-core] `agent status` says what a round waits for at GitHub, for example `waits for GitHub: refresh the
  description of the pull requests, tried 4 times, next try 01:07`; the log has one `github_step_waiting` warning when
  a step starts to wait and one `github_step_resumed` when it goes through.
- [agent] When GitHub or git refuses a step for good (a 404, a token that may not do it) you are told once while it
  waits, and `agent resume <team>` tries it again at once without starting any task of the team.

## 0.26.0 — 20.11.2026 — PR #142

### New features
- [agent] [agent-core] `agent run --allow-unpushed-base` permits an unpublished base explicitly, prints its
  commit count and first commit, and retains that evidence across daemon restarts in every pull request.

### Bug fixes
- [agent] [agent-runtime] `agent run` fetches the base from origin before starting and refuses unpublished
  commits or a failed fetch, so teams no longer inherit unpushed commits silently.

## 0.25.0 — 20.11.2026 — PR #141

### Enhancements
- [agent] **Action:** The first implementation is removed from the repository: the Bash `agent`, its unfinished Go
  version, the terminal panel and the old web interface (`agent live`, `agent web`), their tests and the installer.
  The Rust `agent` and `agentbus` are what is left. An installation of the old version can no longer be installed
  or updated from this repository; git's history has it.
- [agent] `make gate-full` checks `agentbus` and the Rust workspace only. `make unit-bats` does nothing and stays
  until the CI workflow stops calling it.
- [agent] The README describes the Rust `agent` alone: how to build it, set up its home and start the daemon.

## 0.24.0 — 20.11.2026 — PR #139

### Bug fixes
- [agent] A coordinator or member whose run sleeps can review another team while remaining excluded from its own.
- [agent-core] Reviewer stories no longer claim that deterministic assignments were random.

### Enhancements
- [agent] [agent-core] Reviews, verifications and second opinions choose investigators in a fixed order: first those who have not worked for the team, then those asked longest ago, with work for other teams breaking ties.
- [agent] [agent-core] A round without enough reviewers, a second opinion nobody can take or a failed reviewer nobody can replace says it waits for a free independent investigator.

## 0.23.0 — 20.11.2026 — PR #140

### New features
- [agent] [agent-core] `agent status --investigators` lists every configured investigator with its last two activities in the teams that are not closed, newest first, for example `task-…  reviewer  inv-opus  for team-…  done at 12:15:15, 2nd review finished`; one with nothing to do shows `free, no activity in an open team`.

### Enhancements
- [agent] [agent-core] A reviewer's line in `agent status` and `agent status --running` no longer says `waiting` between reviews: it says `working`, `done, 1st review finished 12:08:17`, or `stopped at 12:10:00, reason: the human stopped it`.

## 0.22.0 — 19.11.2026 — PR #137

### New features
- [agent] [agent-core] The daemon remembers each investigator's work for a team: crew membership, reviews, verifications and second opinions, including when they were requested, reported, stopped or failed.

### Enhancements
- [agent-core] **Action:** state files move to schema 11. Older files migrate automatically, rebuilding investigator activity history from the crew, reviews, fix passes and second opinions they retain.

## 0.21.0 — 19.11.2026 — PR #132

### New features
- [agent] [agent-core] When free disk space falls below `disk.low_free_gb` (default 100 GB), `agent status` shows one line with the free space and the limit, and you get one notification listing the five largest worktrees.
- [agent] [agent-core] **Action:** a new optional `[disk]` section with `low_free_gb` and `min_free_gb`; without it the defaults apply. When free disk space falls below `min_free_gb` (default 20 GB), `agent run` is refused with the free space and what to remove; queued rounds wait and go on by themselves when space is back.

### Bug fixes
- [agent] When the state cannot be saved, the daemon first runs the notification command with the reason, then stops as before; before, it stopped without telling anyone.

### Enhancements
- [agent] [agent-runtime] Build directories (`target/`) are removed when their round closes, and a reviewer's is removed at the end of its run when free space is low, so builds no longer fill the disk.
- [agent] The findings comment of a pull request is a list with counters, and closed findings are folded in one block, instead of a table.

## 0.20.0 — 19.11.2026 — PR #136

### Bug fixes
- [agent] A reviewer's report that is right for one item of a pull request and wrong for another no longer stops the
  daemon: the report goes back to the reviewer with the refused line and the answers allowed there, and nothing of it
  is recorded.
- [agent] A lead's explanation of a finding is kept for every item of the pull request, also for an item that joined
  it later; before, it was kept for one item only and the verifiers' answers were refused for the other.
- [agent] [agent-core] At its start the daemon repairs a state in which the items of one pull request disagree on a
  finding, and logs each repaired finding as `restart_corrected`.
- [agent] After a restart, a reviewer whose report was not taken is asked again, and a round no longer says it
  waits for an answer of the human because a reviewer that was replaced had failed.

### Enhancements
- [agent] [agent-core] The verifier's brief names beside each open finding the answers it may get, for example
  `ANSWER: FINDING 28: ACCEPTED — why or REJECTED — why or RAISED [BLOCKER] — why`, and the report is read by the
  same list.

## 0.19.0 — 19.11.2026 — PR #134

### New features
- [agent] [agent-core] A task list can start with lines `Allow: <path or glob>`, for example
  `Allow: tests/golden/*.txt`. The checks before a review no longer ask you about changes to those paths.
  `AGENTS.md`, `CLAUDE.md` and `.github/workflows/` can never be allowed this way: `agent run` refuses such a task list
  and starts nothing.
- [agent] `agent run` prints the allowances when the team starts, and the pull request lists what the round changed
  under them in `## Allowed beforehand`.

### Enhancements
- [agent-core] Moving a file from one protected place to another now asks about both places, not only the old one.
- [agent-core] The state file is migrated to the new version when it is read; a brief that started with an `Allow:`
  line before is kept as it was.

## 0.18.0 — 19.11.2026 — PR #133

### New features
- [agent-core] `[build] incremental = true` in the configuration turns incremental compilation back on for agents
  and the gate.

### Enhancements
- [agent] [agent-core] [agent-runtime] [agent-web] The build directory of a task's worktree is about three times
  smaller (13 GB → 4 GB after one gate): less debug information in development and test builds.
- [agent] [agent-runtime] Agents and the gate build without incremental compilation.

## 0.17.0 — 19.11.2026 — PR #131

### Bug fixes
- [agent] A message sent to a lead during a fix pass no longer loses the lead's fix response.
- [agent] [agent-core] A lead is woken at most twice for the same message, also after a restart of the daemon.

## 0.16.0 — 19.11.2026 — PR #130

### Bug fixes
- [agent-core] A reason after `RESOLVED` or `FIXED` is accepted, for example `FINDING 3: FIXED — moved the check`;
  before, such a line was refused and the round stood still.
- [agent-core] A lead's answer to a finding that does not block (`[MINOR]`) is accepted instead of refused.

### Enhancements
- [agent] [agent-core] A correction sent to an agent quotes the refused line and says what is wrong with it.
