# GitHub presentation, reviewer rotation, and prompt audit — inv-astra

Status: implementation complete; final `make gate` passed all seven steps.
Audited base: `9575b28cb48d9e96b91644b8c882e426c254d435`.
Branch: `agents/inv-astra/20261103-123532`. No pull request was opened.

## Decisions and scope

The human approved making rotation the default while preserving explicitly configured
`random`/`list` behavior; retaining the limited-pool and 20-minute busy-timeout exceptions;
and investigating the intermittent baseline failure without weakening its test. The human
also authorized replacing obsolete default-selection and plain-name attribution assertions.
Only those existing tests were adjusted. New behavior was covered in a committed RED cycle
before implementation (`a62709c`).

This investigation used only its own worktree. No other agents, bus tools, other worktrees,
hardware, real agent CLIs, installation changes, GitHub comments or PR creation were used.
All required tools were available; no packages or dependencies were installed. Since the
independent run has no notebook tool, this file records the findings.

## Fixes for points 1–5

1. **Stable bold identity colors.** `github_identity` in `bin/agent` is shared by PR
   attributions, investigator bylines and rendered prompt attribution. It derives a color
   from the first six SHA-256 hex digits of the configured name, independent of role,
   model, roster and round. MathJax renders the name bold and colored; underscores are
   escaped. The supplied Gemini/Claude/Codex examples and two Codex investigators have
   distinct colors in regression coverage. The GitHub wrapper still injects only a missing
   byline; it uses the same identity as the review prompt.
2. **File links.** A shared `prompts/github-writing.md` fragment now reaches every publishing
   role and independent phase. It requires verified full-commit file permalinks in prose,
   including inline review comments, base-commit links for deleted files, URL-encoded paths,
   directory links and optional line anchors. Code samples and nonexistent paths are not
   rewritten. The fragment explicitly does not grant permission to publish or commit.
3. **Recommendation first.** The review body contract now places `## Recommendation`
   immediately after the investigator byline, before assessment, findings and checks.
   Ready to merge is bold green with ✅; Needs changes is bold red with 🔧, matching
   notification icons. Incomplete stays explicit with 🧩. Machine-readable
   `VERDICT: ready-to-merge | needs-fixes | incomplete` is unchanged.
4. **Reviewer rotation.** The repeated default pair was caused by the deliberate `random`
   reuse path, not a random-seed bug. Both runtime and template defaults now select
   `rotate`. The existing persisted history and last-two-pairs exclusion remain. Selection
   minimizes previous member overlap, then pair recency, then previous model overlap,
   then member recency. Provider/model identity is inferred from current configuration;
   old history files need no migration. Regression coverage runs six separate selections
   for the same PR: A+B, C+D, A+C, then all remaining mixed pairs before reuse. A separate
   case verifies preference for models absent from the last review.
5. **Claude footer.** Both new and resumed Claude invocations supply
   `--settings '{"attribution":{"commit":"","pr":""}}'`. This disables vendor attribution
   for that invocation without editing user settings. Shared publishing instructions also
   prohibit vendor footers and vendor co-author trailers in comments/reviews, preserving
   team-agents attribution and the task's mandatory trailers.

