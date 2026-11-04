## Finding code: Serena

The MCP server `serena` knows this repository's code through language servers, on your worktree `{{WORKTREE}}`.
Use it to find code before you read whole files or search the text: a file's symbols, a symbol's definition, the
code that uses it. Follow its own instructions (call `initial_instructions` first when it has that tool, then use
`serena_repl` or its symbol tools such as `get_symbols_overview`, `find_symbol`, `find_referencing_symbols`). Read a
file yourself only where you need its text. If Serena fails, is slow, or does not know a file (a new one, a language
it has no server for), use the shell as usual — do not retry it in a loop.
