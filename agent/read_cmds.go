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
		msg = prefix + ": " + err.Message
	}
	return &CommandError{
		Cause:   cause,
		Message: msg,
		Err:     errors.Join(cause.info().class.Sentinel(), err),
	}
}

// writeOutput writes s to out, returning CommandStorage error on failure.
func writeOutput(out io.Writer, s, op string) *CommandError {
	if _, err := io.WriteString(out, s); err != nil {
		return fromStorage(CommandStorage, storageErr(err, "write %s", op))
	}
	return nil
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
	n = firstNonEmpty(n, "30")
	statusOut, err := busCmd(ctx, a, busBin, "status", "--dir", busDir)
	if statusOut != "" {
		if werr := writeOutput(stdout, statusOut+"\n", "bus status"); werr != nil && err == nil {
			return werr
		}
	}
	if err != nil {
		return commandFromBus("bus status", err)
	}
	logOut, err := busCmd(ctx, a, busBin, "log", "--dir", busDir, "-n", n)
	if logOut != "" {
		if werr := writeOutput(stdout, logOut, "bus log"); werr != nil && err == nil {
			return werr
		}
	}
	if err != nil {
		return commandFromBus("bus log", err)
	}
	return nil
}

// cmdHelpers lists active helper agents.
func (a *App) cmdHelpers(ctx context.Context, args []string, stdout io.Writer) *CommandError {
	out, err := busCmd(ctx, a, a.busBinary(), append([]string{"helper-list", "--dir", a.busDirectory()}, args...)...)
	if out != "" {
		if werr := writeOutput(stdout, out, "helpers"); werr != nil && err == nil {
			return werr
		}
	}
	if err != nil {
		return commandFromBus("helpers", err)
	}
	return nil
}

// cmdInv displays the investigation queue or details of an investigation.
func (a *App) cmdInv(ctx context.Context, args []string, stdout io.Writer) *CommandError {
	busBin, busDir := a.busBinary(), a.busDirectory()
	queueMax, invSlots := "3", "1"
	if cfg := a.listingConfig(); cfg != nil && cfg.Scalars != nil {
		queueMax = firstNonEmpty(cfg.Scalars["INVESTIGATION_QUEUE_MAX"], "3")
		invSlots = firstNonEmpty(cfg.Scalars["INV_SLOTS"], "1")
	}
	if len(args) == 0 || args[0] == "" || args[0] == "--all" {
		invArgs := []string{"inv-list", "--dir", busDir, "--queue-max", queueMax, "--inv-slots", invSlots}
		if len(args) > 0 && args[0] == "--all" {
			invArgs = append(invArgs, "--all")
		}
		out, err := busCmd(ctx, a, busBin, invArgs...)
		if out != "" {
			if werr := writeOutput(stdout, out, "inv-list"); werr != nil && err == nil {
				return werr
			}
		}
		if err != nil {
			return commandFromBus("inv list", err)
		}
		if len(args) == 0 || args[0] == "" {
			return writeOutput(stdout, "\nAll: agent inv --all   |   details: agent inv <number>\n", "inv footer")
		}
		return nil
	}

	id := strings.TrimPrefix(args[0], "#")
	out, err := busCmd(ctx, a, busBin, "inv-get", "--dir", busDir, "--id", id)
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
	var repo string
	rest := args
	if len(args) > 0 && args[0] != "" && !strings.HasPrefix(args[0], "-") {
		repo, rest = args[0], args[1:]
	}
	if repo == "" {
		var rootBuf, commonBuf bytes.Buffer
		if res, err := a.execute(ctx, Command{Name: "git", Args: []string{"rev-parse", "--show-toplevel"}, Dir: a.workTree, Stdout: &rootBuf}); err == nil && res.ExitCode == 0 {
			if res2, err2 := a.execute(ctx, Command{Name: "git", Args: []string{"-C", strings.TrimSpace(rootBuf.String()), "rev-parse", "--path-format=absolute", "--git-common-dir"}, Stdout: &commonBuf}); err2 == nil && res2.ExitCode == 0 {
				if base := filepath.Base(filepath.Dir(strings.TrimSpace(commonBuf.String()))); base != "." && base != "/" && base != "" {
					repo = base
				}
			}
		}
	}
	notesArgs := []string{"notes", "--dir", busDir, "--list"}
	if repo != "" {
		notesArgs = append([]string{"notes", "--dir", busDir, "--repo", repo}, rest...)
	} else if err := writeOutput(stdout, "Repository notebooks:\n", "notes header"); err != nil {
		return err
	}
	out, err := busCmd(ctx, a, busBin, notesArgs...)
	if out != "" {
		if werr := writeOutput(stdout, out, "notes"); werr != nil && err == nil {
			return werr
		}
	}
	if err != nil {
		return commandFromBus("bus notes", err)
	}
	return nil
}
