# Round accounting — inv-opus (claude/opus)

Round: harness/round-accounting

Branch `agents/inv-opus/20261109-212035`, from base
[`45f1e7a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/45f1e7a29ebd2797d3112e39e129bfe938236604).
Both parts of the topic are implemented, test first, and pass `agent gate` (`8/8` steps). No PR was opened (Phase 1).

## Commits

Part 1 — the round ID, a team's tasks, the report by round (`615` lines, `+561` / `−54`):

- [`6da11cf`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6da11cfa1c625cf868fce945d42cb0a62e3ee643) — $\color{red}{\textbf{RED}}$: `tests/unit/round_id.bats`, `tests/unit/tokens_round.bats` (`14/14` fail)
- [`8cf6351`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8cf6351478b1519c3465ce8fcc9ce74d0b6de198) — $\color{green}{\textbf{GREEN}}$: gate PASS recorded at this commit

Part 2 — sub-rounds per run, the nodes' future work (`226` lines, `+219` / `−7`):

- [`506f495`](https://github.com/mateusz-szczyrzyca/team-agents/commit/506f495f3136fe2bccc1d160ce051c854351553b) — $\color{red}{\textbf{RED}}$: `tests/unit/round_runs.bats` (`6/6` fail)
- [`f070ce1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f070ce1746e2aef451794db27221a138157b42cb) — $\color{green}{\textbf{GREEN}}$: gate PASS recorded at this commit

The topic asks for two PRs: Part 1 is the first two commits, Part 2 the last two. Part 2 is stacked on Part 1, so the
merger can open the first PR from
[`8cf6351`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8cf6351478b1519c3465ce8fcc9ce74d0b6de198) and the
second from the branch head once the first is merged.

## What was done

### Part 1

- **`round_id_of`** (`bin/agent`) reads the `Round:` line of a task text: none → `?`; one valid line → its ID, with
  blanks (and a CR) trimmed; a second `Round:` line, or an ID that breaks `[A-Za-z0-9._-]+(/[A-Za-z0-9._-]+)*`, returns
  `1` and names the wrong line (`line 3 of the text, "Round: R13", is a second Round: line (the first is line 1)`).
  Only a line that *starts* with `Round:` counts.
- **Refusal before anything starts** in every `agent run` form: solo and `--team`, `--investigation` (in
  `validate_investigation_topic` and again in `cmd__investigation_team`), `--resume`, and `--need` / `--on`.
