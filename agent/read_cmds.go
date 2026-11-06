package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"
)

// commandFromBus wraps an agentbus execution error into a CommandError while preserving the typed BusError.
func commandFromBus(prefix string, err *BusError) *CommandError {
	if err == nil {
		return nil
	}
	cause := CommandProcess
	if err.Cause == BusNotFound {
		cause = CommandNotFound
	}
	msg := err.Message
	if prefix != "" {
		msg = prefix + ": " + msg
	}
	return &CommandError{Cause: cause, Message: msg, Err: errors.Join(cause.info().class.Sentinel(), err)}
}

// runBusDisplay writes non-empty text to out and returns cmdErr (or write failure).
func runBusDisplay(out io.Writer, cmdErr *CommandError, text, op string) *CommandError {
	if text != "" {
		if _, err := io.WriteString(out, text); err != nil {
			return fromStorage(CommandStorage, storageErr(err, "write %s", op))
		}
	}
	return cmdErr
}

// writeOutput writes s to out, returning CommandStorage error on failure.
func writeOutput(out io.Writer, s, op string) *CommandError {
	return runBusDisplay(out, nil, s, op)
}

// firstNonEmpty returns the first non-empty string among the provided values.
func firstNonEmpty(vals ...string) string {
	for _, v := range vals {
		if v != "" {
			return v
		}
	}
	return ""
}

// busBinary returns the configured agentbus path or a default resolution.
func (a *App) busBinary() string {
	return firstNonEmpty(a.busBin, os.Getenv("AGENTBUS_BIN"), "agentbus")
}

// busDirectory returns the bus state directory.
func (a *App) busDirectory() string {
	if home := os.Getenv("HOME"); home != "" {
		return firstNonEmpty(a.busDir, os.Getenv("BUS_DIR"), filepath.Join(home, "agents", "bus"))
	}
	return firstNonEmpty(a.busDir, os.Getenv("BUS_DIR"), "bus")
}

// cmdBus displays agentbus status and the latest log entries.
func (a *App) cmdBus(ctx context.Context, n string, stdout io.Writer) *CommandError {
	busBin, busDir := a.busBinary(), a.busDirectory()
	statusOut, err := busCmd(ctx, a, busBin, "status", "--dir", busDir)
	if err == nil {
		statusOut += "\n"
	}
	if werr := runBusDisplay(stdout, commandFromBus("bus status", err), statusOut, "bus status"); werr != nil {
		return werr
	}
	logOut, err := busCmd(ctx, a, busBin, "log", "--dir", busDir, "-n", firstNonEmpty(n, "30"))
	return runBusDisplay(stdout, commandFromBus("bus log", err), logOut, "bus log")
}

// cmdHelpers lists active helper agents.
func (a *App) cmdHelpers(ctx context.Context, args []string, stdout io.Writer) *CommandError {
	out, err := busCmd(ctx, a, a.busBinary(), append([]string{"helper-list", "--dir", a.busDirectory()}, args...)...)
	return runBusDisplay(stdout, commandFromBus("helpers", err), out, "helpers")
}

// cmdInv displays the investigation queue or details of an investigation.
func (a *App) cmdInv(ctx context.Context, args []string, stdout io.Writer) *CommandError {
	busBin, busDir := a.busBinary(), a.busDirectory()
	cfg := a.listingConfig()
	qMax, slots := firstNonEmpty(cfg.Scalars["INVESTIGATION_QUEUE_MAX"], "3"), firstNonEmpty(cfg.Scalars["INV_SLOTS"], "1")
	if len(args) == 0 || args[0] == "" || args[0] == "--all" {
		invArgs := []string{"inv-list", "--dir", busDir, "--queue-max", qMax, "--inv-slots", slots}
		if len(args) > 0 && args[0] == "--all" {
			invArgs = append(invArgs, "--all")
		}
		out, err := busCmd(ctx, a, busBin, invArgs...)
		if werr := runBusDisplay(stdout, commandFromBus("inv list", err), out, "inv-list"); werr != nil {
			return werr
		}
		if len(args) == 0 || args[0] == "" {
			return writeOutput(stdout, "\nAll: agent inv --all   |   details: agent inv <number>\n", "inv footer")
		}
		return nil
	}

	out, err := busCmd(ctx, a, busBin, "inv-get", "--dir", busDir, "--id", strings.TrimPrefix(args[0], "#"))
	if err != nil {
		return commandFromBus("inv get", err)
	}
	var res struct {
		File string `json:"file"`
	}
	if jerr := json.Unmarshal([]byte(out), &res); jerr != nil {
		return opErr(CommandStorage, "parse inv-get json: %s", jerr.Error())
	}
	data, rerr := os.ReadFile(res.File) // #nosec G304 -- local investigation record path returned by agentbus inv-get
	if rerr != nil {
		return fromStorage(CommandStorage, storageErr(rerr, "read investigation file %s", res.File))
	}
	return writeOutput(stdout, string(data), "investigation content")
}

// cmdNotes displays or searches notebook entries.
func (a *App) cmdNotes(ctx context.Context, args []string, stdout io.Writer) *CommandError {
	busBin, busDir := a.busBinary(), a.busDirectory()
	repo, rest := "", args
	if len(args) > 0 && args[0] != "" && !strings.HasPrefix(args[0], "-") {
		repo, rest = args[0], args[1:]
	} else {
		var rBuf, cBuf bytes.Buffer
		if res, err := a.execute(ctx, Command{Name: "git", Args: []string{"rev-parse", "--show-toplevel"}, Dir: a.workTree, Stdout: &rBuf}); err == nil && res.ExitCode == 0 {
			if res2, err2 := a.execute(ctx, Command{Name: "git", Args: []string{"-C", strings.TrimSpace(rBuf.String()), "rev-parse", "--path-format=absolute", "--git-common-dir"}, Stdout: &cBuf}); err2 == nil && res2.ExitCode == 0 {
				if base := filepath.Base(filepath.Dir(strings.TrimSpace(cBuf.String()))); base != "." && base != "/" {
					repo = base
				}
			}
		}
	}
	cmdArgs := []string{"notes", "--dir", busDir, "--list"}
	if repo != "" {
		cmdArgs = append([]string{"notes", "--dir", busDir, "--repo", repo}, rest...)
	} else if err := writeOutput(stdout, "Repository notebooks:\n", "notes header"); err != nil {
		return err
	}
	out, err := busCmd(ctx, a, busBin, cmdArgs...)
	return runBusDisplay(stdout, commandFromBus("bus notes", err), out, "notes")
}
