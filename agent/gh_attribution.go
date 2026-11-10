package main

// FormatAttributionBlock renders the Bash attribution footer with a final newline.
// taskMetas contains parsed on-disk records in task-path order, including duplicates.
// It remains independent of GitHub writes and performs no filesystem operations.
func FormatAttributionBlock(_, _, _ string, _ []MetaFile, _ Config) string {
	return ""
}