- **`ROUND_ID` at creation, every site.** The six places that wrote `ROUND=0` now write `ROUND_ID` on the same line:
  - `cmd__investigation_team` (the investigation team's parent): from the topic.
  - `start_investigation` (single investigations and `--verify` reviews): from the requesting task.
  - `start_helper` (workers): from the requesting task.
  - `create_task`: a new optional 9th argument, the round (default `?`). Its callers: `agent run` (the text); the
    investigation phases 1, 2 and 3 (the investigation team's round at that moment); the advisory reviews of an
    investigation resume (the resume text's round, else the investigation team's); a node job (`job_round_id`, below).
  - `resume_run`: the human's addition's `Round:` line, else the previous team's (its coordinator's task's, else the
    first that has one; `?` for none). Replacements after a failover come through this path (a hand-back resume).
  - `remote_member_task` (the lead's proxy of a remote member): a new optional 8th argument.

  A coordinator taken over (`coordinator_takeover`) creates no task, so nothing to record there.
  A source test in `tests/unit/round_id.bats` fails if a line that writes `ROUND=0` does not also write `ROUND_ID`.
- **`team_task_closure`** replaces the one level of `PARENT_TASK` in `agent tokens <team>`: one `grep` over all
  `meta.env` files builds the links, then a breadth-first walk with a visited set (a cycle ends where it closes).
  A task whose `TEAM` is another team is never taken in. When no task has `TEAM=<id>` but `<id>` is itself a task
  (an investigation team, `…-investigation-team`), the walk starts from it: `agent tokens <investigation-team>` used to
  die with "no task of team".
- **`task_role`**: `review` (`REVIEW=1`), `worker` (`KIND=helper`), `investigation` (an investigation or an
  investigation team), `coordinator` (the team's lead task, from the bus, looked up once per team), `member`.
- **`tokens_table`**: the existing columns unchanged, `ROLE` and `ROUND` appended (`-` for a task without `ROUND_ID`).
  `agent tokens --round <id>` selects the tasks with `ROUND_ID` `<id>` or `<id>/…` (literal comparison, so `R12` takes
  `R12/A/#74` but not `R120` or `R1`), and prints `role <name>` and `model <name>` sums before the `total` row.
- `tests/unit/tokens.bats` passes unchanged.

### Part 2

- **The run marker.** The runner writes `▶ run 7 (resume) round R12/A pr #74` (or `pr -`). Every parser of that line
  is anchored at its start and reads only the run number and the mode — `runs_usage` (usage per run, the fresh-session
  rule, `agent tokens`, `agent spend`) and `run_days` — and the tests that build or grep the line (`spend.bats`,
  `fresh_relative.bats`, `fresh_member_done.bats`, `debug_agent.bats`, `team_pause.bats`, `tokens.bats`) match a
  prefix. The live panel, the web interface and the Go code (`agentbus`, `live`, `agent/`) do not parse it.
- **`run_pr`** decides the PR of a run: the task's own `PR_NUMBER`, else its team's round PR (`round_pr_of_team`), else
  its `ROUND_PR`, else `REQUESTER_PR` — new in the `meta.env` of workers, investigations and reviews: the requester's
  `run_pr` when the task is made, so a worker made while its requester was on `#74` stays on `#74`.
- **`run_subrounds`** labels each run's usage from its marker; `agent tokens --round` prints `sub-round <label>` sums.
  A run without a marker (made before, or usage before the first run line) counts under `<ROUND_ID>/?`; the total is
  unchanged.
- [`docs/node-improvements.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f070ce1746e2aef451794db27221a138157b42cb/docs/node-improvements.md),
  linked from a new section "Still to be made" of
  [`docs/multi-instance.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/f070ce1746e2aef451794db27221a138157b42cb/docs/multi-instance.md).

## Interpretations the merger should know

- **A solo task's role is `coordinator`.** The topic's roles have no case for a task without a team; a solo task leads
  its own work and asks for its own workers.
- **A hand-back never decides the round.** Fix passes, checks before the review and failover replacements resume a
  team with the system's text (`AGENT_HANDBACK_LEAD` is set): a review quoted in a fix pass can contain a line such as
  `Round: 2 of 3`, which would otherwise refuse the resume and stop the autopilot. Those resumes inherit the previous
  round and do not parse the text. Tested (`a hand-back's text … never decides the round`).
- **`?` and `-` differ.** `?` is a task made after this work whose text had no `Round:` line; `-` in a report is a task
  made before (no `ROUND_ID`). `--round '?'` selects the first kind only; old tasks are not labelled.
- **A node job is not refused for its round.** The lead's `agent run` already refused a broken text; on the node,
  `job_round_id` takes `?` and logs `job_round_unreadable` for a text it cannot read. A resumed remote member's text is
  the old text plus the human's addition, so an addition with its own `Round:` line makes the node's round `?`. Listed
  in `docs/node-improvements.md`, with the remedy (the round as a field of the job).
- **Investigation resume.** `agent run --resume <investigation-team> "…Round: R13…"` stores the round as
  `INVESTIGATION_RESUME_ROUND_ID` on the parent; the advisory reviews take it, else the parent's round. The parent's own
  `ROUND_ID` is not changed.
- **Sub-round label before a PR:** `<round>/(no PR)`, as the topic's Terms say; the marker itself writes `pr -`.

## Checks

```bash
agent gate --wait          # at 8cf6351 and again at f070ce1
```

- `gate: all 8 steps passed` at both commits (`check-generated`, `lint-shell`, `lint-go`, `lint-docs`, `unit-go`,
  `unit-bats`, `harness`, `unit-agent-bats`); `0` lint findings.
- New tests: `10` in `round_id.bats`, `4` in `tokens_round.bats`, `6` in `round_runs.bats` — `20/20` pass;
  `tokens.bats` `5/5` unchanged.
- `agent round-size 20261109-212035-inv-opus`: `829` lines in all (`+774` / `−55`), within the soft limit.

## Compatibility (AGENTS.md rule 9)

- `meta.env`: new keys `ROUND_ID`, `REQUESTER_PR`, `INVESTIGATION_RESUME_ROUND_ID`; old tasks without them read as
  before (`-`, `?/?`, the parent's round).
- `activity.jsonl`: the run start line gains a suffix; old lines without it are still parsed (tests: `runs without a
  marker count under <round>/?`, and every existing test that builds `▶ run N (mode)` lines).
- `agent tokens <team|task>`: same columns in the same order, two appended; the `total` row unchanged. The team's
  tasks are now the closure — a team's report can include more tasks than before (that is the fix).
- `agent run`: a text with two `Round:` lines or a broken ID is now refused. A text that used `Round:` at the start
  of a line for something else (`Round: 2 of 3`) is refused too — said in the README.
- New log event `job_round_unreadable` (warn).
- `tests/agent-go-map.tsv` gets `20` rows for the new tests (round `R53`, beside `tokens.bats`); without them
  `check-agent-test-map` fails.

## Lint suppressions

None.

## Skipped tests

None.

## Test changes after a RED commit

Two of my own new tests were corrected in their GREEN commits, and the commit messages say so:

- `round_id.bats`: the worker and review tests read `meta.env` while it was still written field by field; they now
  wait for the task's status first (a race in the test).
- `round_runs.bats`: the first test checked `agent tokens --runs` for a run with no usage — the fake CLI reports none,
  so the test now appends a usage line first.

No existing test was changed.

## Not done

- The panel's view of rounds, synchronising usage from nodes, the round's life cycle, labelling old tasks — out of
  scope by the topic.
- `agent spend` and `agent debug` were not given round columns. `agent debug` shows the last `15` lines of
  `agent tokens <team>`, which now carry `ROLE` and `ROUND`.

## Suggestions for AGENTS.md (not changed: rule 2)

- Under "Tests" / bats: a test that reads a new task's `meta.env` must wait for a field written last (`STATUS`), since
  the script writes the metadata one `meta_set` at a time. Both of my races were that.
- Under "Tests": every new bats test needs a row in `tests/agent-go-map.tsv`, or the gate fails
  (`tools/check-agent-test-map.sh`, run by `tools/check-agent-parity.sh` in `unit-agent-bats`).

There are no agentbus tools in this run, so nothing went to the repository notebook.
