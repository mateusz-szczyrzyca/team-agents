# GitHub presentation, reviewer rotation, and prompt audit — inv-astra

Status: awaiting the human's decision required by `AGENTS.md`, Never rule 1.
Audited base: `9575b28cb48d9e96b91644b8c882e426c254d435`.
No implementation or existing test was changed. This report does not satisfy the requested
merge-ready implementation of points 1–6. The question is in this task's `questions-1.md`;
its decision options are also recorded below so they travel with the branch.

## Blocking contract conflict (point 4)

`tests/unit/review_pair_random.bats` explicitly tests **"a PR's next review pass keeps its pair"**.
Its fixture does not set the draw mode: it exercises the default. Changing the default to
rotation or prohibiting repeated pairs makes that existing assertion fail. The same file
explicitly tests the first-two-free behavior of `AGENT_REVIEW_DRAW=list`.
`tests/unit/review_pair_rotate.bats` also expects the default supervisor not to dispatch
a waiting rotation, and explicitly permits repeated pairs with two investigators and
after a busy-investigator timeout.

The binding repository instruction says: "If the task requires behaviour that an existing
test forbids, stop and ask the human in your questions file; wait for the answer."
The task asks for new regression tests but does not explicitly authorize changing existing ones.
I stopped implementation rather than weaken tests, disguise a default as an explicit legacy
setting, or claim the critical bug was fixed through an unused optional path.

Decision requested:

1. Authorize updating the obsolete pair-retention/default-supervision expectations and
   changing the default to rotation; preserve explicitly selected legacy `random`/`list`
   modes, document their behavior, and add default-mode regressions (recommended); or
   require rotation in every mode, authorizing the corresponding compatibility and test changes.
2. Keep the existing limited-pool/timeout exceptions, but require a different pair whenever
   available (recommended); or prohibit repeats absolutely and wait for another eligible pair,
   including indefinitely when only two investigators exist.

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

After resolving the question, test default-mode six-pair coverage, the same PR and resumed
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
- `bats tests/unit/review_pair_random.bats tests/unit/review_pair_rotate.bats`: **12/12 passed**,
  including the obsolete default pair-retention assertion.
- `make gate`: **6/7 steps passed**. Generated-code, shell lint, Go lint, declaration comments,
  Go tests/vet, and harness passed. The Bats step failed on the unchanged test
  `a leased parent without manifest keeps the lease while its tagged child is still running`
  in `tests/unit/investigation_team_admission.bats:175`: `[ "$lease_now" = 1 ]` failed.
  Full local log: `build/gate/unit-bats.log`. No retry, test edit, or workaround was applied;
  the cause has not been established. This is an additional baseline validation blocker for
  implementation, reported to the human in question 3. Only this findings document is pushed.
- This is a documentation-only checkpoint. No new behavior or tests were added, no lint
  suppressions were added, and no persisted format or public interface changed.
- Point 7 remains pending. The merger must not present this report as a completed fix set.
