# GitHub presentation, reviewer rotation, and prompt audit — inv-astra

Status: awaiting the second clarification required by `AGENTS.md`, Never rule 1.
Audited base: `9575b28cb48d9e96b91644b8c882e426c254d435`.
No implementation or test changes are included in this checkpoint. This report does not
satisfy the requested merge-ready implementation of points 1–6. The new question is in
this task's `questions-2.md`; its decision options are recorded below so they travel
with the branch.

## Accepted first-round decisions

The human answered **a** to all three questions in `questions-1.md`:

1. Make rotation the default and authorize updating obsolete default pair-retention and
   supervisor expectations; preserve explicitly configured legacy `random`/`list` modes.
2. Retain the existing limited-pool and busy-timeout exceptions. Rotate whenever an
   alternative pair is available; document and log exceptions.
3. Investigate the unrelated baseline lease-test failure without weakening its test.

These decisions resolve the original selection-contract blocker. Existing installations
that explicitly configure `random` or `list` will need to select `rotate` to obtain the
new behavior; a default change alone cannot override their explicit configuration.

## New blocking test conflict (point 1)

Two additional existing tests require the old plain-name presentation:

- `tests/unit/skips.bats:60–61` anchors coordinator/member attribution to backticked names.
- `tests/unit/review_byline.bats:44` requires the exact plain investigator byline.

The requested bold colored identities conflict with those assertions. The first approval
covers reviewer-selection expectations, not these presentation tests. `AGENTS.md`, Never
rule 1 says: "If the task requires behaviour that an existing test forbids, stop and ask
the human in your questions file; wait for the answer."

The second clarification file asks for one of these decisions:

- **a (recommended):** authorize updating these obsolete attribution assertions, retaining
  identity/model and nonduplicated-byline checks, and add stable-color regressions.
- **b:** preserve the assertions and canonical plain attribution; restrict colored identities
  to additional presentation, leaving part of point 1 unmet.

No answer has been received. Do not preserve redundant plain attribution lines or introduce
unused legacy rendering paths merely to evade the existing tests. Implementation is paused
at this newly discovered conflict, as required by the human's clarification instructions.

## Findings and proposed fixes

### 1. Stable bold identity colors

`bin/agent:1071` (`agent_desc`), `:1254` (`review_byline`), and `:2661`
(`R[ATTRIBUTION]`) each produce attribution separately. They emit backticks, not a stable
color mapping. `prompts/writing.md` also asks for agent names in code format, which conflicts
with the requested colored bold identity. The runner's GitHub wrapper prepends a byline
from `AGENT_REVIEW_BYLINE`; correcting only the review prompt would leave that path inconsistent.

Use one renderer for all three sites, with a documented stable identity-to-color policy.
Do not derive colors from round number, role, current participant position, or Bash map
iteration order. Known configured names must remain visually distinct, including investigators
using the same provider. A small hash palette alone cannot guarantee distinct colors;
explicit stable mappings or persisted collision resolution need consideration. Escape names
before inserting them into TeX. Reuse the rendered attribution verbatim in prompts.

GitHub documents MathJax rendering for PRs and Markdown. The existing writing prompt already
uses `\color` and `\textbf` for RED/GREEN, so the same approach is a reasonable implementation
candidate; browser rendering of the exact generated expressions still needs verification.
[GitHub math documentation](https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/writing-mathematical-expressions).

Regression coverage: repeated rounds and fresh sessions yield identical identity markup;
the named Gemini/Claude/Codex examples differ; two investigators on one provider differ;
byline injection and prompt signatures agree; punctuation is escaped safely.

### 2. File references can be links

