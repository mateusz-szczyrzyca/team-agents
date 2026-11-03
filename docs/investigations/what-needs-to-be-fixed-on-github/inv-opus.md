# What needs to be fixed on GitHub — inv-opus

Base: [`9575b28`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9575b28cb48d9e96b91644b8c882e426c254d435).
Branch: `agents/inv-opus/20261103-123532`. All six points have code or prompt fixes on this branch, each written
test first (a RED commit with the failing run, then GREEN). `make gate`: all 7 steps pass.

## Status per point

| # | Point | Status | Where |
|---|---|---|---|
| 1 | Names bold, one colour each, the same in every round | fixed | `bin/agent` (`gh_colour_names`, `agent_github_colors`), `tests/unit/github_names.bats` |
| 2 | File names in GitHub texts as links | fixed in the prompts (GitHub has no auto-link for paths) | `prompts/writing.md`, now also given to reviewers and independent investigators |
| 3 | Recommendation first; green/red bold with the notification's icons | fixed in the prompt | `prompts/review.md` |
| 4 | The same pair reviews a PR again | **fixed** (critical) | `bin/agent` (`start_review_pair`, `review_pair_next`), `tests/unit/review_pair_repeat.bats` |
| 5 | No "Generated with Claude Code" | fixed | `run_claude` (`--settings`), the agents' `gh`, `prompts/writing.md` |
| 6 | Inconsistencies of the merged prompt report | 9 of 12 fixed (4 before this task, 5 here); 1 left (item 10) | see below |
| 7 | Mergeable to `main` | yes: `make gate` passes; no existing test changed | — |

## 4. Repeated review pairs (critical)

**Cause.** The installation runs the default `AGENT_REVIEW_DRAW=random` (`~/agents/agents.conf`). In that mode
`start_review_pair` *deliberately* kept a PR's pair for its next review pass ("each re-checks its own findings").
The `rotate` mode added before did not help: it is not the default, and the installed `agent` does not have it yet.
The installation's records (read only) show the result. PR #47 was reviewed by `inv-gemini-argon` + `inv-opus` at
11:05 and again at 11:39. PR #43 was reviewed by `inv-astra` + `inv-kimi` twice.

**Fix.** It applies in every mode:

- **The PR's last pair is avoided.** A pair whose reviews of the PR have ended (a verdict of its last pass is in
  `reviews/<repo>-<pr>/`) does not review that PR again while another pair can.
- **`random` reads the history.** It never takes one of the last two pairs and prefers the fewest members of the
  last pair. Then it draws at random. With A, B, C, D this gives **A+B, then C+D, then mixed pairs** (A+C, B+D, …):
  the human's example.
- **`rotate` also avoids the PR's own pair**, even when other PRs were reviewed since.
- **`list` takes the first free investigators** outside that pair.
- **Waiting.** When only a busy investigator makes such a pair, the pair waits `AGENT_REVIEW_ROTATE_WAIT_MIN`
  (20 min), as `rotate` already did; the supervise timer dispatches it. After that it repeats the pair and logs
  `review_pair_repeated`. With two investigators their pair reviews at once.

**Tests.** `tests/unit/review_pair_repeat.bats` has 8 tests. They cover the human's A/B/C/D sequence for one PR
and across PRs, the wait and its fallback, one busy investigator, two investigators, `rotate`, `list` and the timer.

**One thing to decide.** The existing test `review_pair_random.bats` — "a PR's next review pass keeps its pair" —
pins the old behaviour. Its fixture never writes a verdict. The new rule keeps the pair only for a pass whose
reviews never ended (that pair has not reviewed yet), so the test still passes unchanged. Its *name* still states
the rule the human has now forbidden. Rule 1 of `AGENTS.md` keeps me from renaming or removing it: decide whether
it should go.

Not changed: a single `--verify` review, which has one investigator. Its re-review can still go to the same
investigator.

## 1. Names in colour

GitHub has no coloured text in Markdown except through math. `$\color{#8250DF}{\textbf{gemini-1}}$` renders bold
and coloured. Checked offline with MathJax 3 (`mathjax-full`, which GitHub uses): hex colours work, the
`\color[HTML]{…}` form does not.

- **Where the colouring happens.** The prompts and existing tests keep the plain names (`review_byline.bats` and
  `skips.bats` pin them). The agents' `gh` colours the names as it publishes. It covers the attribution lines
  (`- coordinator:`, `- member:`, `- agent:`) and the review byline. It works on `pr|issue create|edit|comment`
  (`--body`, `--body-file`) and on `api …/pulls|issues…` with `--input`, the review included.
