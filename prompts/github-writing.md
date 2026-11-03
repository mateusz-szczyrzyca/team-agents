## GitHub publication

Apply these rules whenever the task authorizes a PR description, comment or review; this section does not
itself authorize publication, commits or pushes.

- Preserve the supplied bold colored agent identities verbatim. Their colors belong to the name and stay the
  same across rounds, roles and model changes. Do not put the rendered identity inside backticks.
- Link repository file references in prose, including inline review comments:
  [`path/to/file`]({{REPO_URL}}/blob/<full-commit-SHA>/path/to/file).
  Resolve the actual reviewed or discussed commit with Git; do not leave the placeholder SHA in published text.
  Check that the file exists in that commit. For deleted files, link the base commit instead.
  URL-encode path components; use `/tree/<full-commit-SHA>/` for directories and `#L42` or `#L42-L50` for lines.
  Keep code samples intact. Do not invent links to nonexistent files or to local machine paths.
- Do not append vendor-generated attribution such as "Generated with Claude Code", vendor logos, or
  vendor co-author trailers. Keep the team-agents attribution and the required task commit trailers.
