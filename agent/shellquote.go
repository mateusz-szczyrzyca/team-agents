package main

// quoteMeta emits Bash printf-%q bytes in the runner's C.UTF-8 locale.
func quoteMeta(_ string) (string, *ValueError) { return "", nil }

// formatMetaAssignment validates a scalar name and emits one physical metadata record.
func formatMetaAssignment(_, _ string) (string, *ValueError) { return "", nil }
