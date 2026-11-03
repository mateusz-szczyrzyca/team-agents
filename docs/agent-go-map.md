# Go rewrite parity ledger

`tests/agent-go-map.tsv` inventories the existing Bats contracts for the Go
rewrite. This R01 inventory does not activate candidate execution or claim Go
coverage. The unchanged Bash implementation remains the behavioral oracle.

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
