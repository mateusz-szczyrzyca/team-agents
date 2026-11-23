# What is new in `agent`

Newest first. One entry per version, written by the pull request that raises it — the rule and the form are in
[`AGENTS.md`](AGENTS.md), "Commits and pull requests". Every line names in brackets the crates or modules the
change is in: `[agent]`, `[agent-core]`, `[agent-runtime]`, `[agent-web]`, `[agentbus]`. The entries start with
0.16.0; what came before is in the merged pull requests.

## 0.78.0 — 23.11.2026 — PR #224

### New features
- [agent] [agent-core] `agent status` names a task that failed twice without using tools. The human notice names the agent, CLI and model and asks that its configuration be checked before it receives more work.

### Bug fixes
- [agent] [agent-core] [agent-runtime] When the first run of a task or new session reports `STATUS: FAILED` without a tool call, the daemon tries once in a fresh session and explains where to find the bus tools. This applies to every role and survives a daemon restart.
- [agent] A failed member wakes its coordinator with the item and its last words. The coordinator may wait for the human and may take over only when its role permits it; a successful startup retry sends no failure notice.

### Enhancements
- [agent-core] **Action:** Saved state migrates from schema 14 to 15 to remember tool use and the startup retry. Keep a backup before upgrading if you need to return to an older daemon. Older status clients retain their negotiated format.

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
- [agent] [agent-core] **Action:** The daemon removes teams with no work for two hours, checking at startup and once a minute. This release removes idle teams without starting replacements; automatic replacement follows in the next part. Set `[limits] idle_team_minutes = 0` to keep the previous behavior. Running or queued work, rounds waiting for a free investigator, human pause or stop, provider reset waits and another team's pull-request merge protect a team; a sleeping team waiting for review verdicts or an open human question alone does not.
- [agent] [agent-runtime] Before automatic removal, history records the last activity, items, split parts, round states and branches. Removal keeps the command's bundle safeguards and notification. A refusal keeps the team and names its branches in one notification; the daemon retries once a minute while the team remains idle, without repeating the refusal notice.

### Enhancements
- [agent] [agent-core] Idle observations survive a daemon restart. Removal counts from the later of recorded activity and current daemon startup, so downtime never counts and an old team gets a full configured idle interval after restart. Keep a state backup before upgrading if you need to return to an older daemon, which does not recognize the new observation field.

### Configuration
- [agent-core] new: `[limits] idle_team_minutes` (default `120`) — minutes without work while the current daemon is running before automatic removal; downtime does not count and `0` disables it.
- [agent-core] new: `[limits] idle_team_restarts` (default `1`) — reserved for the next part's replacement limit; it has no effect in this release.

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
- [agent] [agent-core] **Action:** `[replacement] rejections = 5` is the limit the count is shown against; `0` shows the count with no limit. This version only counts and shows: it does not yet replace the agent at the limit.

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
  Automatic execution of those parts will follow in the next part; answers do not activate splits yet.
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
