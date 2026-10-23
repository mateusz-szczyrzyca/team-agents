package main

// validJobIdentifier accepts one bounded logical name, never a filesystem path.
func validJobIdentifier(_ string) bool { return false }

// validJobBase checks a Git ref name and an optional exact object ID without running Git.
func validJobBase(_, _ string) bool { return false }
