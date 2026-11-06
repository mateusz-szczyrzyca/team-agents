package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
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

// busBinary returns the configured agentbus path or a default resolution.
func (a *App) busBinary() string {
	if a.busBin != "" {
		return a.busBin
	}
	if env := os.Getenv("AGENTBUS_BIN"); env != "" {
		return env
	}
	return "agentbus"
}

// busDirectory returns the bus state directory.
func (a *App) busDirectory() string {
	if a.busDir != "" {
		return a.busDir
	}
	if env := os.Getenv("BUS_DIR"); env != "" {
		return env
	}
	if home := os.Getenv("HOME"); home != "" {
		return filepath.Join(home, "agents", "bus")
	}
	return "bus"
}

// cmdBus displays agentbus status and the latest log entries.
func (a *App) cmdBus(ctx context.Context, n string, stdout io.Writer) *CommandError {
	busBin, busDir := a.busBinary(), a.busDirectory()
	if n == "" {
		n = "30"
	}
	statusOut, err := busCmd(ctx, a, busBin, "status", "--dir", busDir)
	if err != nil {
		return commandFromBus("bus status", err)
	}
	logOut, err := busCmd(ctx, a, busBin, "log", "--dir", busDir, "-n", n)
	if err != nil {
		return commandFromBus("bus log", err)
	}
	if _, werr := fmt.Fprintf(stdout, "%s\n%s", statusOut, logOut); werr != nil {
		return fromStorage(CommandStorage, storageErr(werr, "write bus status"))
	}
	return nil
}

// cmdHelpers lists active helper agents.
func (a *App) cmdHelpers(ctx context.Context, args []string, stdout io.Writer) *CommandError {
	out, err := busCmd(ctx, a, a.busBinary(), append([]string{"helper-list", "--dir", a.busDirectory()}, args...)...)
	if err != nil {
		return commandFromBus("helpers", err)
	}
	if _, werr := fmt.Fprint(stdout, out); werr != nil {
		return fromStorage(CommandStorage, storageErr(werr, "write helpers"))
	}
	return nil
}

// cmdInv displays the investigation queue or details of an investigation.
func (a *App) cmdInv(ctx context.Context, args []string, stdout io.Writer) *CommandError {
	busBin, busDir := a.busBinary(), a.busDirectory()
	queueMax, invSlots := "3", "1"
	if cfg := a.listingConfig(); cfg != nil && cfg.Scalars != nil {
		if q := cfg.Scalars["INVESTIGATION_QUEUE_MAX"]; q != "" {
			queueMax = q
		}
		if s := cfg.Scalars["INV_SLOTS"]; s != "" {
			invSlots = s
		}
	}
	if len(args) == 0 || args[0] == "" || args[0] == "--all" {
		invArgs := []string{"inv-list", "--dir", busDir, "--queue-max", queueMax, "--inv-slots", invSlots}
		if len(args) > 0 && args[0] == "--all" {
			invArgs = append(invArgs, "--all")
		}
		out, err := busCmd(ctx, a, busBin, invArgs...)
		if err != nil {
			return commandFromBus("inv list", err)
		}
		if _, werr := fmt.Fprint(stdout, out); werr != nil {
			return fromStorage(CommandStorage, storageErr(werr, "write inv-list"))
		}
		if len(args) == 0 || args[0] == "" {
			if _, werr := fmt.Fprintf(stdout, "\nAll: agent inv --all   |   details: agent inv <number>\n"); werr != nil {
				return fromStorage(CommandStorage, storageErr(werr, "write inv footer"))
			}
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
	if _, werr := fmt.Fprint(stdout, string(data)); werr != nil {
		return fromStorage(CommandStorage, storageErr(werr, "write investigation content"))
	}
	return nil
}

// cmdNotes displays or searches notebook entries.
func (a *App) cmdNotes(ctx context.Context, args []string, stdout io.Writer) *CommandError {
	busBin, busDir := a.busBinary(), a.busDirectory()
	var repo string
	var rest []string
	if len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		repo, rest = args[0], args[1:]
	} else {
		rest = args
	}
	if repo == "" {
		var rootBuf bytes.Buffer
		cmd1 := Command{Name: "git", Args: []string{"rev-parse", "--show-toplevel"}, Dir: a.workTree, Stdout: &rootBuf}
		if res, err := a.execute(ctx, cmd1); err == nil && res.ExitCode == 0 {
			if root := strings.TrimSpace(rootBuf.String()); root != "" {
				var commonBuf bytes.Buffer
				cmd2 := Command{Name: "git", Args: []string{"-C", root, "rev-parse", "--path-format=absolute", "--git-common-dir"}, Stdout: &commonBuf}
				if res2, err2 := a.execute(ctx, cmd2); err2 == nil && res2.ExitCode == 0 {
					if commonDir := strings.TrimSpace(commonBuf.String()); commonDir != "" {
						if base := filepath.Base(filepath.Dir(commonDir)); base != "." && base != "/" && base != "" {
							repo = base
						}
					}
				}
			}
		}
	}
	if repo == "" {
		if _, werr := fmt.Fprintf(stdout, "Repository notebooks:\n"); werr != nil {
			return fromStorage(CommandStorage, storageErr(werr, "write notes header"))
		}
		out, err := busCmd(ctx, a, busBin, "notes", "--dir", busDir, "--list")
		if err != nil {
			return commandFromBus("bus notes --list", err)
		}
		if _, werr := fmt.Fprint(stdout, out); werr != nil {
			return fromStorage(CommandStorage, storageErr(werr, "write notes list"))
		}
		return nil
	}
	out, err := busCmd(ctx, a, busBin, append([]string{"notes", "--dir", busDir, "--repo", repo}, rest...)...)
	if err != nil {
		return commandFromBus("bus notes", err)
	}
	if _, werr := fmt.Fprint(stdout, out); werr != nil {
		return fromStorage(CommandStorage, storageErr(werr, "write notes"))
	}
	return nil
}
