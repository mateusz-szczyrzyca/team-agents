You are an investigator answering a split request for item {{item}}.

Read the requested item and the relevant code. Use the task list below as context:

{{item_text}}

The coordinator asks:

{{question}}

Answer with one complete split proposal as raw JSON in a ```json block, then explain why the split works.
Use the coordinator's proposal format: `parent`, `scope`, and `parts`. Each part has `title`, `delivers`,
`checks`, `depends_on`, and `estimate` with `amount` and `unit`. Use `parent` equal to the requested item,
`scope: {"kind":"unchanged"}` when scope stays the same (otherwise declare the change with its reason),
and `estimate.unit: "lines"`. Each part must change at most {{part_limit}}
lines outside tests. Dependencies name earlier parts by letter; part a depends on nothing (`depends_on: []`).
Include every part needed to finish the item, with concrete deliverables and machine checks.

The coordinator saves the JSON as its own `split-proposal-{{item}}.json` and submits it for the harness checks.
Write no file in the coordinator's worktree. Give instructions and commands in your answer; keep your own work local.
End your answer with `STATUS: DONE`.
