# Continuation of task `20261103-161254-inv-opus` — inv-opus

Branch: `agents/inv-opus/20261103-173238`. It starts with the `12` commits of `agents/inv-opus/20261103-161254`,
fast-forwarded unchanged, so it holds the whole result. The earlier report is
`docs/investigations/what-should-to-be-fixed-on-github-but-previously-failed/inv-opus.md`.

## Why the task stopped

- Run 1 finished its work, pushed it and asked one question (`questions-1.md`). It ended with `STATUS: QUESTIONS`,
  so it was waiting, not stopped.
- You answered **a)**. Run 2 resumed the same Claude session and exited after `16` s with
  `Error: Session ID 2954eff9-… is already in use.` (`err-2.log`). The runner recorded that as "the independent run
  ended without DONE", and the investigation failed.
- The session was taken because the runner had just started a *fresh* session for that run
  (`fresh_session`: "your last run read some 151690 tokens"). The fresh session reused the old session's ID.
  The bug is in the runner's fresh-session start. I did not investigate it further; it is worth its own task.

## What was done now: answer a) to question 1

The PR's previous pair no longer reviews it again when there are `3` or more investigators. When only that pair is
free, the review keeps waiting, however long that takes. The supervise timer keeps dispatching it, and the first
free other investigator forms a partly new pair at once (A+C, …). The `AGENT_REVIEW_WAIT_MIN` notification tells you
once that it waits. Unchanged:

- One of the last two pairs of *other* PRs is still taken after `AGENT_REVIEW_ROTATE_WAIT_MIN` (`random`,
  `rotate`), logged as `review_pair_repeated`.
- With `2` investigators their pair still reviews every time.

| Commit | What |
|---|---|
| [`828ab78`](https://github.com/mateusz-szczyrzyca/team-agents/commit/828ab78adf0245efd7251df4ee956a3cf96500dc) | RED: `4` new tests in `tests/unit/review_pair_repeat.bats` (`4` investigators a day later, `3` investigators then the third free, `rotate` and `list`, the timer keeps the pass pending). The approved change to `review_pair_repeat.bats:64` |
| [`6b5ca3a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6b5ca3a9ff9dfc4ecb91643b694a19c452529651) | GREEN: `review_pair_next` in `bin/agent`; README, `config/agents.conf` |

## Existing test changed (with your verdict)

- `tests/unit/review_pair_repeat.bats:64`: its last assertion expected `inv-a inv-b` after the wait. It now expects
  no pair.
- I also renamed the test, from "… it waits AGENT_REVIEW_ROTATE_WAIT_MIN, then repeats it" to "… it waits, and does
  not repeat it after AGENT_REVIEW_ROTATE_WAIT_MIN", together with its line in `tests/agent-go-map.tsv`. The old name
  stated the forbidden rule. Revert the rename if you want only the assertion changed.

## Compatibility

- No file format, subcommand output or setting changed.
- `AGENT_REVIEW_ROTATE_WAIT_MIN` no longer applies to a PR's own pair. The README and `config/agents.conf` say so.
- Risk (as in the question): if every other investigator stays busy or out of quota for hours, the PR's review
  waits that long.

## Checks

- `make gate`: `7/7` on [`6b5ca3a`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6b5ca3a9ff9dfc4ecb91643b694a19c452529651). Run with `TMPDIR=$HOME/.cache/inv-opus-tmp BATS_JOBS=8`, and with the runner's
  `AGENT_*` variables unset (next point).
- **Found:** the bats suite inherits the runner's environment. This run gets `AGENT_GITHUB_NAME_COLORS`,
  `AGENT_GITHUB_COLORS`, `AGENT_REVIEW_BYLINE`, `AGENT_REAL_GH` … from `agent _runner`. With them set,
  `review_byline.bats:26` fails, on the base `d3f5334` too: the agents' `gh` colours the name in the byline. Every
  agent that runs `make gate` inside a run can hit this. Fix (a test-helper change, so not mine to make under rule 1):
  `tests/unit/helpers.bash` should unset every `AGENT_*` variable of the runner before each test.
- Side observation: this run's `AGENT_GITHUB_NAME_COLORS` has `codex-1`, `gemini-1` and `deepinfra-qwen`, but no
  investigator. That fits the installation still running the code from before this branch (earlier report). On this
  branch the runner registers every investigator.

## Still to do (you)

- Merge, then update the installation (`./install.sh`). Until then none of these fixes run.
- Turn Serena on with `AGENT_SERENA="serena"` in `~/agents/agents.conf` (earlier report).
- Look at the fresh-session ID reuse that stopped run 2 (above).

## Suggestions for `AGENTS.md` (not changed — rule 2)

- Bats tests must not depend on the caller's `AGENT_*` environment: an agent's `make gate` runs inside a runner.
