package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
)

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
	busBin := a.busBinary()
	busDir := a.busDirectory()
	if n == "" {
		n = "30"
	}
	statusOut, err := busCmd(ctx, a, busBin, "status", "--dir", busDir)
	if err != nil {
		return opErr(CommandStorage, "bus status: %s", err.Error())
	}
	if _, werr := fmt.Fprint(stdout, statusOut); werr != nil {
		return fromStorage(CommandStorage, storageErr(werr, "write bus status"))
	}
	if _, werr := fmt.Fprintln(stdout); werr != nil {
		return fromStorage(CommandStorage, storageErr(werr, "write bus status newline"))
	}
	logOut, err := busCmd(ctx, a, busBin, "log", "--dir", busDir, "-n", n)
	if err != nil {
		return opErr(CommandStorage, "bus log: %s", err.Error())
	}
	if _, werr := fmt.Fprint(stdout, logOut); werr != nil {
		return fromStorage(CommandStorage, storageErr(werr, "write bus log"))
	}
	return nil
}

// cmdHelpers lists active helper agents.
func (a *App) cmdHelpers(ctx context.Context, args []string, stdout io.Writer) *CommandError {
	busBin := a.busBinary()
	busDir := a.busDirectory()
	cmdArgs := append([]string{"helper-list", "--dir", busDir}, args...)
	out, err := busCmd(ctx, a, busBin, cmdArgs...)
	if err != nil {
		return opErr(CommandStorage, "helpers: %s", err.Error())
	}
	if _, werr := fmt.Fprint(stdout, out); werr != nil {
		return fromStorage(CommandStorage, storageErr(werr, "write helpers"))
	}
	return nil
}

// cmdInv displays the investigation queue or details of an investigation.
func (a *App) cmdInv(ctx context.Context, args []string, stdout io.Writer) *CommandError {
	busBin := a.busBinary()
	busDir := a.busDirectory()
	queueMax := "3" // oracle default for INVESTIGATION_QUEUE_MAX
	invSlots := "1" // oracle default for INV_SLOTS
	if len(args) == 0 || args[0] == "" {
		out, err := busCmd(ctx, a, busBin, "inv-list", "--dir", busDir, "--queue-max", queueMax, "--inv-slots", invSlots)
		if err != nil {
			return opErr(CommandStorage, "inv list: %s", err.Error())
		}
		if _, werr := fmt.Fprint(stdout, out); werr != nil {
			return fromStorage(CommandStorage, storageErr(werr, "write inv-list"))
		}
		if _, werr := fmt.Fprintf(stdout, "\nAll: agent inv --all   |   details: agent inv <number>\n"); werr != nil {
			return fromStorage(CommandStorage, storageErr(werr, "write inv footer"))
		}
		return nil
	}
	if args[0] == "--all" {
		out, err := busCmd(ctx, a, busBin, "inv-list", "--dir", busDir, "--queue-max", queueMax, "--inv-slots", invSlots, "--all")
		if err != nil {
			return opErr(CommandStorage, "inv list --all: %s", err.Error())
		}
		if _, werr := fmt.Fprint(stdout, out); werr != nil {
			return fromStorage(CommandStorage, storageErr(werr, "write inv-list --all"))
		}
		return nil
	}

	id := strings.TrimPrefix(args[0], "#")
	out, err := busCmd(ctx, a, busBin, "inv-get", "--dir", busDir, "--id", id)
	if err != nil {
		return opErr(CommandStorage, "inv get: %s", err.Error())
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
	busBin := a.busBinary()
	busDir := a.busDirectory()
	var repo string
	var rest []string
	if len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		repo = args[0]
		rest = args[1:]
	} else {
		rest = args
	}
	if repo == "" {
		var rootBuf bytes.Buffer
		cmd1 := Command{
			Name:   "git",
			Args:   []string{"rev-parse", "--show-toplevel"},
			Dir:    a.workTree,
			Stdout: &rootBuf,
		}
		if res, err := a.execute(ctx, cmd1); err == nil && res.ExitCode == 0 {
			root := strings.TrimSpace(rootBuf.String())
			if root != "" {
				var commonBuf bytes.Buffer
				cmd2 := Command{
					Name:   "git",
					Args:   []string{"-C", root, "rev-parse", "--path-format=absolute", "--git-common-dir"},
					Stdout: &commonBuf,
				}
				if res2, err2 := a.execute(ctx, cmd2); err2 == nil && res2.ExitCode == 0 {
					commonDir := strings.TrimSpace(commonBuf.String())
					if commonDir != "" {
						base := filepath.Base(filepath.Dir(commonDir))
						if base != "." && base != "/" && base != "" {
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
			return opErr(CommandStorage, "bus notes --list: %s", err.Error())
		}
		if _, werr := fmt.Fprint(stdout, out); werr != nil {
			return fromStorage(CommandStorage, storageErr(werr, "write notes list"))
		}
		return nil
	}
	cmdArgs := append([]string{"notes", "--dir", busDir, "--repo", repo}, rest...)
	out, err := busCmd(ctx, a, busBin, cmdArgs...)
	if err != nil {
		return opErr(CommandStorage, "bus notes: %s", err.Error())
	}
	if _, werr := fmt.Fprint(stdout, out); werr != nil {
		return fromStorage(CommandStorage, storageErr(werr, "write notes"))
	}
	return nil
}
