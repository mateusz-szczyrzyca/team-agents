# Investigation Findings — What Needs to Be Fixed on GitHub (`inv-gemini-argon`)

- **Task:** `20261103-123532-inv-gemini-argon`
- **Investigator:** `inv-gemini-argon` (`gemini`, model `gemini-4-argon-high`)
- **Branch:** `agents/inv-gemini-argon/20261103-123532`
- **Base commit:** `9575b28cb48d9e96b91644b8c882e426c254d435`

---

## Executive Summary

All six problem areas identified in the investigation topic have been diagnosed to their exact root causes in `bin/agent`, `agentbus/main.go`, `config/agents.conf`, and `prompts/*.md`, and fixed test-first (RED → GREEN) on branch `agents/inv-gemini-argon/20261103-123532`:

1. **Consistent bold distinct colors for model and investigator names across rounds:** Added deterministic `agent_color` and `agent_color_name` helpers in `bin/agent` and wired them into `agent_desc` (with fallback to `INVESTIGATORS_TYPE`/`INVESTIGATORS_MODEL`), `review_byline`, and `R[ATTRIBUTION]`.
2. **Clickable links for repository filenames on GitHub:** GitHub supports linking repository paths to `<repo_url>/blob/<ref>/<path>` (which also redirects directory paths like `tools/errgen` to `/tree/<ref>/<path>`). Added automatic path linking in `agent_gh` (`gh_format_markdown`), format validation in `pr_format_issues`, and instructions in `prompts/writing.md`, `prompts/review.md`, and `prompts/inv-merge.md`.
3. **Review `Recommendation` section at the top with colored/icon verdicts:** Updated `prompts/review.md` and `agent_gh` (`gh_format_markdown`) so `### Recommendation` is placed immediately after the top review byline (moving it to the top if a model places it at the bottom) and verdicts are formatted with the notification icons and bold colors (`✅ $\color{green}{\textbf{Ready to merge}}$` and `🔧 $\color{red}{\textbf{Needs changes}}$`).
4. **Prohibiting the same investigator pair on a second review:** Changed default `AGENT_REVIEW_DRAW` to `rotate` in `config/agents.conf`, updated `start_review_pair` to use `review_pair_rotate` for all unseeded review draws (even when an existing `~/agents/agents.conf` still has `AGENT_REVIEW_DRAW=random`), and updated `review_pair_pick` to exclude the PR's own previous pair (`A.agent` and `B.agent`) and prefer non-overlapping investigators (`A,B` → `C,D` → mixed pairs `A+C`, `B+D`, `A+D`, `B+C`).
5. **Preventing Claude Code attribution clauses:** Passed `--settings '{"attribution":{"commit":"","pr":""},"includeCoAuthoredBy":false}'` in `run_claude`, stripped `Generated with Claude Code` / `Co-Authored-By: Claude` lines in `agent_gh`, flagged them in `pr_format_issues`, and prohibited them in the prompts.
6. **Fixing prompt and `agentbus` inconsistencies from `docs/investigations/i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md`:** Fixed all remaining unaddressed inconsistencies from §8 of that report (items 1, 4, 5, 7, 8, 9, 11, 12; items 2, 3, 6 were already fixed in `49f88fa` and `9575b28`, and item 10's node-job contract is documented below).

> **Post-merge host note:** Because `AGENTS.md` Rule 3 forbids agents from running `./install.sh` against the live host installation, `/home/dev/.local/bin/agent` and `/home/dev/agents/prompts/` on this machine were last installed prior to PR #44 (`2026-11-03 05:33`). Once this branch is merged to `main`, run `./install.sh` on the host so the updated script, `agentbus` binary, and prompts take effect in `~/agents/`.

---

## Detailed Root Causes & Fixes by Topic Item

### 1. Consistent Bold Distinct Colors for Model and Investigator Names Across Rounds

- **Root cause:**
  - `agent_desc` (`bin/agent`) formatted agent names in plain backticks (`` `gemini-1` ``) and only checked `AGENT_TYPE`/`AGENT_MODEL` (ignoring `INVESTIGATORS_TYPE`/`INVESTIGATORS_MODEL`, which also caused Phase 2 merge PR #48 to render `- agent: \`inv-opus\` (?)`).
  - `review_byline` (`bin/agent`) and `R[ATTRIBUTION]` (`build_prompt`) also formatted investigator names in plain backticks (`Review by the team-agents investigator \`inv-astra\` (codex, model gpt-6-astra), request #66.`).
  - Without a canonical color mapping in `bin/agent`, models writing PR descriptions either left names uncolored in backticks or invented arbitrary MathJax colors across rounds.
- **Fix:**
  - Added `agent_color` and `agent_color_name` in `bin/agent`, mapping each configured agent (`gemini-1`, `claude-haiku`, `codex-1`, `claude-sonnet`, `openrouter-*`, `deepinfra-*`) and investigator (`inv-astra`, `inv-opus`, `inv-kimi`, `inv-gemini-argon`) to a fixed, high-contrast hex color (`$\color{#2563eb}{\textbf{gemini-1}}$`, `$\color{#d97706}{\textbf{claude-haiku}}$`, `$\color{#16a34a}{\textbf{codex-1}}$`, `$\color{#0891b2}{\textbf{inv-astra}}$`, `$\color{#9333ea}{\textbf{inv-opus}}$`, `$\color{#ea580c}{\textbf{inv-kimi}}$`, etc.) with a deterministic `cksum` palette fallback for any other name.
  - Updated `agent_desc`, `review_byline` (when called by `agents_gh_env`), `gh_format_markdown`, and `R[ATTRIBUTION]` in `build_prompt` to emit `agent_color_name`.

### 2. Linking Filenames in GitHub Comments, PR Descriptions, and Reviews

- **Root cause:**
  - GitHub does not auto-link bare or backticked repository file paths (such as `` `agent/errors.go` `` or `tools/errgen`) in PR descriptions, comments, or reviews, but it **does** support standard Markdown links to `https://github.com/<owner>/<repo>/blob/<ref>/<path>` (and automatically redirects `/blob/<ref>/<dir>` to `/tree/<ref>/<dir>` for directories).
- **Fix:**
  - Added automatic repository path linking in `gh_format_markdown` (`bin/agent`), invoked by `agent_gh` on outgoing `gh pr create`, `gh pr edit`, `gh pr comment`, `gh issue comment`, and `gh api .../pulls/<n>/reviews` payloads. Outside fenced code blocks and existing Markdown links/URLs, both backticked and bare repository paths (`agent/errors.go`, `agent/errors.json`, `agent/errors_gen.go`, `tools/errgen`, etc.) are converted to `[`path`](<repo_url>/blob/<ref>/<path>)`.
  - Updated `pr_format_issues` to flag unlinked backticked repository paths in PR descriptions.
  - Updated `prompts/writing.md`, `prompts/review.md`, and `prompts/inv-merge.md` to instruct agents to link repository files and directories on GitHub.

### 3. Review `Recommendation` Section at the Top with Colored/Icon Verdicts

- **Root cause:**
  - `prompts/review.md:59-60` listed `recommendation: "ready to merge" or "needs changes"` last in the description of `body`, causing investigators on PR #47 to place `### Recommendation` at the very bottom of their reviews as plain bold text (`**ready to merge**` / `**Needs changes.**`).
- **Fix:**
  - Updated `prompts/review.md` to require `### Recommendation` immediately after the top `Review by the team-agents investigator ...` byline, using the exact icons from `notify` (`bin/agent`):
    - `✅ $\color{green}{\textbf{Ready to merge}}$`
    - `🔧 $\color{red}{\textbf{Needs changes}}$`
  - Updated `gh_format_markdown` in `agent_gh` (`bin/agent`) so that when an investigator posts a review via `gh api .../pulls/<n>/reviews`, any `## Recommendation` / `### Recommendation` section placed lower in the review body is automatically moved to the top (right after the top byline) and its verdict is formatted as `✅ $\color{green}{\textbf{Ready to merge}}$` or `🔧 $\color{red}{\textbf{Needs changes}}$`.

### 4. Prohibiting the Same Investigator Pair on a Second Review

- **Root cause:**
  - Three compounding issues caused PR #47's second review (`inv121` and `inv122`) to pick the exact same pair (`inv-gemini-argon` and `inv-opus`) as its first review (`inv119` and `inv120`):
    1. `config/agents.conf` and `/home/dev/agents/agents.conf` still had `AGENT_REVIEW_DRAW=random` as the default, and `start_review_pair` (`bin/agent`) explicitly reused `$dir/A.agent` and `$dir/B.agent` (`pa` and `pb`) whenever `AGENT_REVIEW_DRAW != rotate`.
    2. `review_pair_pick` only inspected `$AGENTS_HOME/reviews/history` (`last` and `prev`) and ignored the PR's own `$dir/A.agent` and `$dir/B.agent`, so if `reviews/history` was missing or other PRs were reviewed in between, the PR's own previous pair could be chosen again.
    3. `/home/dev/.local/bin/agent` on the host had not been reinstalled after PR #44.
- **Fix:**
  - Updated `config/agents.conf` and `README.md` to set `AGENT_REVIEW_DRAW=rotate` by default.
  - Updated `start_review_pair` (`bin/agent`) to use `review_pair_rotate` for all unseeded draws (`[[ $AGENT_REVIEW_DRAW != list && -z ${AGENT_REVIEW_SEED:-} ]]`), ensuring that even existing installations with `AGENT_REVIEW_DRAW=random` in `~/agents/agents.conf` rotate pairs instead of reusing `A.agent` and `B.agent`.
  - Passed the PR's own previous review pair (`REVIEW_PR_LAST` from `$dir/A.agent` and `$dir/B.agent`) into `review_pair_pick` so a PR's second review strictly excludes its first pair and scores overlap against `pr_last` (`ov=0` for the two unused investigators). With 4 investigators `A, B, C, D`, review 1 picks `A B`, review 2 picks `C D`, and review 3+ mixes pairs (`A C`, `B D`, `A D`, `B C`).

### 5. Preventing Claude Code Attribution Clauses

- **Root cause:**
  - Claude Code CLI injects system instructions to append `🤖 Generated with [Claude Code](https://claude.com/claude-code)` to PR descriptions and `Co-Authored-By: Claude ...` to commits unless disabled via `--settings`. This appeared on PR #46 and PR #48.
- **Fix:**
  - Added `--settings '{"attribution":{"commit":"","pr":""},"includeCoAuthoredBy":false}'` to `run_claude` in `bin/agent`.
  - Stripped any `Generated with Claude Code` and `Co-Authored-By: Claude` lines in `gh_format_markdown` (`agent_gh`) and flagged them in `pr_format_issues`.
  - Added explicit prohibition in `prompts/writing.md`, `prompts/review.md`, and `prompts/inv-merge.md`.

### 6. Audit & Fixes for `docs/investigations/i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md` (§8)

| # | Inconsistency in `merged.md` §8 | Status before this branch | Action taken on this branch |
|---|---|---|---|
| 1 | `start_investigation` drops `.question` for `kind == review`, hiding independence and re-review context | Unfixed | Fixed in `start_investigation` (`bin/agent`): `.question` is now written to `task.md` for `kind == review`. |
| 2 | `CLAUDE.md` is stale compared to `AGENTS.md` | Fixed in `49f88fa` | Verified (`CLAUDE.md` is a symlink to `AGENTS.md`). |
| 3 | Prompts name nonexistent tool `investigate` (`team.md`, `tdd-rounds.md`) | Fixed in `9575b28` | Verified (`request_investigation` is used in both files). |
| 4 | `skip_note` appends repository-wide skipped tests to `helper` tasks | Partially fixed in `9575b28` | Fixed in `skip_note` (`bin/agent`): omitted for `KIND == helper` as well as `investigation`. |
| 5 | `agentbus` `initialize` `instructions` string offers `request_investigation` to `RoleInvestigator` | Unfixed | Fixed in `agentbus/main.go`: omitted `request_investigation` from `instructions` when `s.role == RoleInvestigator`. |
| 6 | `protocol.md:110` tells every solo model `"You are a cheaper everyday model"` | Fixed in `9575b28` | Verified (`"You are a model"`). |
| 7 | `prompts/helper.md` does not mention `request_investigation` and `checkpoint` | Unfixed | Fixed in `prompts/helper.md`: documented both tools and when (not) to use them. |
| 8 | Independent phases get text assuming a team/bus (`review.md` `reserve`/`release`, `--verify`, `agents_md_text` `notebook_append`) | Unfixed | Fixed in `agents_md_text` (`bin/agent`) and `prompts/review.md`. |
| 9 | `prompts/inv-resume.md:8` cites `"CLARIFICATION RULES point 6"`, which independent phases never receive | Unfixed | Fixed in `prompts/inv-resume.md`: replaced with self-contained wording. |
| 10 | Remote `delegate` (`requires=[...]`) runs as a node job (`protocol.md`) rather than local `helper.md` | By design | Documented: remote jobs execute in isolated node checkouts without access to the requester's local worktree or local `agentbus`, and return results via pushed branch/PR (`remoteHelperResult`). |
| 11 | Polish heading `## Kontekst` in `bin/agent` (`start_helper`) | `Recenzja PR` fixed in `9575b28`; `## Kontekst` remained | Fixed in `start_helper` (`bin/agent`): changed `## Kontekst` to `## Context`. |
| 12 | Independent investigation `fresh` session renders base prompt and `# TOPIC` twice | Unfixed | Fixed in `build_prompt` (`bin/agent`): handled `mode == fresh` inside the `is_independent_investigation` branch so the base prompt and `# TOPIC` render only once. |

---

## Verification

- **Go unit tests (`agentbus`):** `go test -run TestInitializeInstructionsOmitRequestInvestigationForInvestigator ./...` — passed.
- **Bats unit tests:**
  - `bats tests/unit/investigation_prompt_clarification_resume.bats tests/unit/investigation_independent_resume_prompt.bats` — `6/6` passed.
  - `bats tests/unit/review_pair_rotate.bats tests/unit/review_pair_random.bats` — `14/14` passed.
  - `bats tests/unit/github_format.bats tests/unit/review_byline.bats tests/unit/skips.bats tests/unit/agent_gh.bats` — `22/22` passed.
- **Full repository gate:** `make gate` (`7/7` steps: `check-generated`, `lint-shell`, `lint-go`, `doccheck`, `test-agentbus`, `test-live`, `test-unit` across all `499` Bats tests) — passed with `0` findings.