GitHub supports permanent file/line links in comments. Use
``[`agent/errors.go`](https://github.com/<owner>/<repo>/blob/<full-head-SHA>/agent/errors.go)``.
Use the reviewed commit rather than moving `main`; use the base commit for deleted files,
encode path characters, and use `tree` for directories. Do not link nonexistent paths.
[GitHub permalink documentation](https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/creating-a-permanent-link-to-a-code-snippet).

`prompts/writing.md` specifies links for commits and PRs but only code formatting for file paths.
`review.md` does not receive `writing.md` through the ordinary review route. Add concise file-link
instructions to every publishing route, ideally from a shared GitHub-writing fragment.
Avoid blindly regex-rewriting arbitrary prose: it can corrupt existing links, code samples,
URLs, and references to historical/deleted files. If enforcing links in the publishing wrapper,
test those boundaries as well as both summary and inline-comment bodies.

### 3. Recommendation first, matching notification icons

`prompts/review.md` currently requests assessment, problems, checks, then recommendation.
It requires a byline at both ends. Change the body contract to byline, **Recommendation**,
then findings/checks, with exactly one recommendation. The current notification mapping in
`bin/agent:3190–3191` is **✅ ready to merge** and **🔧 needs fixes**; use 🔧 for
the requested red **Needs changes**, not a newly invented red-cross icon.

Keep `VERDICT: ready-to-merge | needs-fixes | incomplete` unchanged: it is machine-readable.
An incomplete/failed review must not acquire a green recommendation by default. The
`agent_gh` wrapper currently only prepends the byline; it does not reorder the review.
Tests should check the rendered publishing contract and actual submitted JSON where enforcement
is added, while retaining the verdict parser's existing safety behavior.

### 4. Why the same pair returns

Both `bin/agent:374` and `config/agents.conf:136` default to `random`.
`start_review_pair` (`bin/agent:2072`) explicitly reloads `A.agent` and `B.agent` and retains
them if free in non-rotation modes. This is intentional old behavior, not a missing random seed.
The README documents the same contract. Merely improving `review_pair_rotate` cannot fix it.

Rotation already implements A+B, C+D, A+C, B+D, A+D, B+C when all four investigators are free.
Existing rotation tests cover that sequence and a same-PR re-review, but do not exercise
it as the default. Global name-pair history is shared across repositories; it tracks names,
not provider/model identities. Renaming an investigator or changing its configured model
does not provide model-aware fairness.

Under the accepted first-round decision, test default-mode six-pair coverage, the same PR and resumed
coordinator tasks, explicit compatibility modes, busy/quota-limited candidates, history
reload, and dispatch after capacity becomes available. If model diversity is added, use
configured provider/model identity as a tie-breaker while preserving distinct investigator names.
The running installation was not inspected or changed; a deployed explicit `random` setting
would survive a new default unless migration or an explicit configuration change addresses it.

### 5. Claude-generated footer

`run_claude` supplies model, session, and MCP settings but no attribution override.
Claude's documented `attribution.pr` defaults to its generated-with footer; an empty string
disables it. Pass the setting per invocation, cover both new/resumed sessions, and retain
team-agents' own attribution and required commit trailers. Avoid writing user-wide settings.
Consider the independent merger route as well as ordinary workers.
[Claude attribution reference](https://code.claude.com/docs/en/settings-reference#attributionpr).

Also explicitly forbid vendor-generated footers in shared publishing instructions, including
comments/reviews (the documented setting specifically controls PR descriptions). Stub argument
capture can verify runner settings without running a paid CLI. Installed-version behavior
and rendered output were not exercised in this investigation.

## Re-audit of the earlier merged report (point 6)

The numbered entries below correspond to section 8 of
`docs/investigations/i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md`.
That document is a historical snapshot, not evidence that every item remains open.

| Item | Current finding | Needed action |
|---|---|---|
| 1. Reviewer never sees request | Still open: `start_investigation` uses PR metadata and the parent task, dropping `.question` in the review branch. | Preserve the request, independence instruction, fix-pass number, and supplied previous review; test exact delivery. |
| 2. Stale `CLAUDE.md` | Fixed: it is a symlink to `AGENTS.md`. | No edit needed. |
| 3. Nonexistent `investigate` tool | Fixed in the cited `team.md` and `tdd-rounds.md` locations. | Keep `request_investigation`; avoid duplicate corrective instructions. |
| 4. Helpers told to fill unrelated skips | Still open: `skip_note` excludes investigations only. | Suppress or scope the directive for helpers; test new/resume/fresh. |
| 5. MCP instructions advertise unavailable investigator tool | Still open: initialize unconditionally advertises `request_investigation`; tool listing excludes investigators. | Generate instructions from role/capabilities; test initialize and tools/list consistency, including relay restrictions. |
| 6. Every solo model called cheap | Fixed: protocol now says "You are a model." | No edit needed. |
| 7. Helper tool guidance incomplete | Still open: helper prompt omits available investigation/checkpoint guidance. | Explain permitted use without broadening the assigned task or violating no-commit rules. |
| 8. Independent phases receive bus/team assumptions | Still open: shared review requires reserve/release; `agents_md_text` unconditionally recommends notebook_append. | Render role-appropriate instructions and resource restrictions, rather than depend on later contradictory overrides. |
| 9. Independent resume cites absent clarification section | Still open in `inv-resume.md`. | Refer to the phase's own question format. |
| 10. Remote helper receives solo permissions | Still open by source trace: remote request uses `Selector: worker`; node startup uses `create_task`, which sets `KIND=worker`. | Preserve delegation kind and requester context across the job boundary, with backward-compatible persisted job data and fixtures if fields change. |
| 11. Polish review headings | Fixed in the cited review queue and startup paths. | No edit needed. |
| 12. Independent fresh prompt duplicates base | Still open: fresh recursively calls new and resume, each rendering the independent base/topic. | Separate base rendering from follow-ups; test base/topic count and preservation of answers, notes, and handoff. |

The existing `investigation_prompt_clarification_resume.bats` fresh-session test checks that
the **answers header** appears once. That is compatible with the **base prompt and topic**
appearing twice; it does not prove item 12 fixed. Add a separate regression instead of changing
that test. Keep phase 1/2 code-writing rules separate from phase 3/5 read-only review rules.

## Validation and completion boundary

- Only this worktree was inspected. No coordination, bus tools, other worktrees, hardware,
  real agent CLIs, installation changes, PR creation, or GitHub comments were used.
- Before clarification, `bats tests/unit/review_pair_random.bats tests/unit/review_pair_rotate.bats`
  passed **12/12**, including the obsolete default pair-retention assertion.
- The first baseline `make gate` passed **6/7** steps and failed the unchanged test
  `a leased parent without manifest keeps the lease while its tagged child is still running`
  at `tests/unit/investigation_team_admission.bats:175`: `[ "$lease_now" = 1 ]` failed.
- After the human authorized investigation, the same lease test **passed in isolation**.
  It also did not fail in the subsequent full run with draft regressions. The fixture has
  a 20-second child deadline and the suite uses 32 parallel jobs here, but a simple timeout
  does not explain why the original run reached the lease assertion after waiting for the
  child to finish successfully. No root cause has been established. No production change,
  timeout workaround, or test weakening was applied.
- Draft regression coverage was prepared for default rotation, model diversity, identities,
  shared GitHub/file-link instructions, recommendation order, independent fresh prompts,
  helper scope, queued review requests, Claude settings, and MCP capability guidance.
  Against unchanged production code, the run failed on the new Bats and Go assertions;
  the draft also initially had shell-lint findings, corrected before saving. Some tests
  still need refinement, and the last model-diversity test was not run. These drafts are
  not a completed or committed RED cycle and are not part of the pushed change.
- The draft tracked-file patch, two new test files, and that gate's logs are saved locally
  at `/tmp/inv-astra-resume-20261103-z5ec1w4r` for resumption after approval. All draft test
  and build-list changes were removed from this worktree before the final baseline gate.
- Final baseline `make gate` after restoring drafts: **6/7 passed**. The lease test passed,
  but the unchanged `independent investigator fresh session prompt does not duplicate
  answered questions` test failed at `tests/unit/investigation_prompt_clarification_resume.bats:65`:
  `grep -c "# THE HUMAN'S ANSWERS"` found no header. It **passed immediately in isolation**.
  This second intermittent failure is also unresolved; a clean full gate has not been
  demonstrated. Logs are preserved in the local draft directory under `gate-baseline`.
  No failing implementation is being pushed; this commit contains findings only.
- This is a documentation-only checkpoint. No lint suppressions, persisted format changes,
  public interface changes, or implementation fixes are included.
- Point 7 remains pending. The merger must not present this report as a completed fix set.
