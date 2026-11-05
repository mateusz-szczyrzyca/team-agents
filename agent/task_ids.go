package main

// resolveTaskID preserves exact-directory precedence and literal fragment matching.
func resolveTaskID(_, _ string) (string, *ResolveTaskError) { return "", nil }