GitHub documents MathJax in pull requests and Markdown, and permanent code/file links:
[math rendering](https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/writing-mathematical-expressions),
[permanent links](https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/creating-a-permanent-link-to-a-code-snippet).
Claude documents disabling PR attribution with an empty string and per-run settings:
[attribution reference](https://code.claude.com/docs/en/settings-reference#attributionpr),
[settings](https://code.claude.com/docs/en/settings).

## Re-audit of the earlier merged report (point 6)

Numbers refer to section 8 of
`docs/investigations/i-need-to-know-which-rules-or-prompts-applied-to-worker-models-n/merged.md`.
That file remains a historical snapshot; already-fixed items were not duplicated.

| Item | Finding and resulting action |
|---|---|
| 1. Review request dropped | Fixed: preserve queued `.question` alongside the original task, including independence, fix-pass number and previous findings. |
| 2. Stale `CLAUDE.md` | Already fixed at the base: symlink to `AGENTS.md`; no edit. |
| 3. Nonexistent `investigate` tool | Already fixed in the cited prompts; no edit. |
| 4. Helpers assigned unrelated skipped tests | Fixed: `skip_note` excludes helpers as well as investigations, including follow-up routes. |
| 5. Unavailable tools in MCP startup guidance | Fixed: investigator initialization omits `request_investigation`; remote-member guidance omits unavailable reservations, escalation and blocking waits. Local workers retain escalation guidance. |
| 6. Every solo model described as cheap | Already fixed: protocol says “You are a model”; no edit. |
| 7. Missing helper tool guidance | Fixed: describe investigation/checkpoint use within the assigned scope without granting commits. |
| 8. Bus/team assumptions in independent phases | Fixed: render independent read-only review instructions and isolated resource use; instruction gaps go in findings/reviews instead of an unavailable notebook. Review wording covers independent authors as well as teams, without assuming a `--verify` fix pass. |
| 9. Missing clarification section referenced | Fixed: independent resume spells out the numbered question, Recommended and Verdict format instead of citing absent rules. |
| 10. Remote delegation expands permissions | Fixed: optional `Helper` discriminator survives the job boundary; the node records helper kind and requester, renders helper scope, and delivers completion through the durable job rather than an unrelated local helper record. Missing remote context returns `UNCLEAR`. |
| 11. Polish review headings | Already fixed in the cited queue/startup paths; no edit. |
| 12. Fresh independent prompt duplicated | Fixed: fresh-session composition renders the base/topic once, then the handoff and resume additions. Standalone resume retains its phase context; answers and human notes are preserved. |

The remote request field is additive and omitted when false. A pre-change JSON fixture
verifies old requests still load and serialize without acquiring helper scope. Newly frozen
helper requests retain the discriminator across retries. Existing frozen requests are not
mutated, preserving idempotency; create a new request to adopt helper scope for those jobs.
The submitting host, lead and destination node must all be updated. Ordinary worker
jobs and remote team-member requests retain their existing meaning.

## Verification

- RED commit `a62709c`: `make gate` passed shell/Go lint, generated checks, doccheck and
  harness checks. Failures were the new regressions and explicitly authorized presentation
  expectations. Unchanged baseline tests passed.
- New coverage: 11 Bats tests for default six-pair rotation, model diversity, stable identity,
  wrapper/prompt consistency, shared publication rules, recommendation order, independent
  fresh prompts, scoped helpers, queued review instructions, Claude invocation settings and
  remote helper startup/completion; three Go tests for capability-aware initialization,
  frozen helper scope and old request compatibility.
- Focused new Bats and Go tests passed after implementation. The legacy random/list tests
  passed after their authorized helper explicitly selected the legacy mode before sourcing
  runtime defaults. Existing rotation, byline and skipped-test coverage remains in place.
- Final full `make gate`: **7/7 passed**, including 501 Bats tests and 20 harness tests.
  The preceding full implementation gate also passed. Logs are in `build/gate/`.
- No lint suppressions, new dependencies, generated-code edits, workflow edits or repository
  instruction-file changes were introduced.

Earlier baseline runs intermittently failed the unchanged leased-parent test at
`investigation_team_admission.bats:175` and fresh-answer test at
`investigation_prompt_clarification_resume.bats:65`. Both passed in isolation and in the
committed RED gate's unchanged-test portion and both final implementation gates. Their root causes remain unestablished; no
baseline test, timeout or parallelism setting was weakened to obtain a pass. The lease
fixture has a 20-second child deadline and the suite uses parallel jobs, but that alone
does not explain the observed assertion sequence and is not presented as a diagnosis.

## Compatibility and practical limits

- Existing installations that explicitly set `AGENT_REVIEW_DRAW=random` or `list` must
  switch to `rotate` to adopt rotation. Those modes intentionally retain a PR's available
  previous pair, as authorized. The limited-pool and busy-timeout exceptions remain and
  emit `review_pair_repeated`; four available investigators do not use those exceptions.
- Colors are stable name hashes, not a guarantee of collision-free colors or accessible
  contrast for every possible configured name. The complete name remains visible and
  must remain the identity; plain-text clients show the math markup.
- File links and recommendation order are publishing instructions, not an arbitrary
  Markdown rewriting layer. Tests verify the delivered prompt contracts; a model can
  still disobey them. Byline identity and Claude settings are runner-enforced.
- No GitHub comment was posted and no real Claude invocation was made. Exact browser
  rendering and installed-provider behavior are therefore not claimed as end-to-end
  verified. No existing GitHub comments are retroactively edited.
- The branch is prepared for Phase 2 integration. The designated merger must check
  conflicts with other independently developed branches; this investigation does not
  merge into `main` or open a PR.
