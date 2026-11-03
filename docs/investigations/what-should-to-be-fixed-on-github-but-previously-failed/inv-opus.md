# What should be fixed on GitHub, but previously failed — inv-opus

Base: [`d3f5334`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d3f5334ea193656111b78277a3046a02d00c1d23)
(PR #49 merged). Branch: `agents/inv-opus/20261103-161254`. Each change is test first (a RED commit with the failing
run, then GREEN). `make gate`: 7/7 on the final commit. One question is open (below).

## Why points 1, 3 and 4 still failed: the installation runs the code from before PR #49

PR #49 (colours, recommendation layout, review pairs) was merged at 12:54 UTC. The installation was not updated after
that. Everything seen on PR #50 came from the old code:

| What runs | Evidence (read only) |
|---|---|
| `~/.local/bin/agent` | dated 04:33 UTC, 8150 lines. `main` has 8862; the old one has no `gh_colour_names` (no colours at all) |
| `~/agents/prompts/` | dated 04:33 UTC. `review.md` differs from `main` and has no `## Recommendation` rule, no ✅/🔧 colours |
| `~/agents/reviews/history` | missing: the old script never writes the review history |

What that produced on PR #50:

- **The same pair twice (point 4).** Pass 0: reviews #123/#124 by `inv-astra` + `inv-opus` (13:43 UTC). Fix pass 1:
  reviews #125/#126 by `inv-astra` + `inv-opus` again (14:52 UTC). The old `AGENT_REVIEW_DRAW=random`, which
  `~/agents/agents.conf` sets, keeps a PR's pair **on purpose** ("each re-checks its own findings"). `main` no
  longer does.
- **Plain names (point 1).** The PR's attribution (`- coordinator: \`codex-1\` …`) and the bylines are plain: the
  old `gh` wrapper colours nothing.
- **"Recommendation: **needs changes**" (point 3).** inv-astra wrote that in its own words. The installed prompt asks
  for no colour.

