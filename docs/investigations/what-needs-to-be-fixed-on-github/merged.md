# What needs to be fixed on GitHub — merged result

Merged by `inv-opus` from the phase 1 results recorded in the manifest. Base:
[`9575b28`](https://github.com/mateusz-szczyrzyca/team-agents/commit/9575b28cb48d9e96b91644b8c882e426c254d435).

| Investigator | Recorded result | Used |
|---|---|---|
| `inv-opus` | [`5685890`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5685890fd9063254e403688e3087f984667f33d9), report [`inv-opus.md`](inv-opus.md) | the base: all 9 commits, unchanged |
| `inv-astra` | [`25afa8f`](https://github.com/mateusz-szczyrzyca/team-agents/commit/25afa8f4715f1c4adbf91895e9a196525f9c4ef8), report [`inv-astra.md`](inv-astra.md) | the model tie-break of review pairs, the file-link details; the rest overlaps with inv-opus or is left (below) |
| `inv-gemini-argon` | none: status `failed` ("the independent run ended without a successful result") | nothing merged. Its branch has unrecorded commits and a report. One observation in it, checked here independently, is fixed: an investigator's attribution read `(?)` |

## Per point

| # | Point | Result | From |
|---|---|---|---|
| 1 | Names bold, one colour each, the same in every round | the agents' `gh` colours the attribution lines and the review byline as it publishes; colours from `AGENT_GITHUB_COLORS` in the configuration's order, never two alike in one text while the palette has another. An investigator's attribution now names its CLI and model | `inv-opus`; `(?)`: this merge |
| 2 | File names as links | `prompts/writing.md`: a file is a link to `/blob/<full SHA>/<path>` (line anchors, directories as `/tree/`, deleted files at the base commit, encoded paths, never in code blocks); `writing.md` now reaches reviewers and independent investigators | `inv-opus`; details: `inv-astra` |
| 3 | Recommendation first, green/red, bold, notification icons | `prompts/review.md`: `## Recommendation` right after the byline: `✅` Ready to merge (green), `🔧` Needs changes (red), `🧩` incomplete | `inv-opus` |
| 4 | The same pair reviews a PR again (critical) | fixed in every `AGENT_REVIEW_DRAW` mode: the PR's last pair is avoided; `random` and `rotate` also skip the last two pairs of any PR. With A, B, C, D this gives A+B, then C+D, then mixed pairs. Then models the last pair did not use are preferred | `inv-opus`; models: `inv-astra` |
| 5 | No "Generated with Claude Code" | `run_claude --settings` (`includeCoAuthoredBy: false`, empty `attribution`) for new and resumed sessions; the agents' `gh` drops the line; `writing.md` forbids it | `inv-opus` |
| 6 | The earlier merged prompt report | items 1, 4, 5, 7, 8, 9, 12 fixed here; 2, 3, 6, 11 fixed before; 10 left (below) | `inv-opus` |
| 7 | Mergeable to `main` | `make gate`: 7/7; no existing test changed | — |

## What was not taken, and why

- **Rotation as the default (`inv-astra`).** The installation sets `AGENT_REVIEW_DRAW=random` explicitly, so a new
  default would not change it. inv-opus's fix applies in every mode. inv-astra also had to change four existing tests
  (approved for its task), and inv-opus needs none.
- **Hash colours in the prompt's attribution (`inv-astra`).** A SHA-256 colour per name does not keep two names apart
  and changes existing attribution tests. inv-opus colours from the configuration's order, and that gives every name
  its own colour.
- **Report item 10, remote helper scope (`inv-astra`).** It adds a `Helper` field to `JobRequest`. The lead decodes
  job requests with `DisallowUnknownFields` (`agentbus/lead_http.go`), so a lead that is not updated rejects every
  delegated job from an updated host. That is a cross-host wire change. It needs its own PR and a rollout plan
  (rule 9). It stays open, as inv-opus left it.
- **Duplicates.** inv-astra's review request in `task.md`, its capability-aware bus instructions, its helper
  `skip_note` and the prompt wording of items 7, 8, 9 and 12 duplicate inv-opus's fixes, which were taken.
