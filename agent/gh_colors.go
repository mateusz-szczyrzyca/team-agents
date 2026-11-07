package main

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"syscall"
)

// isValidColor checks if c matches the oracle pattern ^#[0-9A-Fa-f]{6}$.
func isValidColor(c string) bool {
	return len(c) == 7 && c[0] == '#' && strings.IndexFunc(c[1:], func(r rune) bool {
		return (r < '0' || r > '9') && (r < 'a' || r > 'f') && (r < 'A' || r > 'F')
	}) == -1
}

// isValidName checks if n matches the oracle pattern ^[A-Za-z0-9._@-]+$.
func isValidName(n string) bool {
	return len(n) > 0 && strings.IndexFunc(n, func(r rune) bool {
		return (r < 'a' || r > 'z') && (r < 'A' || r > 'Z') && (r < '0' || r > '9') && r != '.' && r != '_' && r != '@' && r != '-'
	}) == -1
}

// readColorLines parses remembered color assignments from path.
func readColorLines(path string) (pairs [][2]string) {
	data, _ := os.ReadFile(filepath.Clean(path)) //nolint:gosec // caller supplies local color assignment path
	for _, line := range strings.Split(string(data), "\n") {
		if f := strings.Fields(line); len(f) == 2 && isValidName(f[0]) && isValidColor(f[1]) {
			pairs = append(pairs, [2]string{f[0], f[1]})
		}
	}
	return pairs
}

// sortedMapKeys returns the sorted keys of an associative configuration map.
func sortedMapKeys(m map[string]string) (keys []string) {
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}

// RegisterGitHubColors assigns a color from palette to each name in names that does not yet have one.
// Assignments are stored in colorsPath ("<name> <#RRGGBB>\n").
// Concurrency is handled via flock on colorsPath + ".lock".
func RegisterGitHubColors(ctx context.Context, colorsPath string, palette []string, names []string, logger *Logger) (retErr *StorageError) {
	_ = ctx
	_ = logger
	if len(palette) == 0 || len(names) == 0 {
		return nil
	}
	if err := os.MkdirAll(filepath.Dir(colorsPath), 0o750); err != nil {
		return storageErr(err, "mkdir %s", filepath.Dir(colorsPath))
	}
	lockPath := colorsPath + ".lock"
	lockFile, err := os.OpenFile(filepath.Clean(lockPath), os.O_CREATE|os.O_RDWR, 0o600) //nolint:gosec // lock path is within AGENTS_HOME
	if err != nil {
		return storageErr(err, "open lock %s", lockPath)
	}
	defer func() {
		if joinErr := errors.Join(syscall.Flock(int(lockFile.Fd()), syscall.LOCK_UN), lockFile.Close()); joinErr != nil && retErr == nil {
			retErr = storageErr(joinErr, "release lock %s", lockPath)
		}
	}()
	if err := syscall.Flock(int(lockFile.Fd()), syscall.LOCK_EX); err != nil {
		return storageErr(err, "flock %s", lockPath)
	}

	have, uses := make(map[string]string), make(map[string]int)
	for _, p := range readColorLines(colorsPath) {
		have[p[0]], uses[p[1]] = p[1], uses[p[1]]+1
	}
	var toAppend []byte
	for _, n := range names {
		if !isValidName(n) || have[n] != "" {
			continue
		}
		best := palette[0]
		for _, p := range palette[1:] {
			if uses[p] < uses[best] {
				best = p
			}
		}
		toAppend = append(toAppend, (n + " " + best + "\n")...)
		have[n], uses[best] = best, uses[best]+1
	}
	if len(toAppend) > 0 {
		return appendLog(colorsPath, toAppend)
	}
	return nil
}

// FormatGitHubNameColors formats the AGENT_GITHUB_NAME_COLORS string ("name=#RRGGBB ...").
// It first takes remembered colors in colorsPath, then for any configured agents not in have,
// assigns their color from the configuration's order (NORMAL_AGENTS, WORKER_AGENTS, sorted AGENT_TYPE, sorted INVESTIGATORS_TYPE).
func FormatGitHubNameColors(colorsPath string, palette []string, config Config) string {
	if len(palette) == 0 {
		return ""
	}
	have, entries := make(map[string]bool), []string(nil)
	for _, p := range readColorLines(colorsPath) {
		if !have[p[0]] {
			have[p[0]], entries = true, append(entries, p[0]+"="+p[1])
		}
	}
	var names []string
	listed := make(map[string]bool)
	appendName := func(a string) {
		if a != "" && !listed[a] {
			listed[a], names = true, append(names, a)
		}
	}
	for _, k := range []string{"NORMAL_AGENTS", "WORKER_AGENTS"} {
		m := config.Indexed[k]
		var idxs []int
		for s := range m {
			if n, err := strconv.Atoi(s); err == nil {
				idxs = append(idxs, n)
			}
		}
		sort.Ints(idxs)
		for _, n := range idxs {
			appendName(m[strconv.Itoa(n)])
		}
	}
	for _, a := range sortedMapKeys(config.Associative["AGENT_TYPE"]) {
		appendName(a)
	}
	for _, group := range [][]string{names, sortedMapKeys(config.Associative["INVESTIGATORS_TYPE"])} {
		for i, n := range group {
			if !have[n] {
				have[n], entries = true, append(entries, n+"="+palette[i%len(palette)])
			}
		}
	}
	return strings.Join(entries, " ")
}