**To do (you):** update the installation from `main` once this work is merged (`./install.sh`). I may not do it
(AGENTS.md rule 3). Without the update, none of the fixes below (or in PR #49) take effect.

## What was still wrong in `main` — fixed here

| # | Point | Problem in `main` | Fix (commits) |
|---|---|---|---|
| 1 | Colours the same in every round | A colour came from the name's position in `NORMAL_AGENTS` / `WORKER_AGENTS`. Your `agents.conf` redefines `NORMAL_AGENTS` (line 231: `(codex-1 gemini-1)`, dropping `claude-haiku`), so a roster change gave the names other colours. Investigators also shared colours with team agents (`inv-astra` = `claude-haiku`'s colour) | `~/agents/github-colors` remembers each name's colour for good. The runner registers every name a run can publish (all investigators, the run's agent and its team). A new name gets the first palette colour nobody has, then the least used one, under a lock. Three more default colours (10). `795a83a`, `5c04281` |
| 3 | ✅ green / 🔧 red, bold | Only a prompt rule; a model writes its own words | The agents' `gh` rewrites the recommendation line of a review it publishes: "ready to merge" → ✅ $\color{green}{\textbf{Ready to merge}}$, "needs changes" / "needs fixes" → 🔧 $\color{red}{\textbf{Needs changes}}$ (🧩 when incomplete). These are the notifications' icons. PR comments and other lines are left as they are. `9adbcd7`, `598388c` |
| 4 | A, B, C, D: A+B, C+D, then mixed pairs | `random` (your mode) skipped only the last two pairs and drew among ties, so a PR's 5th pass could return to A+B while A+D and B+C had never reviewed | `random` draws only among the least recently used of the best pairs: the six pairs come one each before any repeats. With A+B first: C+D, then A+C, B+D, A+D, B+C in some order. `rotate` already did this. `6547979`, `6e3df0c`, `8813a64` |
| — | Found on the way | `agents_gh_env` replaced the `gh` link with `ln -sfn` (remove, then create). Two runs starting together, like a review pair's two reviewers, raced. One got "Already exists" and its runner died before the agent started ("the agents' gh … could not be set up") | The link is made under its own name and renamed over the old one; it is left alone when already right. `54e13c2`, `b937f3f` |
| 5 | Serena for all CLIs | not configured | New setting `AGENT_SERENA` (Serena's command; empty = off). Every run gets an MCP server `serena` on **its own worktree**: claude (`mcp.json`, also for independent investigators, still without agentbus), codex (`-c mcp_servers.serena…`, 120 s start-up), Antigravity (`.agents/mcp_config.json`), opencode (config). Each uses Serena's context for the client: `claude-code`, `codex`, `antigravity`, `ide`. `.serena/` is excluded from git; a prompt part (`prompts/serena.md`) says to use it first and to fall back to the shell. `64c2ffa`, `3981c9a` |

Point 2 (file links) was not marked "not done". `main` has it as a writing rule (`prompts/writing.md`), but the
installed prompts predate it, so it takes effect only after the update. GitHub turns commit hashes and `#12` into links
on its own, never file paths, so the model must write the link; the rule says how.

## Serena — what you need to do and know

- **Turn it on:** add `AGENT_SERENA="serena"` to `~/agents/agents.conf`. `serena` is in `~/.local/bin`, which is on
  your `RUN_PATH`. I may not edit `agents.conf`.
- **Checked here:** Serena 2.1.0 starts on a clone with `start-mcp-server --context claude-code --project <dir>`
  and answers `tools/list` over stdio. With your global `agent_interface: REPL` its tools are `initial_instructions`
  and `serena_repl`; the prompt therefore names no tool except `initial_instructions`.
- **Why the worktree and not `/home/dev/projects/team-agents`:** that checkout is the project Serena indexed, but
  agents work on their own branches in `~/agents/worktrees/…`. Serena on the checkout would show them other code.
  A worktree is indexed when Serena first needs it, so a run's first searches may be slower.
- **Side effect:** Serena adds every project it opens to `projects:` in `~/.serena/serena_config.yml`. My probe added
  `/tmp/serena-probe`; I removed it again with `serena project remove` (the list is as before: your checkout only).
  With Serena on, every task's worktree will be added there. Clean up with `serena project remove <path>`, or ask for
  that to happen when a worktree is removed (not done here).
- **Not verified with the real CLIs** (AGENTS.md rule 4): whether each CLI starts the server and uses it. The
  generated configurations are covered by tests.

## Open question (`questions-1.md`)

One way a PR's pair can still repeat in `main` and on this branch: when the only free investigators are exactly the
PR's previous pair (C and D busy or out of quota), the pair waits `AGENT_REVIEW_ROTATE_WAIT_MIN` (20 min), then
reviews again (`review_pair_repeated`). The existing test `review_pair_repeat.bats:64` requires this, so changing it
needs your permission (rule 1). Recommended: never repeat with 3 or more investigators; wait until any of the
others is free.

## Not changed

- A `--verify` review (one investigator, not a pair) can still go to the same investigator for its re-review.
- `review_pair_random.bats` still has a test named "a PR's next review pass keeps its pair". It passes because its
  fixture writes no verdict. Its name states the rule you have forbidden; whether it should go is your call (rule 1).
- Existing GitHub texts are not rewritten.

## How this was checked

- New tests: `review_pair_cycle.bats` (4), `github_name_registry.bats` (6), `github_recommendation.bats` (6),
  `agents_gh_race.bats` (1), `serena.bats` (8), all listed in `tests/agent-go-map.tsv`. No test from `main` was
  changed. `8813a64` makes one check of this branch's own `review_pair_cycle.bats` effective: a bare `! cmd` never
  fails a Bats test.
- `make gate` 7/7 after each GREEN commit, run with `TMPDIR=~/.cache/inv-opus-tmp` and `BATS_JOBS=8`. Two reasons:
  - The shared `/tmp` ran out of quota ("Disk quota exceeded"). Other agents' bats runs left large `bats-run-*`
    directories there (one of 8 GB); I did not touch them.
  - With 32 jobs on a host at load ~24, three unchanged timing tests failed once each and passed alone:
    `pause_notify.bats:22`, `delegate_jobs.bats:241`, `restart.bats:67`.
- The race test failed 3 of 3 before the fix: a slow `ln` makes the moment the same for every run.

## Suggestions for `AGENTS.md` (not changed — rule 2)

- Bats: `! cmd` never fails a test (errexit ignores it); use `run ! cmd` or `run cmd` + `[ "$status" -ne 0 ]`.
- The bats suite needs a few GB of temporary space: when `/tmp` is short, run `TMPDIR=<disk dir> make gate`.
- After a merged change to `bin/agent` or `prompts/`, the installation must be updated (`./install.sh`) before its
  behaviour can be judged on GitHub. A note in the PR description template would avoid reports like this one.