- **Same colour in every round.** The colours come from `AGENT_GITHUB_COLORS` (new setting, 7 colours, `none`
  turns it off). They follow the configuration's order: `NORMAL_AGENTS`, `WORKER_AGENTS`, the others; the
  investigators are counted apart. The runner exports the map as `AGENT_GITHUB_NAME_COLORS`.
- **No two names alike.** Two names of one text that would share a colour get different ones while the palette
  has more. A name the configuration does not have (a remote member) gets a colour from its checksum.

## 2. File links

GitHub links commits (`abc1234`) and PRs (`#12`) by itself, never file paths. A link must be written:
`[`path`](<repo>/blob/<SHA>/path#L42)`. `prompts/writing.md` now says so, with the SHA to use: a review uses the
PR's `headRefOid`, so the link survives the merge. Until now reviewers and independent investigators never got
`writing.md`. Now they do (`build_prompt`), so the rule reaches every run that writes on GitHub. This is a prompt
rule: a model can still forget. Automatic linking in the agents' `gh` is possible but risky (paths inside code
blocks, unpushed SHAs), so I did not do it.

## 3. The review's layout

`prompts/review.md` now fixes the order of the body:

1. the byline;
2. `## Recommendation` with exactly one of these lines, which follows the `VERDICT`:
   - `✅ $\color{green}{\textbf{Ready to merge}}$`
   - `🔧 $\color{red}{\textbf{Needs changes}}$`
   - `🧩 $\color{red}{\textbf{Needs changes}}$ — incomplete`

   These are the icons the notifications use for `ready-to-merge`, `needs-fixes` and `incomplete`. For changes,
   the list of what to fix follows;
3. the assessment;
4. the problems by severity;
5. what was verified;
6. the byline again.

## 5. "Generated with Claude Code"

- **Claude Code's settings.** `run_claude` passes `--settings <task>/claude-settings.json` with
  `{"includeCoAuthoredBy": false, "attribution": {"commit": "", "pr": ""}}`. The old key and the new one are both
  set, so either CLI version adds no attribution.
- **The agents' `gh`.** It drops a "🤖 Generated with [Claude Code](…)" line (and the blank line after it) from
  everything it publishes.
- **The prompt.** `writing.md` says: no `Co-Authored-By`, no "Generated with …".
- **Not verified with the real CLI.** It was not run (`AGENTS.md` rule 4). The settings keys are from Claude Code's
  documentation.

## 6. The merged prompt report — what is left

| Item | Status |
|---|---|
| 1 reviewer never sees its request | **fixed**: `investigation_task_text` puts the request (independence, re-review, previous review) into the reviewer's `task.md` |
| 2 `CLAUDE.md` stale | fixed before this task (symlink, `49f88fa`) |
| 3 `investigate` tool | fixed before (`9575b28`) |
| 4 helpers told to fill skipped tests | **fixed**: `skip_note` skips helpers |
| 5 bus instructions offer `request_investigation` to investigators | **fixed**: the text is built from the run's tools (also no `reserve` for remote members); Go test |
| 6 "cheaper everyday model" | partly fixed before ("You are a model." — awkward); **reworded** |
| 7 `helper.md` silent about `request_investigation`, `checkpoint` | **fixed**: rule 6 |
| 8 independent phases get team/bus text | **fixed** in `review.md` (neutral PR origin, "a fix pass", reservations only with the bus) and in `{{AGENTS_MD}}` (the notebook only with the bus) |
| 9 `inv-resume.md` cites "CLARIFICATION RULES point 6" | **fixed**: self-contained |
| 10 local and remote delegation give different rules | **left**: a remote job carries no helper role; it needs a change of the job format (`agentbus/helper_remote.go`, `create_task`), a compatibility topic (rule 9) |
| 11 Polish text | fixed before (`9575b28`) |
| 12 independent phase repeats its prompt (twice in a fresh session) | **fixed**: a resume sends only the answers / note (or "continue"); a fresh session has the first prompt once |

## Suggestions for `AGENTS.md` (not changed — rule 2)

- Every new bats file must also get its lines in `tests/agent-go-map.tsv`, or the `harness` step fails. Say so in
  "Tests".

## How this was checked

- `make gate`: all 7 steps passed after each GREEN commit.
- New tests:
  - `tests/unit/review_pair_repeat.bats`: 8;
  - `tests/unit/github_names.bats`: 8;
  - `tests/unit/prompt_delivery.bats`: 8;
  - `agentbus/instructions_test.go`: 1.
- No existing test was changed.
- Read-only look at the installation: `~/agents/agents.conf`, `~/agents/reviews/*`, the coordinators'
  `activity.jsonl`.
