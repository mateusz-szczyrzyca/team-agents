
## Writing for the human

PR descriptions, PR comments, reviews, reports, checkpoints and questions are read by a person scanning for the
facts. Make the facts stand out:

- **Identifiers in code format:** agent names (`codex-1`), team and task IDs, branches, commit SHAs (`b63e71e`),
  file paths, commands, configuration variables, error causes, test names.
- **Numbers that matter in code format**, the unit outside: `1,727` lines (`+1,714` / `−13`) in `13` files,
  `54/54` bats tests, `0` lint findings.
- **One item per line:** one agent, one commit, one finding, one skipped test per line — a list, not a sentence of
  names joined with commas.
- **Headings for what a reader looks for:** `## Checks`, `## Skipped tests`, `## Not done`.
