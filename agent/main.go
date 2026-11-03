// Package main implements the agent command.
package main

import (
	"fmt"
	"os"
)

// main is the entry point for the agent command.
func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}

// run processes the command line arguments and executes the appropriate command.
func run(args []string) error {
	if len(args) == 0 {
		return showHelp()
	}
	cmd := args[0]
	switch cmd {
	case "help", "-h", "--help":
		return showHelp()
	default:
		return fmt.Errorf("unknown command: %s", cmd)
	}
}

// showHelp displays the usage information.
func showHelp() error {
	_, err := fmt.Fprintf(os.Stdout, "Usage: agent <command>\n")
	if err != nil {
		return err
	}
	_, err = fmt.Fprintf(os.Stdout, "\nCommands:\n")
	if err != nil {
		return err
	}
	_, err = fmt.Fprintf(os.Stdout, "  help, -h, --help  Show this help message\n")
	return err
}
