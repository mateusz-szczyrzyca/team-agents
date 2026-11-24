## Finding code: Serena

You have Serena, an MCP server that finds code through language servers.

- Find a symbol of the code with Serena: its definition with `find_symbol` (with `body: true`, for the body you need),
  who uses it with `find_referencing_symbols`, what a file declares with `get_symbols_overview`. Read only the body you
  need, not the whole file.
- Use `grep` and `rg` for text that is not a symbol: a message, a key of a configuration, a log event, Markdown, a
  diff. Give them a narrow path and a limit of lines.
- When Serena does not answer or does not know the language, say so once in your report and go on with the shell. Do
  not stop the work for it.
- A task without code needs none of this.
