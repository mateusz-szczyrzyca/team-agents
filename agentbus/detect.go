package main

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

// defaultDetectTimeout is the maximum time spent running detection commands.
const defaultDetectTimeout = 2 * time.Second

// toolchainChecks maps toolchain names to their command executable and arguments.
var toolchainChecks = []struct {
	name string
	cmd  string
	args []string
}{
	{name: "go", cmd: "go", args: []string{"version"}},
	{name: "rust", cmd: "rustc", args: []string{"--version"}},
	{name: "node", cmd: "node", args: []string{"--version"}},
	{name: "python", cmd: "python3", args: []string{"--version"}},
	{name: "c", cmd: "gcc", args: []string{"--version"}},
	{name: "clang", cmd: "clang", args: []string{"--version"}},
}

// runDetectCommand executes a command with a strict timeout and returns the trimmed first line of output.
func runDetectCommand(cmdName string, args ...string) (string, bool) {
	path, err := exec.LookPath(cmdName)
	if err != nil {
		return "", false
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultDetectTimeout)
	defer cancel()

	cmd := exec.CommandContext(ctx, path, args...) //nolint:gosec // path looked up via fixed executable name
	out, err := cmd.Output()
	if err != nil {
		return "", false
	}
	lines := strings.Split(strings.TrimSpace(string(out)), "\n")
	if len(lines) == 0 || lines[0] == "" {
		return "", false
	}
	return lines[0], true
}

// DetectCapabilities scans the local environment for OS, architecture, toolchains, and hardware.
func DetectCapabilities() DetectedCapabilities {
	detected := DetectedCapabilities{
		OS:         runtime.GOOS,
		Arch:       runtime.GOARCH,
		Toolchains: []string{},
	}

	for _, check := range toolchainChecks {
		if out, ok := runDetectCommand(check.cmd, check.args...); ok {
			detected.Toolchains = append(detected.Toolchains, check.name+":"+out)
		}
	}

	if runtime.GOOS == "darwin" {
		if out, ok := runDetectCommand("xcodebuild", "-version"); ok {
			detected.XcodeVersion = out
		}
		if out, ok := runDetectCommand("xcrun", "simctl", "list", "runtimes"); ok {
			detected.IOSSimulators = parseIOSSimulators(out)
		}
	}

	detected.HardwareProbes = detectHardwareProbes()
	return detected
}

// parseIOSSimulators extracts runtime names from xcrun simctl output.
func parseIOSSimulators(output string) []string {
	var runtimes []string
	lines := strings.Split(output, "\n")
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "iOS ") {
			runtimes = append(runtimes, trimmed)
		}
	}
	return runtimes
}

// detectHardwareProbes scans /dev/serial/by-id/ for attached hardware debug probes and development boards.
func detectHardwareProbes() []string {
	var probes []string
	serialDir := "/dev/serial/by-id"
	entries, err := os.ReadDir(serialDir)
	if err != nil {
		return probes
	}
	for _, entry := range entries {
		name := entry.Name()
		lower := strings.ToLower(name)
		switch {
		case strings.Contains(lower, "rp2350") || strings.Contains(lower, "picoprobe"):
			probes = append(probes, "rp2350-probe")
		case strings.Contains(lower, "jlink"):
			probes = append(probes, "jlink")
		case strings.Contains(lower, "stlink"):
			probes = append(probes, "stlink")
		default:
			probes = append(probes, filepath.Base(name))
		}
	}
	return probes
}
