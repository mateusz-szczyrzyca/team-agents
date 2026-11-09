# Go rewrite parity ledger

`tests/agent-go-map.tsv` inventories the existing Bats contracts for the Go
rewrite. This R01 inventory does not activate candidate execution or claim Go
coverage. The unchanged Bash implementation remains the behavioral oracle.

A split round has a letter after its number (R05A, R29B); revision 2 of `rewrite-agent-in-go.md` adds
R29B, R30B, R32B, R35B and R53B for the subsystems added after the examined base.

## Baseline

- Examined commit: `940f6fdc067069f92a8023c84ed1b5eb011b95ae`.
- Bats files: `120`.
- Exact `(bats_file, test_name)` pairs: `480`.
- Black-box cases: `359`.
- Source-coupled cases: `121`, spread across `49` files.
- Every row initially has state `planned` and Go mapping `-`.

The ledger preserves titles exactly, including punctuation and whitespace.
Its header names the six fields; paths are relative to the repository root.
Later rounds replace `-` with real Go tests when implementing the corresponding
contract, following the `planned` → `active` → `done` lifecycle.

## Classification audit

`source` means that the case depends on interpreting the Bash implementation,
extracting its functions, or overriding its internal functions or variables.
The classification follows helpers called by each case, rather than searching
only its test body. Important indirect entry points include:

| Bats family | Helper reaching the Bash implementation |
| --- | --- |
| `investigation_team_isolation`, `investigation_team_opencode_schema` | `setup_isolation` |
| `investigation_team_recovery_regressions` | `free_investigators` |
| `review_pair_random` | `pair_of` |
| `remote_control_validation` | `control_fixture` |
| `remote_control_kick_exit` | `kick_exit_fixture` |
| `remote_control_review` | `review_kick_fixture`, `review_answer_fixture` |
| `job_attribution`, `job_attribution_paths` | `attribution_functions`, `attribution_path_functions` |

The second `remote_control_heartbeat_limits` case sources the implementation
through a positional argument. Function extraction in `node_agents` and
`node_report_agent` also counts as source coupling. Sourcing only generated
`meta.env` does not make a case source-coupled: that is a persisted-format
contract accessible to either implementation.

The aggregate classification agrees with the Phase 1 plan, but some earlier
per-file figures differ from the exact declarations. For example, `rotation`
has `2` cases, both source-coupled; `run_notes` has `2` source-coupled cases;
and `investigation_team_recovery_regressions` has `5` source-coupled cases.
Use exact ledger rows rather than earlier family-level counts.

## Round ownership

An implementation round identifies the complete behavior required by the case.
It can be later than the round introducing the named helper. For example,
provider-backed prompt cases need runner completion, node `--once` scenarios
need daemon orchestration, and investigation cases waiting for `done` need
terminal pipeline handling. Mixed files therefore have per-case assignments.

These assignments do not authorize early activation of parent commands.
Before activation, review every assertion, loop, failure injection, and reachable
dependency against the implementation available in that round. Source-coupled
contracts receive native Go equivalents; their original Bats cases continue to
run only against the preserved Bash oracle.

## R12 reconciliation and current ledger status

Following Investigation #251 and human approval (PR #71 follow-up), the active
ledger tracks 177 Bats test files with 730 total test cases:
- 463 blackbox cases
- 267 source-coupled cases
- 12 done cases
- 718 planned cases

The original 48 R12 rows in `tests/agent-go-map.tsv` were partitioned into:
- 29 retained cases across seven bounded write slices (R12A–R12G), with each retained
  row assigned to its earliest completion slice (R12A: 85, 474; R12C: 583, 584, 596, 597,
  603–608, 615; R12D: 223; R12E: 577, 578, 580–582, 592; R12F: 472, 473, 579, 609–614).
  All retained cases remain `planned` until full native equivalents are verified (e.g. 85 and 474 in R12A).
- 19 reassigned cases moved to their respective subsystem rounds:
  - R16 (rendered prompt / writing guidance): 587, 590, 598
  - R18 (Claude/Codex adapter settings): 250, 591, 617–619, 622, 633
  - R19 (Gemini/OpenCode adapter coverage): 616, 620–621, 634
  - R22 (runner lifecycle): 623
  - R35 (investigation task text): 585–586
  - R38 (pipeline phase 2/3 rendering): 588
  - R40 (investigation resume/fresh prompts): 589
- 4 deferred R11 cases (`member_in_team.bats:89`, `member_in_team.bats:102`, `member_pr.bats:29`,
  `pr_ownership.bats:31` at TSV lines 251, 252, 269, 406) retain their R11 provenance and remain
  `planned` until write activation in R12E.

Scope exclusions:
- `round_pr_for` fallback logic remains in R32B (`rewrite-agent-in-go.md:475`).
- Round orchestration remains in R31; formatting bounce/review lifecycle remains in R34;
  remote-job attribution remains in R41/R50.
- Existing R10 setup/byline/registry primitives are reused.

Human approval authorizes metadata-only ledger edits and the replacement of only 18 transitional
write-refusal assertions across five native test files (`gh_read_dispatch_test.go`,
`gh_read_dispatch_api_test.go`, `gh_read_dispatch_api_short_options_test.go`,
`gh_read_dispatch_graphql_mutation_test.go`, `gh_read_dispatch_graphql_query_file_test.go`),
strictly in the slices activating their corresponding write routes. Existing Bash tests and stubs
remain unchanged.
