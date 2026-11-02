// cli.go implements command-line interface parsing and dispatch.
package main

import (
	"fmt"
)

// dispatchCommand processes the command-line arguments and dispatches to the appropriate handler.
// It returns a RunError if the command is invalid or execution fails.
func dispatchCommand(argv []string) *RunError {
	if len(argv) == 0 {
		// No arguments: show status or help placeholder
		return nil
	}

	cmd := argv[0]

	// Handle help flags
	if cmd == "-h" || cmd == "--help" || cmd == "help" {
		fmt.Println(usageText())
		return nil
	}

	// Unknown command
	return NewRunError(RunUnknownCommand, fmt.Sprintf("unknown command: %s", cmd))
}

// usageText returns the full help text for the agent command.
func usageText() string {
	return `agent: task orchestration for AI coding agents

Usage:
  agent [command] [options]

Commands:
  run <agent> "<task>" | -f file.md    start a task
  status [<task>]                      list tasks or task details
  agents                               show configured agents
  show <task>                           show task details
  answer <task>                        edit questions
  resume <task>                        continue after answers
  stop <task> | --running              stop a run
  clean <task>                         remove task worktree
  inv [N | --all]                      show investigation queue
  live [--feed]                        live panel
  ... (more commands in implementation rounds)

Options:
  -h, --help                           show this help message

Modifiers for 'run':
  run --team                           start a team
  run --investigation                  start an investigation
  run --need <capability>              request specific capability
  run --on <node>                      run on specific node
  run --resume                         resume interrupted task
  run --last-team                      continue last team
`
}
