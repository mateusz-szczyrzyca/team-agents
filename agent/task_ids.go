package main

import (
	"errors"
	"os"
	"strings"
	"syscall"
)

// resolveTaskID preserves exact-directory precedence and literal fragment matching.
func resolveTaskID(tasksDir, ref string) (string, *ResolveTaskError) {
	// Concatenation preserves the Bash path and returns the original reference verbatim.
	info, exactErr := os.Stat(tasksDir + "/" + ref)
	if exactErr == nil && info.IsDir() {
		return ref, nil
	}
	// A missing or non-directory exact path falls through to the legacy fragment search.
	// Other inspection failures are reported rather than hidden by an unrelated match.
	if exactErr != nil && !errors.Is(exactErr, os.ErrNotExist) && !errors.Is(exactErr, syscall.ENOTDIR) {
		return "", fromStorage(ResolveTaskStorage, storageErr(exactErr, "cannot inspect task %s", ref))
	}
	entries, err := os.ReadDir(tasksDir)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return "", fromStorage(ResolveTaskStorage, storageErr(err, "cannot list %s", tasksDir))
	}
	var names []string
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), ".") {
			continue
		}
		info, statErr := os.Stat(tasksDir + "/" + entry.Name())
		if statErr != nil {
			// A broken directory symlink is excluded by Bash's */ glob as well.
			if errors.Is(statErr, os.ErrNotExist) {
				continue
			}
			return "", fromStorage(ResolveTaskStorage, storageErr(statErr, "cannot inspect task %s", entry.Name()))
		}
		if info.IsDir() {
			names = append(names, entry.Name())
		}
	}
	// Without directory matches Bash leaves its */ glob unexpanded.
	if len(names) == 0 {
		names = append(names, "*")
	}
	var matches []string
	for _, name := range names {
		if strings.Contains(name, ref) {
			matches = append(matches, name)
		}
	}
	if len(matches) == 1 {
		return matches[0], nil
	}
	cause := ResolveTaskNotFound
	if len(matches) > 1 {
		cause = ResolveTaskAmbiguous
	}
	return "", opErr(cause, "no unique task matches '%s' (%d matches)", ref, len(matches))
}
